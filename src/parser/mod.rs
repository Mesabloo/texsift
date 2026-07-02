pub mod file_stack;
pub mod line_joiner;
pub mod message;

use crate::model::{Event, LogMessage, PassKind};
use file_stack::FileStack;
use line_joiner::LineJoiner;
use message::{MessageMatcher, RawMessage};

fn try_parse_pass_boundary(line: &str) -> Option<PassKind> {
    let rest = line.strip_prefix("Run number ")?;
    let after_num = rest.find(|c: char| !c.is_ascii_digit())?;
    let after = rest[after_num..].strip_prefix(" of rule '")?;
    let end = after.find('\'')?;
    let name = &after[..end];
    Some(if name == "pdflatex" {
        PassKind::Pdflatex
    } else if name.starts_with("bibtex") {
        PassKind::Bibtex
    } else {
        PassKind::Other(name.to_string())
    })
}

fn try_parse_pdf_built(line: &str) -> Option<String> {
    let rest = line.strip_prefix("Output written on ")?;
    let idx = rest.find(" (")?;
    Some(rest[..idx].to_string())
}

/// Fallback pass-boundary detection for engines invoked without latexmk's
/// `Run number ... of rule '...'` wrapper (e.g. plain `lualatex file.tex`):
/// every engine run prints a `This is <Engine>, ...` banner, which for
/// LaTeX-family formats also carries a `format=<name>` token.
fn try_parse_engine_banner(line: &str) -> Option<PassKind> {
    let rest = line.strip_prefix("This is ")?;
    if rest.starts_with("BibTeX") {
        return Some(PassKind::Bibtex);
    }
    if let Some(idx) = rest.find("format=") {
        let after = &rest[idx + "format=".len()..];
        let end = after.find(|c: char| c.is_whitespace() || c == ')').unwrap_or(after.len());
        let fmt = &after[..end];
        return Some(if fmt == "pdflatex" { PassKind::Pdflatex } else { PassKind::Other(fmt.to_string()) });
    }
    let end = rest.find(',').unwrap_or(rest.len());
    Some(PassKind::Other(rest[..end].to_string()))
}

/// Coordinates [`LineJoiner`], [`FileStack`], and [`MessageMatcher`] into a
/// stream of [`Event`]s, per the pipeline in `PLAN.md`.
#[derive(Default)]
pub struct LogParser {
    joiner: LineJoiner,
    stack: FileStack,
    matcher: MessageMatcher,
    /// Once a latexmk `Run number` line has been seen, it is the
    /// authoritative pass-boundary signal for the rest of the stream, and
    /// engine banners (which always precede it in the same first pass under
    /// latexmk) are no longer treated as separate boundaries.
    seen_run_number: bool,
}

impl LogParser {
    pub fn new() -> Self {
        Self {
            joiner: LineJoiner::new(),
            stack: FileStack::new(),
            matcher: MessageMatcher::new(),
            seen_run_number: false,
        }
    }

    /// Feed one raw physical line; returns zero or more [`Event`]s.
    pub fn feed(&mut self, raw_line: &str) -> Vec<Event> {
        let mut out = Vec::new();
        let joined_lines = self.joiner.feed(raw_line);
        for joined in joined_lines {
            self.process_logical_line(&joined, &mut out);
        }
        out
    }

    /// Flush any buffered state at EOF (line joiner + in-progress message).
    pub fn finish(&mut self) -> Vec<Event> {
        let mut out = Vec::new();
        let joined_lines = self.joiner.finish();
        for joined in joined_lines {
            self.process_logical_line(&joined, &mut out);
        }
        for raw in self.matcher.finish() {
            out.push(self.to_event(raw));
        }
        out
    }

