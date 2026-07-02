use std::io::Write;

use colored::{Color, Colorize};

use crate::model::{Category, Event, LogMessage, MessageKind, PassKind};

#[derive(Debug, Clone, Copy)]
pub struct RenderOptions {
    pub ascii: bool,
    pub color: bool,
    pub width: usize,
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self { ascii: false, color: true, width: 80 }
    }
}

/// Whether `package` renders as a bare label (`LaTeX:`, `FiXme:`, or any
/// multi-word tag like `pdfTeX (ext4):`) rather than the usual `Package foo:`
/// form.
fn label_is_bare(package: &str) -> bool {
    package == "LaTeX" || package == "FiXme" || package.contains(' ')
}

fn glyph_and_color(kind: &MessageKind, ascii: bool) -> (&'static str, Color) {
    match kind {
        MessageKind::LatexError | MessageKind::PackageError { .. } => {
            (if ascii { "x" } else { "✕" }, Color::BrightRed)
        }
        MessageKind::PackageWarning { .. } | MessageKind::MissingChar | MessageKind::BibtexWarning => {
            (if ascii { "!" } else { "⚠" }, Color::Yellow)
        }
        MessageKind::OverfullHbox { .. } | MessageKind::OverfullVbox { .. } => {
            (if ascii { ">" } else { "»" }, Color::Magenta)
        }
        MessageKind::UnderfullHbox { .. } => (if ascii { "<" } else { "«" }, Color::Magenta),
        MessageKind::ShowOutput { .. } => (if ascii { "?" } else { "⊢" }, Color::Blue),
    }
}

/// Fully streaming renderer: each [`Event`] is printed as soon as it
/// arrives - nothing is buffered waiting for a pass boundary, so output
/// keeps pace with `latexmk -pvc` piping diagnostics in live as they're
/// parsed, rather than only after a whole pass completes.
///
/// Messages are still grouped under a file header, but that grouping is
/// based purely on *immediate* repetition: a new header is printed whenever
/// the current message's file differs from the previous message's file. If
/// the same file's messages are non-contiguous in the stream (interleaved
/// with another file's), its header is printed again on each return - the
/// tradeoff for not holding anything back. Nesting per
/// [`LogMessage::ancestors`] still collapses "invisible" wrapper files with
/// no messages of their own, computed from what's been seen so far this
/// pass (see [`handle`]).
pub struct Renderer<W: Write> {
    out: W,
    opts: RenderOptions,
    printed_anything: bool,
    current_file: Option<String>,
    files_seen_this_pass: std::collections::HashSet<String>,
    error_count: usize,
    warning_count: usize,
    overfull_count: usize,
    underfull_count: usize,
}

impl<W: Write> Renderer<W> {
    pub fn new(out: W, opts: RenderOptions) -> Self {
        // `colored` auto-suppresses ANSI codes based on its own TTY
        // detection of the real process stdout, which is wrong here: the
        // renderer can write to any `Write` sink (a file, a pipe, a test
        // buffer), and coloring should be governed solely by `opts.color`
        // (ultimately the `--no-color` flag), not terminal auto-detection.
        // Force the override once, monotonically, so it never races with
        // concurrently-running tests that also construct a `Renderer`.
        static FORCE_COLORED_OVERRIDE: std::sync::Once = std::sync::Once::new();
        FORCE_COLORED_OVERRIDE.call_once(|| colored::control::set_override(true));
        Self {
            out,
            opts,
            printed_anything: false,
            current_file: None,
            files_seen_this_pass: std::collections::HashSet::new(),
            error_count: 0,
            warning_count: 0,
            overfull_count: 0,
            underfull_count: 0,
        }
    }

