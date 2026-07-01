use crate::model::MessageKind;

/// A message produced by [`MessageMatcher`], before the coordinator has
/// attached the file-stack-derived `file` field (or, for kinds that carry
/// their own file - GCC-style errors, BibTeX warnings - the coordinator uses
/// `file_override` verbatim instead of the stack top).
#[derive(Debug, Clone, PartialEq)]
pub struct RawMessage {
    pub kind: MessageKind,
    pub text: String,
    pub file_override: Option<String>,
    pub line_range: Option<(u32, u32)>,
    pub page: Option<u32>,
    pub context: Vec<String>,
}

fn is_recognized_event_prefix(line: &str) -> bool {
    const PREFIXES: [&str; 10] = [
        "!",
        "Overfull",
        "Underfull",
        "LaTeX Warning",
        "Package",
        "Class",
        "Missing character",
        "(",
        "[",
        "Warning--",
    ];
    line.starts_with("l.") || line.starts_with("FiXme") || PREFIXES.iter().any(|p| line.starts_with(p))
}

fn looks_like_path(token: &str) -> bool {
    if token.starts_with("./") || token.starts_with('/') {
        return true;
    }
    match token.chars().next() {
        Some(c) if c.is_alphanumeric() => token.contains('.'),
        _ => false,
    }
}

/// Whether `line` ends an in-progress multi-line warning continuation
/// (`PLAN.md`: "accumulate continuation lines until a blank line or a new
/// message prefix is encountered").
///
/// This is deliberately narrower than [`is_recognized_event_prefix`] (used
/// by the `!`-error read-ahead machine, whose terminator list is specified
/// exactly in `PLAN.md`) in two ways that real-world logs need:
/// - A `(` only terminates if what follows actually looks like a file path
///   (`(./intro.tex`, `(/usr/...`), not a short annotation like
///   `(natbib)                Rerun to get citations correct.` - a
///   genuine continuation of the preceding warning, wrapped by natbib
///   itself, not a new file open.
/// - A `<` always terminates: LuaTeX/pdfTeX sometimes print an unrelated
///   `<path/to/font.otf>` font-usage trailer immediately after the final
///   warning of a run, with no blank line separating them; without this,
///   the continuation logic absorbs the entire trailer into the warning.
fn terminates_warning_continuation(line: &str) -> bool {
    if line.starts_with('<') {
        return true;
    }
    if let Some(rest) = line.strip_prefix('(') {
        let token_end = rest
            .find(|c: char| c == '(' || c == ')' || c.is_whitespace())
            .unwrap_or(rest.len());
        return looks_like_path(&rest[..token_end]);
    }
    is_recognized_event_prefix(line)
}

fn try_parse_gcc_style(line: &str) -> Option<(String, u32, String)> {
    let first_colon = line.find(':')?;
    let path = &line[..first_colon];
    if !looks_like_path(path) {
        return None;
    }
    let remainder = &line[first_colon + 1..];
    let second_colon = remainder.find(':')?;
    let num_str = &remainder[..second_colon];
    let n: u32 = num_str.trim().parse().ok()?;
    let after = &remainder[second_colon + 1..];
    let msg = after.strip_prefix(' ').unwrap_or(after);
    if msg.is_empty() {
        return None;
    }
    Some((path.to_string(), n, msg.to_string()))
}

fn classify_error_text(rest: &str) -> (MessageKind, String) {
    if let Some(after) = rest.strip_prefix("Package ") {
        if let Some(idx) = after.find(" Error: ") {
            let pkg = after[..idx].to_string();
            let text = after[idx + " Error: ".len()..].to_string();
            return (MessageKind::PackageError { package: pkg }, text);
        }
    }
    if let Some(after) = rest.strip_prefix("Class ") {
        if let Some(idx) = after.find(" Error: ") {
            let pkg = after[..idx].to_string();
            let text = after[idx + " Error: ".len()..].to_string();
            return (MessageKind::PackageError { package: pkg }, text);
        }
    }
    if let Some(text) = rest.strip_prefix("LaTeX Error: ") {
        return (MessageKind::LatexError, text.to_string());
    }
    (MessageKind::LatexError, rest.to_string())
}

fn try_parse_line_marker(line: &str) -> Option<(u32, String)> {
    let rest = line.strip_prefix("l.")?;
    let end = rest.find(|c: char| !c.is_ascii_digit())?;
    if end == 0 {
        return None;
    }
    let n: u32 = rest[..end].parse().ok()?;
    let after = &rest[end..];
    let after = after.strip_prefix(' ').unwrap_or(after);
    Some((n, after.to_string()))
}