    fn process_logical_line(&mut self, line: &str, out: &mut Vec<Event>) {
        if let Some(kind) = try_parse_pass_boundary(line) {
            self.seen_run_number = true;
            out.push(Event::PassBoundary(kind));
            return;
        }
        if !self.seen_run_number {
            if let Some(kind) = try_parse_engine_banner(line) {
                out.push(Event::PassBoundary(kind));
                return;
            }
        }
        if let Some(path) = try_parse_pdf_built(line) {
            out.push(Event::PdfBuilt { path });
            return;
        }
        // Finalize any pending message (e.g. a multi-line warning ended by
        // this line) using the file-stack state as of the *previous* line,
        // before this line's own `(`/`)` changes are applied - a pending
        // message belongs to whatever file was open when it started, not
        // whatever this terminating line happens to open next.
        for raw in self.matcher.feed(line) {
            out.push(self.to_event(raw));
        }
        self.stack.process_line(line);
    }

    fn to_event(&self, raw: RawMessage) -> Event {
        // A message that carries its own file (GCC-style errors, BibTeX
        // warnings) isn't positioned within the file-open stack at all, so
        // it uses that file verbatim rather than the stack's current file.
        let file = raw.file_override.unwrap_or_else(|| self.stack.current_file().to_string());
        Event::Message(LogMessage {
            kind: raw.kind,
            text: raw.text,
            file,
            line_range: raw.line_range,
            page: raw.page,
            context: raw.context,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::MessageKind;

    fn run(lines: &[&str]) -> Vec<Event> {
        let mut p = LogParser::new();
        let mut out = Vec::new();
        for line in lines {
            out.extend(p.feed(line));
        }
        out.extend(p.finish());
        out
    }

    #[test]
    fn attaches_file_from_stack_and_tracks_nesting() {
        let events = run(&[
            "(./intro.tex",
            "Package examplepkg Warning: Citation `citekey1' undefined on input line 8.",
            "(./sub.tex",
            "Overfull \\hbox (53.3pt too wide) in paragraph at lines 2--3",
            ")",
            ")",
        ]);
        assert_eq!(events.len(), 2);
        match &events[0] {
            Event::Message(m) => {
                assert_eq!(m.file, "./intro.tex");
                assert_eq!(m.kind, MessageKind::PackageWarning { package: "examplepkg".to_string() });
            }
            _ => panic!("expected message"),
        }
        match &events[1] {
            Event::Message(m) => {
                assert_eq!(m.file, "./sub.tex");
                assert!(matches!(m.kind, MessageKind::OverfullHbox { .. }));
            }
            _ => panic!("expected message"),
        }
    }

    #[test]
    fn pass_boundary_and_pdf_built_do_not_reach_file_stack_or_messages() {
        let events = run(&[
            "Run number 1 of rule 'pdflatex'",
            "(./main.tex",
            "Output written on build/main.pdf (3 pages, 1000 bytes).",
            "Run number 1 of rule 'bibtex build/main'",
            "Run number 2 of rule 'sometool'",
        ]);
        assert_eq!(
            events,
            vec![
                Event::PassBoundary(PassKind::Pdflatex),
                Event::PdfBuilt { path: "build/main.pdf".to_string() },
                Event::PassBoundary(PassKind::Bibtex),
                Event::PassBoundary(PassKind::Other("sometool".to_string())),
            ]
        );
    }

    #[test]
    fn engine_banner_is_pass_boundary_when_no_latexmk_wrapper() {
        // A plain `lualatex file.tex` invocation, with no latexmk `Run
        // number` line at all - the engine banner is the only signal that a
        // pass started.
        let events = run(&["This is LuaHBTeX, Version 1.24.0 (TeX Live 2026)  (format=lualatex 2026.5.19)  1 JAN 2026"]);
        assert_eq!(events, vec![Event::PassBoundary(PassKind::Other("lualatex".to_string()))]);
    }

    #[test]
    fn engine_banner_is_ignored_once_a_run_number_line_is_seen() {
        // Under latexmk, the `Run number` line always precedes the engine
        // banner for the same pass; the banner must not also count as a
        // second, duplicate pass boundary.
        let events = run(&[
            "Run number 1 of rule 'pdflatex'",
            "This is pdfTeX, Version 3.14 (TeX Live 2025) (preloaded format=pdflatex)",
        ]);
        assert_eq!(events, vec![Event::PassBoundary(PassKind::Pdflatex)]);
    }

    #[test]
    fn gcc_style_error_file_override_beats_stack() {
        let events = run(&[
            "(./main.tex",
            "(./chapters/intro.tex",
            "./chapters/intro.tex:42: Package examplepkg Error: Unknown option `foo'.",
        ]);
        match &events[0] {
            Event::Message(m) => assert_eq!(m.file, "./chapters/intro.tex"),
            _ => panic!("expected message"),
        }
    }

    #[test]
    fn fatal_error_is_attributed_to_the_file_open_when_it_occurred_not_a_later_reopen() {
        // Regression test for a real latexmk run: the engine's fatal-error
        // banner is followed by latexmk's own retry-log prose, and then a
        // fresh `lualatex` run reopening ./main.tex and nested files. None
        // of that must be misread as context of the fatal error, and file
        // attribution must stay pinned to whatever was open when the fatal
        // error was read, not drift to whatever the log happens to open
        // next while a runaway message was still "open".
        let events = run(&[
            "(./main.tex",
            "!  ==> Fatal error occurred, no output PDF file produced!",
            "Transcript written on main.log.",
            "Latexmk: applying rule 'lualatex'...",
            "(./main.tex",
            "(/usr/local/texlive/2026/texmf-dist/tex/latex/examplecls/examplecls.cls",
            "Document Class: examplecls 2026/02/02 v3.49.2 Example document class",
        ]);
        let fatal = events
            .iter()
            .find_map(|e| match e {
                Event::Message(m) if m.text.contains("Fatal error occurred") => Some(m),
                _ => None,
            })
            .expect("fatal error message");
        assert_eq!(fatal.file, "./main.tex");
        assert!(fatal.context.is_empty());
    }

    fn read_sample(name: &str) -> String {
        let path = format!("{}/tests/fixtures/{}", env!("CARGO_MANIFEST_DIR"), name);
        std::fs::read_to_string(path).expect("sample log should exist under tests/fixtures")
    }

    #[test]
    fn test_log_integration_counts() {
        let raw = read_sample("test.log");
        let mut p = LogParser::new();
        let mut events = Vec::new();
        for line in raw.lines() {
            events.extend(p.feed(line));
        }
        events.extend(p.finish());

        let count_kind = |pred: &dyn Fn(&MessageKind) -> bool| {
            events
                .iter()
                .filter(|e| matches!(e, Event::Message(m) if pred(&m.kind)))
                .count()
        };
        let citation_warnings = events
            .iter()
            .filter(|e| matches!(e, Event::Message(m) if matches!(&m.kind, MessageKind::PackageWarning{package} if package == "natbib") && m.text.starts_with("Citation `")))
            .count();
        assert_eq!(citation_warnings, 58);

        let fixme_warnings = count_kind(&|k| matches!(k, MessageKind::PackageWarning { package } if package == "FiXme"));
        assert_eq!(fixme_warnings, 15);

        let overfull_hbox = count_kind(&|k| matches!(k, MessageKind::OverfullHbox { .. }));
        assert_eq!(overfull_hbox, 18);

        let underfull_hbox = count_kind(&|k| matches!(k, MessageKind::UnderfullHbox { .. }));
        assert_eq!(underfull_hbox, 3);

        let overfull_vbox = count_kind(&|k| matches!(k, MessageKind::OverfullVbox { .. }));
        assert_eq!(overfull_vbox, 3);

        let bibtex_warnings = count_kind(&|k| matches!(k, MessageKind::BibtexWarning));
        assert_eq!(bibtex_warnings, 2);

        let pdflatex_passes = events.iter().filter(|e| matches!(e, Event::PassBoundary(PassKind::Pdflatex))).count();
        assert_eq!(pdflatex_passes, 3);

        let bibtex_passes = events.iter().filter(|e| matches!(e, Event::PassBoundary(PassKind::Bibtex))).count();
        assert_eq!(bibtex_passes, 2);

        let pdf_built = events.iter().filter(|e| matches!(e, Event::PdfBuilt { .. })).count();
        assert_eq!(pdf_built, 3);
    }

    #[test]
    fn test2_log_integration_hard_errors_and_no_info_noise() {
        let raw = read_sample("test2.log");
        let mut p = LogParser::new();
        let mut events = Vec::new();
        for line in raw.lines() {
            events.extend(p.feed(line));
        }
        events.extend(p.finish());

        let errors: Vec<&LogMessage> = events
            .iter()
            .filter_map(|e| match e {
                Event::Message(m) if matches!(m.kind, MessageKind::LatexError) => Some(m),
                _ => None,
            })
            .collect();
        assert_eq!(errors.len(), 2);
        assert!(errors[0].text.contains("Missing $ inserted"));
        assert_eq!(errors[0].line_range, Some((1145, 1145)));
        assert!(errors[1].text.contains("Display math should end with $$"));
        assert_eq!(errors[1].line_range, Some((1145, 1145)));

        let pdf_built = events.iter().filter(|e| matches!(e, Event::PdfBuilt { .. })).count();
        assert_eq!(pdf_built, 1);

        // test2.log has no latexmk `Run number` wrapper at all - the single
        // pass must still be detected, from the `This is LuaHBTeX ...
        // (format=lualatex ...)` engine banner.
        let passes: Vec<&PassKind> = events
            .iter()
            .filter_map(|e| match e {
                Event::PassBoundary(kind) => Some(kind),
                _ => None,
            })
            .collect();
        assert_eq!(passes, vec![&PassKind::Other("lualatex".to_string())]);

        // LuaLaTeX/package Info noise must never surface as messages.
        let noisy = events.iter().any(|e| match e {
            Event::Message(m) => m.text.contains("luaotfload") || m.text.contains("Lua module"),
            _ => false,
        });
        assert!(!noisy);
    }

    #[test]
    fn test6_log_pdf_backend_warning_does_not_swallow_trailing_file_opens() {
        // Regression test: a `pdf backend` warning fired mid-page-shipout in
        // this real log wraps its destination name across an 80-char
        // Lua-originated physical line, then is immediately followed on the
        // next physical line by the page-close `]`, more page numbers, and
        // two file opens (`./chapters/b/wd.tex`, `./chapters/b/semantics.tex`).
        // Before the fix, the warning's continuation-matching swallowed all
        // of that trailing text into the warning message and never reached
        // the file stack, losing both file opens.
        let raw = read_sample("test6.log");
        let mut p = LogParser::new();
        let mut events = Vec::new();
        for line in raw.lines() {
            events.extend(p.feed(line));
        }
        events.extend(p.finish());

        let dup_dest = events
            .iter()
            .find_map(|e| match e {
                Event::Message(m) if m.text.contains("ignoring duplicate destination") => Some(m),
                _ => None,
            })
            .expect("duplicate destination warning");
        assert_eq!(dup_dest.text, "ignoring duplicate destination with the name 'equation.4.1'");

        // `./chapters/b/wd.tex` and `./chapters/b/semantics.tex` open right
        // after the warning on the same swallowed physical line; the file
        // that follows (`semantics.tex`, which does carry its own hbox
        // warnings) must show up as its own event rather than being fused
        // into the pdf-backend warning's text above.
        let opened_files: Vec<&str> = events
            .iter()
            .filter_map(|e| match e {
                Event::Message(m) => Some(m.file.as_str()),
                _ => None,
            })
            .collect();
        assert!(opened_files.contains(&"./chapters/b/semantics.tex"));
    }
}