    pub fn handle(&mut self, event: Event) {
        match event {
            Event::Message(m) => {
                self.tally(&m.kind);
                // A file's visible nesting only counts ancestors that
                // *already* have a message of their own earlier in this
                // pass's stream. A real but message-less wrapper file - one
                // TeX opened but that never produced (or hasn't yet
                // produced, at this point in the stream) a diagnostic of its
                // own - does not add a level of indentation for whatever is
                // nested inside it.
                let depth = m.ancestors.iter().filter(|a| self.files_seen_this_pass.contains(a.as_str())).count();
                if self.current_file.as_deref() != Some(m.file.as_str()) {
                    if self.printed_anything {
                        writeln!(self.out).ok();
                    }
                    // An empty `file` means no input file was open when the
                    // message was produced (e.g. pdf-backend bookkeeping
                    // warnings issued during final page shipout, after every
                    // real file has already closed) - there's nothing
                    // meaningful to head the group with, so skip the header
                    // line rather than printing a blank one.
                    if !m.file.is_empty() {
                        self.print_file_header(&m.file, depth);
                    }
                    self.current_file = Some(m.file.clone());
                    self.files_seen_this_pass.insert(m.file.clone());
                    self.printed_anything = true;
                }
                self.print_message(&m, depth);
            }
            Event::PassBoundary(kind) => {
                self.print_pass_separator(&pass_label(&kind));
                self.current_file = None;
                self.files_seen_this_pass.clear();
            }
            Event::PdfBuilt { path } => {
                self.print_pdf_built(&path);
                self.current_file = None;
                self.files_seen_this_pass.clear();
            }
        }
    }

    /// No-op kept for API stability - nothing is buffered, so there is
    /// nothing left to flush at EOF.
    pub fn finish(&mut self) {}

    /// Flush the underlying writer. Callers using a buffered `W` (e.g.
    /// `BufWriter`) must call this before the process exits, since a
    /// `BufWriter` silently drops flush errors on `Drop`.
    pub fn flush(&mut self) -> std::io::Result<()> {
        self.out.flush()
    }

    /// Print the summary footer (file input only - never call for stdin).
    pub fn render_summary(&mut self) {
        let dash = if self.opts.ascii { "-" } else { "─" };
        writeln!(self.out, "{}", dash.repeat(self.opts.width)).ok();
        let errors = format!("{} errors", self.error_count);
        let errors = if self.error_count > 0 {
            self.paint(&errors, Color::Red)
        } else {
            self.paint(&errors, Color::Green)
        };
        let warnings = self.paint(&format!("{} warnings", self.warning_count), Color::Yellow);
        let overfull = self.paint(&format!("{} overfull", self.overfull_count), Color::Magenta);
        let underfull = self.paint(&format!("{} underfull", self.underfull_count), Color::Magenta);
        writeln!(self.out, "{errors} · {warnings} · {overfull} · {underfull}").ok();
    }

    fn tally(&mut self, kind: &MessageKind) {
        match kind.category() {
            Category::Error => self.error_count += 1,
            Category::OverfullBox => self.overfull_count += 1,
            Category::UnderfullBox => self.underfull_count += 1,
            Category::Warning => self.warning_count += 1,
        }
    }

    fn print_file_header(&mut self, file: &str, depth: usize) {
        let indent = "  ".repeat(depth);
        let painted = self.paint(file, Color::Green);
        writeln!(self.out, "{indent}{painted}").ok();
    }

    fn print_message(&mut self, m: &LogMessage, depth: usize) {
        let indent = "  ".repeat(depth + 1);
        let (glyph, color) = glyph_and_color(&m.kind, self.opts.ascii);
        let glyph_painted = self.paint_bold(glyph, color);
        let (label_plain, label_painted) = self.render_label_parts(m);
        let text = self.render_body_text(m);
        let location = self.render_location(m);

        // Continuation lines align under where the free-form text starts
        // (after the glyph and any "Package foo: " label), not under the
        // glyph itself - the label doesn't repeat on wrapped lines.
        let prefix_width = indent.chars().count() + glyph.chars().count() + 1 + label_plain.chars().count();
        let content_width = self.opts.width.saturating_sub(prefix_width);
        let lines = wrap_plain(&text, content_width);
        let cont_indent = " ".repeat(prefix_width);

        let last = lines.len() - 1;
        for (i, line) in lines.iter().enumerate() {
            if i == 0 {
                write!(self.out, "{indent}{glyph_painted} {label_painted}{line}").ok();
            } else {
                write!(self.out, "{cont_indent}{line}").ok();
            }
            if i == last && let Some(loc) = &location {
                write!(self.out, "  {loc}").ok();
            }
            writeln!(self.out).ok();
        }
        self.print_context(m, &indent, color);
    }