fn extract_marker(text: &str, marker: &str) -> (String, Option<u32>) {
    if let Some(idx) = text.find(marker) {
        let after = &text[idx + marker.len()..];
        let num_end = after
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(after.len());
        if num_end > 0 {
            if let Ok(n) = after[..num_end].parse::<u32>() {
                let mut rest = &after[num_end..];
                if let Some(stripped) = rest.strip_prefix('.') {
                    rest = stripped;
                }
                let combined = format!("{}{}", &text[..idx], rest);
                return (combined, Some(n));
            }
        }
    }
    (text.to_string(), None)
}

fn normalize_spaces(s: &str) -> String {
    s.split(' ').filter(|p| !p.is_empty()).collect::<Vec<_>>().join(" ")
}

fn parse_optional_page_and_line(text: &str) -> (String, Option<u32>, Option<u32>) {
    let (t1, page) = extract_marker(text, "on page ");
    let (t2, line) = extract_marker(&t1, "on input line ");
    let cleaned = normalize_spaces(&t2).trim().to_string();
    (cleaned, line, page)
}

fn try_parse_named_warning(line: &str) -> Option<(String, String)> {
    if let Some(rest) = line.strip_prefix("Package ") {
        if let Some(idx) = rest.find(" Warning: ") {
            return Some((rest[..idx].to_string(), rest[idx + " Warning: ".len()..].to_string()));
        }
        return None;
    }
    if let Some(rest) = line.strip_prefix("Class ") {
        if let Some(idx) = rest.find(" Warning: ") {
            return Some((rest[..idx].to_string(), rest[idx + " Warning: ".len()..].to_string()));
        }
        return None;
    }
    if let Some(idx) = line.find(" Warning: ") {
        let pkg = &line[..idx];
        if !pkg.is_empty() && !pkg.contains(' ') {
            return Some((pkg.to_string(), line[idx + " Warning: ".len()..].to_string()));
        }
    }
    None
}

fn try_parse_engine_warning(line: &str) -> Option<(String, String)> {
    let idx = line.find("warning")?;
    let engine = line[..idx].trim();
    let rest = line[idx + "warning".len()..].trim_start();
    let rest = rest.strip_prefix('(')?;
    let close = rest.find(')')?;
    let tag = &rest[..close];
    let after = rest[close + 1..].strip_prefix(':')?;
    let after = after.strip_prefix(' ').unwrap_or(after);
    let package = if engine.is_empty() {
        tag.to_string()
    } else {
        format!("{} ({})", engine, tag)
    };
    Some((package, after.to_string()))
}

fn parse_box_location(after: &str) -> Option<(u32, u32)> {
    let idx = after.find("at lines ")?;
    let rest = &after[idx + "at lines ".len()..];
    let mut parts = rest.splitn(2, "--");
    let n: u32 = parts.next()?.trim().parse().ok()?;
    let m_part = parts.next()?;
    let end = m_part
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(m_part.len());
    if end == 0 {
        return None;
    }
    let m: u32 = m_part[..end].parse().ok()?;
    Some((n, m))
}

fn try_parse_overfull_hbox(line: &str) -> Option<(f32, Option<(u32, u32)>)> {
    let rest = line.strip_prefix("Overfull \\hbox (")?;
    let idx = rest.find("pt too wide)")?;
    let pt: f32 = rest[..idx].trim().parse().ok()?;
    let after = &rest[idx + "pt too wide)".len()..];
    Some((pt, parse_box_location(after)))
}

fn try_parse_underfull_hbox(line: &str) -> Option<(u32, Option<(u32, u32)>)> {
    let rest = line.strip_prefix("Underfull \\hbox (badness ")?;
    let idx = rest.find(')')?;
    let badness: u32 = rest[..idx].trim().parse().ok()?;
    let after = &rest[idx + 1..];
    Some((badness, parse_box_location(after)))
}

fn try_parse_overfull_vbox(line: &str) -> Option<(f32, Option<(u32, u32)>)> {
    let rest = line.strip_prefix("Overfull \\vbox (")?;
    let idx = rest.find("pt too high)")?;
    let pt: f32 = rest[..idx].trim().parse().ok()?;
    let after = &rest[idx + "pt too high)".len()..];
    Some((pt, parse_box_location(after)))
}

