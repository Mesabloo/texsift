pub(crate) fn looks_like_path(token: &str) -> bool {
    if token.starts_with("./") || token.starts_with('/') {
        return true;
    }
    match token.chars().next() {
        Some(c) if c.is_alphanumeric() => token.contains('.'),
        _ => false,
    }
}

/// Tracks TeX's `(path` / `)` file-open/close nesting across the log
/// stream. Every `(` pushes an entry - `Some(path)` if it looks like a file
/// path, `None` otherwise (e.g. `(output active)`, `(see manual)`) - so that
/// a later `)` always pops the matching entry, whether or not it was a real
/// file open. [`current_file`] reports the innermost open file, skipping
/// over any non-path entries above it.
#[derive(Debug, Default)]
pub struct FileStack {
    stack: Vec<Option<String>>,
}

impl FileStack {
    pub fn new() -> Self {
        Self { stack: Vec::new() }
    }

    /// The innermost currently-open file path, or `""` if none is open.
    pub fn current_file(&self) -> &str {
        for entry in self.stack.iter().rev() {
            if let Some(path) = entry {
                return path;
            }
        }
        ""
    }

    /// Total paren-nesting depth, including non-path entries.
    pub fn depth(&self) -> usize {
        self.stack.len()
    }

    /// Nesting level of [`current_file`], counting only real file entries:
    /// 0 if no file is open, or if exactly one is (the top-level document);
    /// 1 for a file opened from within that one, etc.
    pub fn file_depth(&self) -> usize {
        self.file_chain().len().saturating_sub(1)
    }

    /// The full chain of currently-open real files, outermost first, e.g.
    /// `["./main.tex", "./compiler.tex", "./compiler_g2n.tex"]`. Non-path
    /// entries are skipped, same as [`current_file`].
    pub fn file_chain(&self) -> Vec<String> {
        self.stack.iter().filter_map(|e| e.clone()).collect()
    }

    /// Scan one already-joined logical line, updating the stack.
    pub fn process_line(&mut self, line: &str) {
        for (i, c) in line.char_indices() {
            match c {
                '(' => {
                    let token_start = i + c.len_utf8();
                    let token_end = Self::scan_token(line, token_start);
                    let token = &line[token_start..token_end];
                    if looks_like_path(token) {
                        self.stack.push(Some(token.to_string()));
                    } else {
                        self.stack.push(None);
                    }
                }
                ')' => {
                    self.stack.pop();
                }
                _ => {}
            }
        }
    }

    fn scan_token(line: &str, start: usize) -> usize {
        line[start..]
            .find(|c: char| c == '(' || c == ')' || c.is_whitespace())
            .map(|offset| start + offset)
            .unwrap_or(line.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dot_slash_prefix_is_a_path() {
        let mut s = FileStack::new();
        s.process_line("(./intro.tex");
        assert_eq!(s.current_file(), "./intro.tex");
        assert_eq!(s.depth(), 1);
    }

    #[test]
    fn absolute_texmf_path_is_a_path() {
        let mut s = FileStack::new();
        s.process_line("(/usr/local/texlive/2025/texmf-dist/tex/latex/base/article.cls");
        assert_eq!(s.current_file(), "/usr/local/texlive/2025/texmf-dist/tex/latex/base/article.cls");
    }

    #[test]
    fn bare_letter_prefix_with_extension_is_a_path() {
        let mut s = FileStack::new();
        s.process_line("(article.cls");
        assert_eq!(s.current_file(), "article.cls");
    }

    #[test]
    fn self_closing_relative_path_nets_to_zero_depth() {
        let mut s = FileStack::new();
        s.process_line("(build/main.aux)");
        assert_eq!(s.depth(), 0);
        assert_eq!(s.current_file(), "");
    }

    #[test]
    fn font_file_path_with_slash_and_extension() {
        let mut s = FileStack::new();
        s.process_line("(type1/urw/uhvb8a.pfb)");
        assert_eq!(s.depth(), 0);
    }

    #[test]
    fn output_active_is_not_a_path() {
        let mut s = FileStack::new();
        s.process_line("(./main.tex");
        s.process_line("(output active");
        // A non-path entry was pushed (depth increases)...
        assert_eq!(s.depth(), 2);
        // ...but it does not become the current file.
        assert_eq!(s.current_file(), "./main.tex");
    }

    #[test]
    fn see_manual_is_not_a_path() {
        let mut s = FileStack::new();
        s.process_line("(./main.tex");
        s.process_line("(see manual)");
        assert_eq!(s.depth(), 1);
        assert_eq!(s.current_file(), "./main.tex");
    }

    #[test]
    fn file_depth_ignores_non_path_entries() {
        let mut s = FileStack::new();
        assert_eq!(s.file_depth(), 0);
        s.process_line("(./main.tex");
        assert_eq!(s.file_depth(), 0); // top-level document, not "nested"
        s.process_line("(see manual");
        assert_eq!(s.file_depth(), 0); // non-path open does not add nesting
        s.process_line("(./sub.tex");
        assert_eq!(s.file_depth(), 1); // one real file nested inside another
    }

    #[test]
    fn file_chain_lists_ancestors_outermost_first() {
        let mut s = FileStack::new();
        assert_eq!(s.file_chain(), Vec::<String>::new());
        s.process_line("(./main.tex");
        assert_eq!(s.file_chain(), vec!["./main.tex".to_string()]);
        s.process_line("(see manual");
        assert_eq!(s.file_chain(), vec!["./main.tex".to_string()]);
        s.process_line("(./wrapper.tex(./sub.tex");
        assert_eq!(
            s.file_chain(),
            vec!["./main.tex".to_string(), "./wrapper.tex".to_string(), "./sub.tex".to_string()]
        );
    }

    #[test]
    fn consecutive_closes_pop_each_level() {
        let mut s = FileStack::new();
        s.process_line("(./a.tex(./b.tex(./c.tex");
        assert_eq!(s.depth(), 3);
        assert_eq!(s.current_file(), "./c.tex");
        s.process_line(")))");
        assert_eq!(s.depth(), 0);
        assert_eq!(s.current_file(), "");
    }

    #[test]
    fn closing_on_empty_stack_does_not_panic() {
        let mut s = FileStack::new();
        s.process_line(")))");
        assert_eq!(s.depth(), 0);
    }

    #[test]
    fn nested_include_under_top_level_file() {
        let mut s = FileStack::new();
        s.process_line("(./main.tex");
        assert_eq!(s.current_file(), "./main.tex");
        s.process_line("(./sub/included.tex");
        assert_eq!(s.current_file(), "./sub/included.tex");
        assert_eq!(s.depth(), 2);
        s.process_line(")");
        assert_eq!(s.current_file(), "./main.tex");
        assert_eq!(s.depth(), 1);
    }

    #[test]
    fn real_log_never_underflows_and_settles_shallow() {
        use crate::parser::line_joiner::LineJoiner;

        for path in [
            concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/test.log"),
            concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/test2.log"),
        ] {
            let raw = std::fs::read_to_string(path).expect("sample log should exist under tests/fixtures");
            let mut joiner = LineJoiner::new();
            let mut stack = FileStack::new();
            for line in raw.lines() {
                for joined in joiner.feed(line) {
                    stack.process_line(&joined);
                }
            }
            for joined in joiner.finish() {
                stack.process_line(&joined);
            }
            // The heuristic can't be perfect on every real-world log, but it
            // should never underflow (checked internally via saturating pop)
            // and should not runaway-accumulate unmatched opens.
            assert!(stack.depth() < 10, "{path}: final depth {} looks like a runaway mismatch", stack.depth());
        }
    }
}