    /// The free-form message text, before any label prefix - this is the
    /// part that gets word-wrapped.
    fn render_body_text(&self, m: &LogMessage) -> String {
        match &m.kind {
            MessageKind::PackageError { .. } | MessageKind::PackageWarning { .. } => m.text.clone(),
            MessageKind::BibtexWarning => m.text.clone(),
            MessageKind::OverfullHbox { pt } => format!("Overfull \\hbox {pt:.2}pt too wide"),
            MessageKind::OverfullVbox { pt } => format!("Overfull \\vbox {pt:.2}pt too high"),
            MessageKind::UnderfullHbox { badness } => format!("Underfull \\hbox badness {badness}"),
            MessageKind::LatexError | MessageKind::MissingChar => m.text.clone(),
            MessageKind::ShowOutput { .. } => m.text.clone(),
        }
    }

    /// The "Package foo: " / "BibTeX: " label that precedes the body text,
    /// as both a plain (unstyled, for width accounting) and painted string.
    /// Empty for message kinds with no label.
    fn render_label_parts(&self, m: &LogMessage) -> (String, String) {
        match &m.kind {
            MessageKind::PackageError { package } | MessageKind::PackageWarning { package } => {
                (format!("{}: ", self.render_label_plain(package)), format!("{}: ", self.render_label(package)))
            }
            MessageKind::BibtexWarning => ("BibTeX: ".to_string(), format!("{}: ", self.paint_bold("BibTeX", Color::White))),
            _ => (String::new(), String::new()),
        }
    }

    fn render_label_plain(&self, package: &str) -> String {
        if label_is_bare(package) {
            package.to_string()
        } else {
            format!("Package {package}")
        }
    }

    fn render_label(&self, package: &str) -> String {
        if label_is_bare(package) {
            self.paint_bold(package, Color::White)
        } else {
            format!("{} {}", self.paint(&"Package".to_string(), Color::BrightBlack), self.paint_bold(package, Color::White))
        }
    }

    fn render_location(&self, m: &LogMessage) -> Option<String> {
        let text = match &m.kind {
            MessageKind::OverfullHbox { .. } | MessageKind::OverfullVbox { .. } | MessageKind::UnderfullHbox { .. } => {
                match m.line_range {
                    Some((n, mm)) if n == mm => Some(format!("(line {n})")),
                    Some((n, mm)) => Some(format!("(lines {n}{}{mm})", if self.opts.ascii { "--" } else { "–" })),
                    None => Some("(output active)".to_string()),
                }
            }
            MessageKind::LatexError | MessageKind::PackageError { .. } | MessageKind::ShowOutput { .. } => {
                m.line_range.map(|(n, _)| format!("(line {n})"))
            }
            MessageKind::PackageWarning { .. } | MessageKind::BibtexWarning => match (m.line_range, m.page) {
                (Some((n, _)), Some(p)) => Some(format!("(line {n}, page {p})")),
                (Some((n, _)), None) => Some(format!("(line {n})")),
                (None, Some(p)) => Some(format!("(page {p})")),
                (None, None) => None,
            },
            MessageKind::MissingChar => None,
        };
        text.map(|t| self.paint(&t, Color::BrightBlack))
    }

    fn print_context(&mut self, m: &LogMessage, indent: &str, color: Color) {
        let bar_glyph = if self.opts.ascii { "|" } else { "│" };
        let bar = self.paint(bar_glyph, color);
        let content_width = self.opts.width.saturating_sub(indent.chars().count() + bar_glyph.chars().count() + 1);
        for entry in &m.context {
            for line in entry.split('\n') {
                for wrapped in wrap_plain(line, content_width) {
                    writeln!(self.out, "{indent}{bar} {wrapped}").ok();
                }
            }
        }
    }

    fn print_pass_separator(&mut self, label: &str) {
        if self.printed_anything {
            writeln!(self.out).ok();
        }
        let dash = if self.opts.ascii { "-" } else { "─" };
        let prefix = format!("{}{} {} ", dash, dash, label);
        let prefix_len = prefix.chars().count();
        let fill = if prefix_len < self.opts.width { self.opts.width - prefix_len } else { 0 };
        let line = format!("{prefix}{}", dash.repeat(fill));
        writeln!(self.out, "{}", self.paint(&line, Color::BrightBlack)).ok();
        self.printed_anything = true;
    }

    fn print_pdf_built(&mut self, path: &str) {
        if self.printed_anything {
            writeln!(self.out).ok();
        }
        let glyph = if self.opts.ascii { "*" } else { "✔" };
        writeln!(self.out, "{}", self.paint(&format!("{glyph} PDF written: {path}"), Color::Green)).ok();
        self.printed_anything = true;
    }