fn extract_show_command(after: &str) -> String {
    for variant in ["\\showbox", "\\showthe", "\\showlists", "\\show"] {
        if let Some(rest) = after.strip_prefix(variant) {
            return rest.trim().to_string();
        }
    }
    after.trim().to_string()
}

#[derive(Debug, Clone)]
struct ErrorPartial {
    kind: MessageKind,
    text: String,
    file_override: Option<String>,
    line_range: Option<(u32, u32)>,
    context: Vec<String>,
    snippet1: Option<String>,
    snippet2: Option<String>,
    /// Set once a `<...>`/untagged context entry has already been collected
    /// before `l.N` is reached: the `l.N` trailing token (e.g. the macro
    /// callsite) is then redundant with that context and is not shown.
    discard_snippet: bool,
}

#[derive(Debug, Clone)]
enum ErrorPhase {
    /// `pending` holds a context line awaiting its indented continuation:
    /// (first_line, is_tagged).
    ContextLines { pending: Option<(String, bool)> },
    /// The `l.N` source line has just been read; the very next line, if any,
    /// is its column continuation. The message finalizes as soon as this
    /// phase resolves - the free-form "help" prose TeX prints after that is
    /// not fixed-length and not reliably delimited on a live terminal
    /// stream, so it's deliberately not collected.
    SourceContinuation,
}

#[derive(Debug, Clone)]
enum State {
    Idle,
    ErrorCtx { partial: Box<ErrorPartial>, phase: ErrorPhase },
    Warning { package: String, buffer: String },
    BibtexPending { text: String },
    ShowCollecting { first: Option<String>, context: Vec<String> },
}

#[derive(Debug, Default)]
pub struct MessageMatcher {
    state: State,
}

impl Default for State {
    fn default() -> Self {
        State::Idle
    }
}

impl MessageMatcher {
    pub fn new() -> Self {
        Self { state: State::Idle }
    }

    pub fn feed(&mut self, line: &str) -> Vec<RawMessage> {
        let mut out = Vec::new();
        let state = std::mem::take(&mut self.state);
        self.step(state, line, &mut out);
        out
    }

    pub fn finish(&mut self) -> Vec<RawMessage> {
        let mut out = Vec::new();
        match std::mem::take(&mut self.state) {
            State::Idle => {}
            State::ErrorCtx { partial, .. } => Self::finalize_error(*partial, &mut out),
            State::Warning { package, buffer } => Self::finalize_warning(package, buffer, &mut out),
            State::BibtexPending { text } => out.push(RawMessage {
                kind: MessageKind::BibtexWarning,
                text,
                file_override: None,
                line_range: None,
                page: None,
                context: vec![],
            }),
            State::ShowCollecting { first, context } => out.push(RawMessage {
                kind: MessageKind::ShowOutput { command: String::new() },
                text: first.unwrap_or_default(),
                file_override: None,
                line_range: None,
                page: None,
                context,
            }),
        }
        out
    }

    fn step(&mut self, state: State, line: &str, out: &mut Vec<RawMessage>) {
        match state {
            State::Idle => self.dispatch_idle(line, out),
            State::ErrorCtx { partial, phase } => self.step_error(*partial, phase, line, out),
            State::Warning { package, buffer } => self.step_warning(package, buffer, line, out),
            State::BibtexPending { text } => self.step_bibtex(text, line, out),
            State::ShowCollecting { first, context } => self.step_show(first, context, line, out),
        }
    }

