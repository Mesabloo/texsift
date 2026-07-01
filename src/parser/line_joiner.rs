const FRESH_LINE_PREFIXES: [&str; 16] = [
    "Package ",
    "LaTeX ",
    "Class ",
    "Overfull ",
    "Underfull ",
    "! ",
    "FiXme ",
    "Warning--",
    "(",
    ")",
    "[",
    "]",
    "l.",
    "Run number",
    "Latexmk:",
    "Running '",
];

fn starts_fresh_line(line: &str) -> bool {
    if line.is_empty() {
        return true;
    }
    if line.starts_with("------------") {
        return true;
    }
    FRESH_LINE_PREFIXES.iter().any(|p| line.starts_with(p))
}

/// Reassembles TeX's 79-character hard-wrapped physical lines into logical
/// lines, per the algorithm in `PLAN.md`.
#[derive(Debug, Default)]
pub struct LineJoiner {
    pending: Option<String>,
}

impl LineJoiner {
    pub fn new() -> Self {
        Self { pending: None }
    }

    /// Feed one physical line; returns zero or one completed logical lines.
    ///
    /// Whether wrapping continues is decided by the length of the incoming
    /// physical line itself (TeX chops every wrapped fragment at exactly 79
    /// chars except the last), not by the length of the accumulated buffer.
    pub fn feed(&mut self, line: &str) -> Vec<String> {
        let mut out = Vec::new();
        let incoming_is_79 = line.chars().count() == 79;
        let joined = match self.pending.take() {
            Some(pending) if starts_fresh_line(line) => {
                out.push(pending);
                line.to_string()
            }
            Some(mut pending) => {
                pending.push_str(line);
                pending
            }
            None => line.to_string(),
        };
        if incoming_is_79 {
            self.pending = Some(joined);
        } else {
            out.push(joined);
        }
        out
    }

    /// Flush any buffered fragment at EOF.
    pub fn finish(&mut self) -> Vec<String> {
        self.pending.take().into_iter().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed_all(lines: &[&str]) -> Vec<String> {
        let mut j = LineJoiner::new();
        let mut out = Vec::new();
        for line in lines {
            out.extend(j.feed(line));
        }
        out.extend(j.finish());
        out
    }

    /// Split a full logical line into 79-char physical fragments the way TeX
    /// itself would, so tests never have to hand-count characters.
    fn wrap_at_79(full: &str) -> Vec<String> {
        let chars: Vec<char> = full.chars().collect();
        chars.chunks(79).map(|c| c.iter().collect()).collect()
    }

    #[test]
    fn joins_split_path() {
        // A made-up texmf-style path, long enough to wrap at column 79.
        let full = "(/usr/local/texlive/2099/texmf-dist/tex/generic/example/example-utils.tex)";
        let full = format!("{full}{full}"); // force it well past 79 chars
        let frags = wrap_at_79(&full);
        assert!(frags.len() > 1);
        let refs: Vec<&str> = frags.iter().map(String::as_str).collect();
        let out = feed_all(&refs);
        assert_eq!(out, vec![full]);
    }

    #[test]
    fn joins_split_warning_mid_sentence() {
        let full = "Package examplepkg Warning: Citation `somekey' on page 1 undefined on input line 8.";
        let frags = wrap_at_79(full);
        assert!(frags.len() > 1);
        let refs: Vec<&str> = frags.iter().map(String::as_str).collect();
        let out = feed_all(&refs);
        assert_eq!(out, vec![full.to_string()]);
    }

    #[test]
    fn does_not_join_when_next_line_is_fresh() {
        let frag1 = "x".repeat(79);
        let out = feed_all(&[&frag1, "Package examplepkg Warning: something happened here today"]);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0], frag1);
    }

    #[test]
    fn joins_three_way_wrap() {
        let a = "a".repeat(79);
        let b = "b".repeat(79);
        let c = "cc".to_string();
        let out = feed_all(&[&a, &b, &c]);
        assert_eq!(out, vec![format!("{a}{b}{c}")]);
    }

    #[test]
    fn short_lines_pass_through_unchanged() {
        let out = feed_all(&["short line one", "short line two"]);
        assert_eq!(out, vec!["short line one".to_string(), "short line two".to_string()]);
    }

    #[test]
    fn finish_flushes_pending_79_char_line_at_eof() {
        let line79 = "y".repeat(79);
        let mut j = LineJoiner::new();
        assert!(j.feed(&line79).is_empty());
        assert_eq!(j.finish(), vec![line79]);
    }

    #[test]
    fn real_log_round_trip_natbib_citations() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/test.log");
        let raw = std::fs::read_to_string(path).expect("test.log should exist under tests/fixtures");
        let raw_citation_lines = raw
            .lines()
            .filter(|l| l.starts_with("Package natbib Warning: Citation `"))
            .count();

        let mut j = LineJoiner::new();
        let mut joined = Vec::new();
        for line in raw.lines() {
            joined.extend(j.feed(line));
        }
        joined.extend(j.finish());

        assert!(joined.len() < raw.lines().count(), "joining should reduce line count");

        let joined_complete_citations = joined
            .iter()
            .filter(|l| l.starts_with("Package natbib Warning: Citation `") && l.contains("undefined on input line"))
            .count();
        assert_eq!(
            joined_complete_citations, raw_citation_lines,
            "every citation warning should end up as one complete logical line"
        );
    }

    #[test]
    fn real_log_round_trip_lualatex_noise() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/test2.log");
        let raw = std::fs::read_to_string(path).expect("test2.log should exist under tests/fixtures");

        let mut j = LineJoiner::new();
        let mut joined = Vec::new();
        for line in raw.lines() {
            joined.extend(j.feed(line));
        }
        joined.extend(j.finish());

        assert!(joined.len() < raw.lines().count(), "joining should reduce line count");
        assert!(joined.iter().any(|l| l.contains("Missing $ inserted")));
        assert!(joined.iter().any(|l| l.contains("Display math should end with $$")));
    }
}