    fn paint(&self, s: &str, color: Color) -> String {
        if self.opts.color {
            s.color(color).to_string()
        } else {
            s.to_string()
        }
    }

    fn paint_bold(&self, s: &str, color: Color) -> String {
        if self.opts.color {
            s.color(color).bold().to_string()
        } else {
            s.to_string()
        }
    }
}

/// Word-wrap `text` to `width` columns (character count, ANSI-unaware -
/// callers must pass plain, unpainted text). `width == 0` means "no limit",
/// used both as an explicit opt-out and as the saturating-subtraction
/// result when the prefix already consumes the whole terminal width.
///
/// When `text` already fits, it's returned unmodified (byte-for-byte) so
/// callers relying on exact whitespace (e.g. a source-snippet line whose
/// leading spaces line up a caret under the token above) aren't affected
/// by the common case. Wrapping only kicks in for lines that actually
/// overflow, at which point runs of whitespace are normalized to single
/// spaces as part of reflowing.
fn wrap_plain(text: &str, width: usize) -> Vec<String> {
    if width == 0 || text.chars().count() <= width {
        return vec![text.to_string()];
    }
    let mut lines = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        let word_len = word.chars().count();
        if !current.is_empty() && current.chars().count() + 1 + word_len > width {
            lines.push(std::mem::take(&mut current));
        }
        let mut rest = word;
        while rest.chars().count() > width {
            if !current.is_empty() {
                lines.push(std::mem::take(&mut current));
            }
            let (head, tail) = split_at_char(rest, width);
            lines.push(head.to_string());
            rest = tail;
        }
        if !current.is_empty() {
            current.push(' ');
        }
        current.push_str(rest);
    }
    if !current.is_empty() || lines.is_empty() {
        lines.push(current);
    }
    lines
}

/// Split `s` at the `n`th character boundary (not byte offset).
fn split_at_char(s: &str, n: usize) -> (&str, &str) {
    match s.char_indices().nth(n) {
        Some((idx, _)) => s.split_at(idx),
        None => (s, ""),
    }
}