    fn dispatch_idle(&mut self, line: &str, out: &mut Vec<RawMessage>) {
        if let Some((file, n, msg)) = try_parse_gcc_style(line) {
            Self::start_gcc_error(file, n, &msg, out);
            return;
        }
        if let Some(rest) = line.strip_prefix("! ") {
            self.start_bang_error(rest, out);
            return;
        }
        if line.starts_with("Missing character:") {
            out.push(RawMessage {
                kind: MessageKind::MissingChar,
                text: line.to_string(),
                file_override: None,
                line_range: None,
                page: None,
                context: vec![],
            });
            return;
        }
        if let Some((pt, lr)) = try_parse_overfull_hbox(line) {
            out.push(RawMessage {
                kind: MessageKind::OverfullHbox { pt },
                text: line.to_string(),
                file_override: None,
                line_range: lr,
                page: None,
                context: vec![],
            });
            return;
        }
        if let Some((badness, lr)) = try_parse_underfull_hbox(line) {
            out.push(RawMessage {
                kind: MessageKind::UnderfullHbox { badness },
                text: line.to_string(),
                file_override: None,
                line_range: lr,
                page: None,
                context: vec![],
            });
            return;
        }
        if let Some((pt, lr)) = try_parse_overfull_vbox(line) {
            out.push(RawMessage {
                kind: MessageKind::OverfullVbox { pt },
                text: line.to_string(),
                file_override: None,
                line_range: lr,
                page: None,
                context: vec![],
            });
            return;
        }
        if let Some(text) = line.strip_prefix("Warning--") {
            self.state = State::BibtexPending { text: text.to_string() };
            return;
        }
        if let Some((pkg, text)) = try_parse_named_warning(line) {
            self.state = State::Warning { package: pkg, buffer: text };
            return;
        }
        if let Some((pkg, text)) = try_parse_engine_warning(line) {
            self.state = State::Warning { package: pkg, buffer: text };
            return;
        }
        if let Some(rest) = line.strip_prefix("> ") {
            self.state = State::ShowCollecting {
                first: Some(rest.to_string()),
                context: vec![],
            };
            return;
        }
        // Anything else (Info lines, LaTeX Font Info, LuaLaTeX loader noise,
        // Latexmk wrapper lines, blank lines, etc.) is silently ignored.
    }

    fn start_bang_error(&mut self, rest: &str, out: &mut Vec<RawMessage>) {
        let (kind, text) = classify_error_text(rest);
        // The engine's final abort banner ("==> Fatal error occurred, no
        // output PDF file produced!") is printed directly from shutdown code
        // (`close_files_and_terminate`), not through TeX's normal
        // error/context-display machinery - it never carries a source
        // location or context, so there's nothing to wait for. Treating it
        // like an ordinary bang error left it open hunting for an `l.N` that
        // will never come, which - especially on a live, non-newline-clean
        // terminal stream - can run on for as long as the process keeps
        // producing output (e.g. latexmk's retry log after the abort),
        // silently reattributing the message to whatever file happens to be
        // open by the time something finally closes it.
        if rest.contains("Fatal error occurred") {
            out.push(RawMessage {
                kind,
                text,
                file_override: None,
                line_range: None,
                page: None,
                context: vec![],
            });
            return;
        }
        self.state = State::ErrorCtx {
            partial: Box::new(ErrorPartial {
                kind,
                text,
                file_override: None,
                line_range: None,
                context: vec![],
                snippet1: None,
                snippet2: None,
                discard_snippet: false,
            }),
            phase: ErrorPhase::ContextLines { pending: None },
        };
    }

    /// GCC-style errors already carry a line number (`file:N: message`) with
    /// no `l.N` marker to wait for, so unlike a bang error there is no
    /// context to collect - the message is complete as soon as this line is
    /// parsed.
    fn start_gcc_error(file: String, n: u32, msg: &str, out: &mut Vec<RawMessage>) {
        let (kind, text) = classify_error_text(msg);
        out.push(RawMessage {
            kind,
            text,
            file_override: Some(file),
            line_range: Some((n, n)),
            page: None,
            context: vec![],
        });
    }