fn pass_label(kind: &PassKind) -> String {
    match kind {
        PassKind::Pdflatex => "pdflatex".to_string(),
        PassKind::Bibtex => "bibtex".to_string(),
        PassKind::Other(name) => name.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(events: Vec<Event>, opts: RenderOptions) -> String {
        let mut buf: Vec<u8> = Vec::new();
        {
            let mut r = Renderer::new(&mut buf, opts);
            for e in events {
                r.handle(e);
            }
            r.finish();
        }
        String::from_utf8(buf).unwrap()
    }

    fn no_color(width: usize) -> RenderOptions {
        RenderOptions { ascii: false, color: false, width }
    }

    fn ascii_no_color(width: usize) -> RenderOptions {
        RenderOptions { ascii: true, color: false, width }
    }

    /// A [`LogMessage`] with sensible defaults (no ancestors, no
    /// line/page, no context) - callers override only the fields that
    /// matter for what they're testing via struct-update syntax.
    fn message(kind: MessageKind, file: &str, text: &str) -> LogMessage {
        LogMessage {
            kind,
            text: text.to_string(),
            file: file.to_string(),
            ancestors: vec![],
            line_range: None,
            page: None,
            context: vec![],
        }
    }

    fn warning(file: &str, ancestors: &[&str], package: &str, text: &str, line: u32, page: u32) -> Event {
        Event::Message(LogMessage {
            ancestors: ancestors.iter().map(|s| s.to_string()).collect(),
            line_range: Some((line, line)),
            page: Some(page),
            ..message(MessageKind::PackageWarning { package: package.to_string() }, file, text)
        })
    }

    #[test]
    fn consecutive_messages_for_the_same_file_share_one_header() {
        let out = render(
            vec![
                warning("./intro.tex", &[], "examplepkg", "Citation `key1' undefined", 8, 1),
                warning("./intro.tex", &[], "examplepkg", "Citation `key2' undefined", 8, 1),
            ],
            no_color(80),
        );
        assert_eq!(
            out,
            "./intro.tex\n\
             \x20 ! Package examplepkg: Citation `key1' undefined  (line 8, page 1)\n\
             \x20 ! Package examplepkg: Citation `key2' undefined  (line 8, page 1)\n"
                .replace('!', "⚠")
        );
    }

    #[test]
    fn messages_print_immediately_rather_than_waiting_to_be_grouped() {
        // Streaming mode (piping from latexmk -pvc) must show each
        // diagnostic as soon as it's parsed, not buffer a whole pass and
        // reorder by file. So if ./intro.tex's messages are interrupted by
        // an unrelated ./sub.tex message, ./intro.tex's header prints again
        // on return, rather than being retroactively grouped together.
        let out = render(
            vec![
                warning("./intro.tex", &[], "examplepkg", "Citation `key1' undefined", 8, 1),
                warning("./sub.tex", &["./intro.tex"], "examplepkg", "Something", 2, 1),
                warning("./intro.tex", &[], "examplepkg", "Citation `key2' undefined", 8, 1),
            ],
            no_color(80),
        );
        assert_eq!(
            out,
            "./intro.tex\n\
             \x20 ! Package examplepkg: Citation `key1' undefined  (line 8, page 1)\n\
             \n\
             \x20 ./sub.tex\n\
             \x20\x20\x20\x20! Package examplepkg: Something  (line 2, page 1)\n\
             \n\
             ./intro.tex\n\
             \x20 ! Package examplepkg: Citation `key2' undefined  (line 8, page 1)\n"
                .replace('!', "⚠")
        );
    }

    #[test]
    fn empty_file_transition_skips_the_header_line() {
        // Some messages (e.g. pdf-backend "unreferenced destination"
        // warnings during final page shipout) arrive with an empty `file`
        // because no input file is open at that point in the log. A file
        // transition into "" must not print a blank header line - just the
        // usual single blank-line separator, then the message.
        let out = render(
            vec![
                warning("./chapters/intro.tex", &[], "examplepkg", "Something", 2, 1),
                warning("", &[], "pdf backend", "unreferenced destination with name 'x'", 0, 0),
            ],
            no_color(80),
        );
        assert_eq!(
            out,
            "./chapters/intro.tex\n\
             \x20 ⚠ Package examplepkg: Something  (line 2, page 1)\n\
             \n\
             \x20 ⚠ pdf backend: unreferenced destination with name 'x'  (line 0, page 0)\n"
        );
    }

    #[test]
    fn message_less_wrapper_file_is_collapsed_out_of_indentation() {
        // ./wrapper.tex sits between ./main.tex and ./sub.tex in the real
        // file-open chain, but never has any messages of its own, so it
        // never gets a header printed - its child should render at the same
        // depth as a normal top-level file, not two levels deep.
        let out = render(
            vec![warning("./sub.tex", &["./main.tex", "./wrapper.tex"], "examplepkg", "Something", 2, 1)],
            no_color(80),
        );
        assert_eq!(
            out,
            "./sub.tex\n  ⚠ Package examplepkg: Something  (line 2, page 1)\n"
        );
    }

    #[test]
    fn ancestor_message_arriving_later_in_the_pass_does_not_retroactively_nest_its_earlier_child() {
        // ./main.tex is a real ancestor of build/main.aux (TeX opens the aux
        // file from within the still-open main.tex at end-of-document), but
        // main.tex's own message in this pass happens to come *after*
        // build/main.aux's first message in the log stream. build/main.aux
        // must render at depth 0, not nested under a parent that "becomes
        // visible" only retroactively.
        let out = render(
            vec![
                warning("build/main.aux", &["./main.tex"], "LaTeX", "Label multiply defined", 10, 1),
                Event::Message(message(
                    MessageKind::PackageWarning { package: "natbib".to_string() },
                    "./main.tex",
                    "There were undefined citations",
                )),
            ],
            no_color(80),
        );
        assert_eq!(
            out,
            "build/main.aux\n  ⚠ LaTeX: Label multiply defined  (line 10, page 1)\n\
             \n\
             ./main.tex\n  ⚠ Package natbib: There were undefined citations\n"
        );
    }

    #[test]
    fn ascii_theme_swaps_glyphs_not_colors() {
        let out = render(
            vec![Event::Message(LogMessage {
                line_range: Some((1145, 1145)),
                context: vec!["<inserted text> $".to_string()],
                ..message(MessageKind::LatexError, "./main.tex", "Missing $ inserted.")
            })],
            ascii_no_color(80),
        );
        assert_eq!(
            out,
            "./main.tex\n\
             \x20 x Missing $ inserted.  (line 1145)\n\
             \x20 | <inserted text> $\n"
        );
        assert!(!out.contains('✕'));
        assert!(!out.contains('│'));
    }

    #[test]
    fn width_override_controls_separator_length() {
        let out = render(vec![Event::PassBoundary(PassKind::Pdflatex)], no_color(40));
        let first_line = out.lines().next().unwrap();
        assert_eq!(first_line.chars().count(), 40);
        assert!(first_line.starts_with("── pdflatex "));
    }

    #[test]
    fn summary_footer_colors_by_error_count() {
        let mut buf: Vec<u8> = Vec::new();
        {
            let mut r = Renderer::new(&mut buf, RenderOptions { ascii: false, color: true, width: 20 });
            r.handle(Event::Message(message(MessageKind::LatexError, "./main.tex", "boom")));
            r.finish();
            r.render_summary();
        }
        let out = String::from_utf8(buf).unwrap();
        assert!(out.contains("1 errors"));
        assert!(out.contains("31")); // ANSI red code present somewhere
    }

    #[test]
    fn overfull_box_location_variants() {
        let with_lines = render(
            vec![Event::Message(LogMessage {
                line_range: Some((2, 3)),
                ..message(MessageKind::OverfullHbox { pt: 53.32617 }, "./main.tex", "")
            })],
            no_color(80),
        );
        assert!(with_lines.contains("Overfull \\hbox 53.33pt too wide  (lines 2–3)"));

        let output_active = render(
            vec![Event::Message(message(MessageKind::OverfullHbox { pt: 13.30402 }, "./main.tex", ""))],
            no_color(80),
        );
        assert!(output_active.contains("Overfull \\hbox 13.30pt too wide  (output active)"));
    }

    #[test]
    fn wrap_plain_leaves_short_text_untouched_including_whitespace() {
        // Below the width limit, the exact original string comes back -
        // callers rely on this to preserve meaningful indentation (e.g. a
        // caret column under a source snippet) that split_whitespace would
        // otherwise collapse.
        assert_eq!(wrap_plain("   a  b", 80), vec!["   a  b".to_string()]);
    }

    #[test]
    fn wrap_plain_breaks_on_word_boundaries_at_width() {
        assert_eq!(
            wrap_plain("one two three four", 9),
            vec!["one two".to_string(), "three".to_string(), "four".to_string()]
        );
    }

    #[test]
    fn wrap_plain_hard_breaks_a_single_word_longer_than_width() {
        assert_eq!(wrap_plain("abcdefghij", 4), vec!["abcd".to_string(), "efgh".to_string(), "ij".to_string()]);
    }

    #[test]
    fn wrap_plain_zero_width_means_unbounded() {
        assert_eq!(wrap_plain("one two three four five", 0), vec!["one two three four five".to_string()]);
    }

    #[test]
    fn context_wraps_at_width_with_bar_prefixed_continuation() {
        let out = render(
            vec![Event::Message(LogMessage {
                line_range: Some((1145, 1145)),
                context: vec![
                    "I've inserted a begin-math/end-math symbol since I think you left one out."
                        .to_string(),
                ],
                ..message(MessageKind::LatexError, "./main.tex", "Missing $ inserted.")
            })],
            ascii_no_color(60),
        );
        assert_eq!(
            out,
            "./main.tex\n\
             \x20 x Missing $ inserted.  (line 1145)\n\
             \x20 | I've inserted a begin-math/end-math symbol since I think\n\
             \x20 | you left one out.\n"
        );
    }

    #[test]
    fn long_package_warning_body_wraps_with_continuation_aligned_under_label() {
        let out = render(
            vec![warning("./main.tex", &[], "examplepkg", "Citation for key alpha beta gamma delta undefined", 8, 1)],
            no_color(40),
        );
        assert_eq!(
            out,
            "./main.tex\n\
             \x20 ! Package examplepkg: Citation for key\n\
             \x20                       alpha beta gamma\n\
             \x20                       delta undefined  (line 8, page 1)\n"
                .replace('!', "⚠")
        );
    }

    #[test]
    fn width_zero_disables_wrapping_in_render_options() {
        let long_context = "one two three four five six seven eight nine ten".to_string();
        let out = render(
            vec![Event::Message(LogMessage {
                context: vec![long_context.clone()],
                ..message(MessageKind::LatexError, "./main.tex", "boom")
            })],
            ascii_no_color(0),
        );
        // width: 0 means "unbounded" - the context line stays on one line
        // rather than being broken up.
        assert!(out.contains(&format!("| {long_context}\n")), "expected unwrapped context line, got:\n{out}");
    }
}