    fn step_error(&mut self, mut partial: ErrorPartial, phase: ErrorPhase, line: &str, out: &mut Vec<RawMessage>) {
        match phase {
            ErrorPhase::ContextLines { pending } => {
                if let Some((first_line, tagged)) = pending {
                    // `line` was going to be joined onto `first_line` as its
                    // continuation, but if it's actually the `l.N` marker, a
                    // recognized new event, or a blank separator, it must be
                    // handled as such rather than being unconditionally
                    // absorbed as text - otherwise whether a real terminator
                    // is honored would depend on the parity of how many
                    // untagged context lines happened to precede it (odd
                    // numbers of context lines would hide the very next
                    // terminator inside a joined entry). Flush `first_line`
                    // on its own and reprocess `line` from a clean
                    // `pending: None` state instead.
                    let is_terminator =
                        line.trim().is_empty() || is_recognized_event_prefix(line) || try_parse_line_marker(line).is_some();
                    if is_terminator {
                        let entry = if tagged { first_line.trim_end().to_string() } else { first_line };
                        partial.context.push(entry);
                        self.step_error(partial, ErrorPhase::ContextLines { pending: None }, line, out);
                        return;
                    }
                    let entry = if tagged {
                        format!("{} {}", first_line.trim_end(), line.trim())
                    } else {
                        format!("{}\n{}", first_line, line)
                    };
                    partial.context.push(entry);
                    self.state = State::ErrorCtx {
                        partial: Box::new(partial),
                        phase: ErrorPhase::ContextLines { pending: None },
                    };
                    return;
                }
                if let Some((n, snippet1)) = try_parse_line_marker(line) {
                    partial.line_range = Some((n, n));
                    if partial.context.is_empty() {
                        partial.snippet1 = if snippet1.trim().is_empty() { None } else { Some(snippet1) };
                    } else {
                        partial.discard_snippet = true;
                    }
                    self.state = State::ErrorCtx {
                        partial: Box::new(partial),
                        phase: ErrorPhase::SourceContinuation,
                    };
                    return;
                }
                if is_recognized_event_prefix(line) {
                    Self::finalize_error(partial, out);
                    self.state = State::Idle;
                    self.dispatch_idle(line, out);
                    return;
                }
                if line.trim().is_empty() {
                    self.state = State::ErrorCtx {
                        partial: Box::new(partial),
                        phase: ErrorPhase::ContextLines { pending: None },
                    };
                    return;
                }
                let tagged = line.trim_start().starts_with('<');
                self.state = State::ErrorCtx {
                    partial: Box::new(partial),
                    phase: ErrorPhase::ContextLines {
                        pending: Some((line.to_string(), tagged)),
                    },
                };
            }
            ErrorPhase::SourceContinuation => {
                if line.trim().is_empty() {
                    Self::finalize_error(partial, out);
                    self.state = State::Idle;
                    return;
                }
                if is_recognized_event_prefix(line) {
                    Self::finalize_error(partial, out);
                    self.state = State::Idle;
                    self.dispatch_idle(line, out);
                    return;
                }
                partial.snippet2 = Some(line.to_string());
                Self::finalize_error(partial, out);
                self.state = State::Idle;
            }
        }
    }

    fn finalize_error(mut partial: ErrorPartial, out: &mut Vec<RawMessage>) {
        if !partial.discard_snippet {
            let l1_blank = partial.snippet1.as_deref().map(|s| s.trim().is_empty()).unwrap_or(true);
            let l2_blank = partial.snippet2.as_deref().map(|s| s.trim().is_empty()).unwrap_or(true);
            match (l1_blank, l2_blank) {
                (true, true) => {}
                (false, true) => partial.context.push(partial.snippet1.take().unwrap()),
                (true, false) => partial.context.push(partial.snippet2.take().unwrap()),
                (false, false) => partial.context.push(format!(
                    "{}\n{}",
                    partial.snippet1.take().unwrap(),
                    partial.snippet2.take().unwrap()
                )),
            }
        }
        out.push(RawMessage {
            kind: partial.kind,
            text: partial.text,
            file_override: partial.file_override,
            line_range: partial.line_range,
            page: None,
            context: partial.context,
        });
    }

    fn step_warning(&mut self, package: String, buffer: String, line: &str, out: &mut Vec<RawMessage>) {
        if line.trim().is_empty() {
            Self::finalize_warning(package, buffer, out);
            self.state = State::Idle;
            return;
        }
        if terminates_warning_continuation(line) {
            Self::finalize_warning(package, buffer, out);
            self.state = State::Idle;
            self.dispatch_idle(line, out);
            return;
        }
        let mut new_buffer = buffer;
        new_buffer.push(' ');
        new_buffer.push_str(line.trim());
        self.state = State::Warning { package, buffer: new_buffer };
    }

    fn finalize_warning(package: String, buffer: String, out: &mut Vec<RawMessage>) {
        let (text, line_no, page) = parse_optional_page_and_line(&buffer);
        out.push(RawMessage {
            kind: MessageKind::PackageWarning { package },
            text,
            file_override: None,
            line_range: line_no.map(|n| (n, n)),
            page,
            context: vec![],
        });
    }

    fn step_bibtex(&mut self, text: String, line: &str, out: &mut Vec<RawMessage>) {
        if let Some(rest) = line.strip_prefix("--line ") {
            if let Some(idx) = rest.find(" of file ") {
                let n: u32 = rest[..idx].trim().parse().unwrap_or(0);
                let file = rest[idx + " of file ".len()..].trim().to_string();
                out.push(RawMessage {
                    kind: MessageKind::BibtexWarning,
                    text,
                    file_override: Some(file),
                    line_range: Some((n, n)),
                    page: None,
                    context: vec![],
                });
                self.state = State::Idle;
                return;
            }
        }
        out.push(RawMessage {
            kind: MessageKind::BibtexWarning,
            text,
            file_override: None,
            line_range: None,
            page: None,
            context: vec![],
        });
        self.state = State::Idle;
        self.dispatch_idle(line, out);
    }

    fn step_show(&mut self, first: Option<String>, mut context: Vec<String>, line: &str, out: &mut Vec<RawMessage>) {
        if let Some((n, after)) = try_parse_line_marker(line) {
            let command = extract_show_command(&after);
            out.push(RawMessage {
                kind: MessageKind::ShowOutput { command },
                text: first.unwrap_or_default(),
                file_override: None,
                line_range: Some((n, n)),
                page: None,
                context,
            });
            self.state = State::Idle;
            return;
        }
        // The definition/value dump between the leading `> ` line and the
        // `l.N \show...` line is not always itself `> `-prefixed (e.g. a
        // `\show` macro body continuation) - collect it verbatim, stripping
        // the prefix only when present.
        let entry = line.strip_prefix("> ").unwrap_or(line).to_string();
        context.push(entry);
        self.state = State::ShowCollecting { first, context };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed_all(lines: &[&str]) -> Vec<RawMessage> {
        let mut m = MessageMatcher::new();
        let mut out = Vec::new();
        for line in lines {
            out.extend(m.feed(line));
        }
        out.extend(m.finish());
        out
    }

    #[test]
    fn package_warning_with_line_and_page() {
        let msgs = feed_all(&["Package examplepkg Warning: Citation `citekey1' on page 1 undefined on input line 8."]);
        assert_eq!(msgs.len(), 1);
        assert_eq!(
            msgs[0].kind,
            MessageKind::PackageWarning { package: "examplepkg".to_string() }
        );
        assert_eq!(msgs[0].text, "Citation `citekey1' undefined");
        assert_eq!(msgs[0].line_range, Some((8, 8)));
        assert_eq!(msgs[0].page, Some(1));
    }

    #[test]
    fn class_warning_matches_same_shape() {
        let msgs = feed_all(&["Class examplecls Warning: Something happened on input line 3."]);
        assert_eq!(
            msgs[0].kind,
            MessageKind::PackageWarning { package: "examplecls".to_string() }
        );
        assert_eq!(msgs[0].text, "Something happened");
        assert_eq!(msgs[0].line_range, Some((3, 3)));
    }

    #[test]
    fn catch_all_latex_warning() {
        let msgs = feed_all(&["LaTeX Warning: Reference `ex:foo' undefined on input line 13 on page 1."]);
        assert_eq!(
            msgs[0].kind,
            MessageKind::PackageWarning { package: "LaTeX".to_string() }
        );
        assert_eq!(msgs[0].text, "Reference `ex:foo' undefined");
        assert_eq!(msgs[0].line_range, Some((13, 13)));
        assert_eq!(msgs[0].page, Some(1));
    }

    #[test]
    fn engine_warning_with_engine_prefix() {
        let msgs = feed_all(&["pdfTeX warning (ext4): destination with the same identifier"]);
        assert_eq!(
            msgs[0].kind,
            MessageKind::PackageWarning { package: "pdfTeX (ext4)".to_string() }
        );
        assert_eq!(msgs[0].text, "destination with the same identifier");
    }

    #[test]
    fn engine_warning_without_engine_prefix() {
        let msgs = feed_all(&["warning  (pdf backend): some backend note"]);
        assert_eq!(
            msgs[0].kind,
            MessageKind::PackageWarning { package: "pdf backend".to_string() }
        );
        assert_eq!(msgs[0].text, "some backend note");
    }

    #[test]
    fn missing_character() {
        let msgs = feed_all(&["Missing character: There is no ~ in font examplefont!"]);
        assert_eq!(msgs[0].kind, MessageKind::MissingChar);
        assert_eq!(msgs[0].text, "Missing character: There is no ~ in font examplefont!");
    }

    #[test]
    fn overfull_hbox_output_active() {
        let msgs = feed_all(&["Overfull \\hbox (13.3pt too wide) has occurred while \\output is active"]);
        match msgs[0].kind {
            MessageKind::OverfullHbox { pt } => assert!((pt - 13.3).abs() < 0.001),
            _ => panic!("wrong kind"),
        }
        assert_eq!(msgs[0].line_range, None);
    }

    #[test]
    fn overfull_hbox_with_lines() {
        let msgs = feed_all(&["Overfull \\hbox (53.3pt too wide) in paragraph at lines 2--3"]);
        assert_eq!(msgs[0].line_range, Some((2, 3)));
    }

    #[test]
    fn underfull_hbox_badness() {
        let msgs = feed_all(&["Underfull \\hbox (badness 10000) has occurred while \\output is active"]);
        assert_eq!(msgs[0].kind, MessageKind::UnderfullHbox { badness: 10000 });
    }

    #[test]
    fn overfull_vbox() {
        let msgs = feed_all(&["Overfull \\vbox (50.2pt too high) has occurred while \\output is active"]);
        match msgs[0].kind {
            MessageKind::OverfullVbox { pt } => assert!((pt - 50.2).abs() < 0.001),
            _ => panic!("wrong kind"),
        }
    }

    #[test]
    fn bibtex_warning() {
        let msgs = feed_all(&[
            "Warning--entry type for \"sometype\" isn't style-file defined",
            "--line 42 of file refs.bib",
        ]);
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].kind, MessageKind::BibtexWarning);
        assert_eq!(msgs[0].text, "entry type for \"sometype\" isn't style-file defined");
        assert_eq!(msgs[0].line_range, Some((42, 42)));
        assert_eq!(msgs[0].file_override, Some("refs.bib".to_string()));
    }

    #[test]
    fn fixme_warning_matches_catch_all_row() {
        // FiXme has no dedicated row in the detection table - it's just
        // another single-token label picked up by the `<X> Warning: `
        // catch-all, the same path "LaTeX Warning:" goes through.
        let msgs = feed_all(&["FiXme Warning: 'a made-up note about something' on input line 18."]);
        assert_eq!(
            msgs[0].kind,
            MessageKind::PackageWarning { package: "FiXme".to_string() }
        );
        assert_eq!(msgs[0].text, "'a made-up note about something'");
        assert_eq!(msgs[0].line_range, Some((18, 18)));
    }

    #[test]
    fn show_output() {
        let msgs = feed_all(&["> \\mymacro=macro:", "#1->\\protect \\mymacro {#1}.", "l.10 \\show\\mymacro"]);
        assert_eq!(msgs.len(), 1);
        assert_eq!(
            msgs[0].kind,
            MessageKind::ShowOutput { command: "\\mymacro".to_string() }
        );
        assert_eq!(msgs[0].text, "\\mymacro=macro:");
        assert_eq!(msgs[0].context, vec!["#1->\\protect \\mymacro {#1}.".to_string()]);
        assert_eq!(msgs[0].line_range, Some((10, 10)));
    }

    #[test]
    fn gcc_style_package_error() {
        // The trailing prose line is TeX's free-form help text, which is no
        // longer collected - only the error itself is emitted.
        let msgs = feed_all(&[
            "./mydoc.tex:42: Package examplepkg Error: Unknown option `foo'.",
            "You might have misspelled `foo' or the language is not loaded.",
        ]);
        assert_eq!(msgs.len(), 1);
        assert_eq!(
            msgs[0].kind,
            MessageKind::PackageError { package: "examplepkg".to_string() }
        );
        assert_eq!(msgs[0].text, "Unknown option `foo'.");
        assert_eq!(msgs[0].file_override, Some("./mydoc.tex".to_string()));
        assert_eq!(msgs[0].line_range, Some((42, 42)));
        assert!(msgs[0].context.is_empty());
    }

    #[test]
    fn fatal_error_has_no_context_and_finalizes_immediately() {
        // The engine's shutdown banner never carries an `l.N`/context, unlike
        // ordinary bang errors - it must not sit around waiting for one,
        // absorbing whatever text is produced afterwards (e.g. latexmk's own
        // retry-log prose following a real abort).
        let msgs = feed_all(&[
            "!  ==> Fatal error occurred, no output PDF file produced!",
            "Transcript written on main.log.",
            "Latexmk: Getting log file 'build/main.log'",
        ]);
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].kind, MessageKind::LatexError);
        assert!(msgs[0].text.contains("Fatal error occurred"));
        assert!(msgs[0].context.is_empty());
        assert_eq!(msgs[0].line_range, None);
    }

    #[test]
    fn bang_error_context_terminator_is_honored_regardless_of_line_parity() {
        // A real `l.N` marker landing right after an *odd* number of raw
        // (untagged) context lines used to get paired up with the preceding
        // line and swallowed as plain text, because the join logic never
        // re-checked terminator conditions on the second line of a pair -
        // whether `l.N` was recognized depended on the parity of how many
        // context lines came before it.
        let msgs = feed_all(&["! Undefined control sequence.", "\\foobar", "l.42 \\callsite", "              "]);
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].line_range, Some((42, 42)));
        assert_eq!(msgs[0].context, vec!["\\foobar".to_string()]);
    }

    #[test]
    fn bang_error_simple_tagged_context() {
        // Mirrors the "Missing $ inserted." / "<inserted text>" shape, with
        // placeholder line number and generic file (not present in this
        // module - file is attached later by the coordinator). The trailing
        // hint prose is fed too, to confirm it's simply dropped rather than
        // absorbed into this message or leaking into the next one.
        let msgs = feed_all(&[
            "! Missing $ inserted.",
            "<inserted text> ",
            "                $",
            "l.7 ",
            "     ",
            "Some hint prose describing the mistake, spanning",
            "more than one physical line of hint text.",
        ]);
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].kind, MessageKind::LatexError);
        assert_eq!(msgs[0].text, "Missing $ inserted.");
        assert_eq!(msgs[0].line_range, Some((7, 7)));
        assert_eq!(msgs[0].context, vec!["<inserted text> $".to_string()]);
    }

    #[test]
    fn bang_error_undefined_control_sequence_simple() {
        let msgs = feed_all(&[
            "! Undefined control sequence.",
            "l.42 \\foobar",
            "             some more context",
            "The control sequence at the end of the top line",
            "of your error message was never \\def'ed.",
        ]);
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].text, "Undefined control sequence.");
        assert_eq!(msgs[0].line_range, Some((42, 42)));
        assert_eq!(
            msgs[0].context,
            vec!["\\foobar\n             some more context".to_string()]
        );
    }

    #[test]
    fn bang_error_undefined_control_sequence_untagged_context() {
        let msgs = feed_all(&[
            "! Undefined control sequence.",
            "\\foobar",
            "        some more context",
            "l.42 \\callsite",
            "              ",
            "The control sequence at the end of the top line",
            "of your error message was never \\def'ed.",
        ]);
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].line_range, Some((42, 42)));
        assert_eq!(
            msgs[0].context,
            vec!["\\foobar\n        some more context".to_string()]
        );
    }

    #[test]
    fn ignored_lines_produce_no_messages() {
        let msgs = feed_all(&[
            "Package examplepkg Info: some informational note.",
            "LaTeX Font Info:    Font shape note.",
            "luaotfload | conf : Root cache directory note.",
            "Lua module: examplemod 1.0 some description",
            "Latexmk: Doing something.",
            "------------",
        ]);
        assert!(msgs.is_empty());
    }

    #[test]
    fn multi_line_warning_continuation() {
        let msgs = feed_all(&[
            "Package examplepkg Warning: This warning wraps across",
            "two physical lines on input line 5.",
        ]);
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].text, "This warning wraps across two physical lines");
        assert_eq!(msgs[0].line_range, Some((5, 5)));
    }

    #[test]
    fn warning_continuation_survives_a_package_name_annotation_line() {
        // Some packages (natbib among them) wrap their own warning text
        // across a second physical line prefixed with the package name in
        // parens, e.g. "(examplepkg)                more text.". This isn't
        // a file open - `examplepkg` doesn't look like a path - so it must
        // not be mistaken for one and cut the warning short.
        let msgs = feed_all(&[
            "Package examplepkg Warning: Citation(s) may have changed.",
            "(examplepkg)                Rerun to get citations correct.",
        ]);
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].text, "Citation(s) may have changed. (examplepkg) Rerun to get citations correct.");
    }

    #[test]
    fn warning_continuation_stops_at_an_unrelated_font_trailer() {
        // LuaTeX/pdfTeX sometimes print a `<path/to/font.otf>` font-usage
        // trailer immediately after the very last warning of a run, with no
        // blank line separating them. This is unrelated content, not a
        // continuation, and must not be absorbed into the warning text.
        let msgs = feed_all(&[
            "warning  (pdf backend): unreferenced destination with name 'chapter.1'",
            "</usr/local/texlive/2026/texmf-dist/fonts/opentype/public/example/Example-Bold.otf>",
        ]);
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].text, "unreferenced destination with name 'chapter.1'");
    }
}
