use std::io::Write;
use std::process::{Command, Stdio};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_texsift")
}

fn sample_path(name: &str) -> String {
    format!("{}/tests/fixtures/{}", env!("CARGO_MANIFEST_DIR"), name)
}

fn run_with_stdin(args: &[&str], stdin_content: &str) -> (bool, String) {
    let mut child = Command::new(bin())
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn binary");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin_content.as_bytes())
        .unwrap();
    let output = child.wait_with_output().expect("failed to wait on child");
    (output.status.success(), String::from_utf8_lossy(&output.stdout).into_owned())
}

fn run_with_args(args: &[&str]) -> (bool, String) {
    let output = Command::new(bin()).args(args).output().expect("failed to run binary");
    (output.status.success(), String::from_utf8_lossy(&output.stdout).into_owned())
}

#[test]
fn stdin_mode_has_no_summary_footer() {
    let raw = std::fs::read_to_string(sample_path("test.log")).unwrap();
    let (ok, stdout) = run_with_stdin(&["--no-color"], &raw);
    assert!(ok);
    assert!(!stdout.contains(" errors ·"), "stdin mode must not print the summary footer");
}

#[test]
fn file_mode_prints_summary_footer_with_expected_counts() {
    let (ok, stdout) = run_with_args(&["--no-color", &sample_path("test.log")]);
    assert!(ok);
    // test.log has no hard `!` errors, only warnings and box diagnostics.
    // 186 warnings = every `Package`/`Class`/`LaTeX`/`FiXme` Warning: line
    // (62 + 0 + 107 + 15) plus 2 BibTeX warnings, independently verified via
    // `grep -c` against the raw log (no deduplication across the log's 3
    // pdflatex passes, per PLAN.md).
    assert!(stdout.contains("0 errors · 186 warnings · 21 overfull · 3 underfull"), "unexpected footer, got tail:\n{}", &stdout[stdout.len().saturating_sub(300)..]);
}

#[test]
fn no_color_flag_strips_all_ansi_codes() {
    let (ok, stdout) = run_with_args(&["--no-color", &sample_path("test.log")]);
    assert!(ok);
    assert!(!stdout.contains('\u{1b}'), "output should contain no ANSI escape sequences");
}

#[test]
fn ascii_flag_swaps_glyphs() {
    let raw = std::fs::read_to_string(sample_path("test2.log")).unwrap();
    let (ok, stdout) = run_with_stdin(&["--ascii", "--no-color"], &raw);
    assert!(ok);
    assert!(stdout.contains('x'), "ascii error glyph should appear for the hard errors in test2.log");
    assert!(!stdout.contains('✕'));
    assert!(!stdout.contains('▲'));
    assert!(!stdout.contains('■'));
    assert!(!stdout.contains('□'));
}

#[test]
fn width_flag_controls_separator_length() {
    let (ok, stdout) = run_with_args(&["--no-color", "--width=40", &sample_path("test.log")]);
    assert!(ok);
    let separator = stdout.lines().find(|l| l.starts_with("──")).expect("expected a pass separator line");
    assert_eq!(separator.chars().count(), 40);
}

#[test]
fn bare_no_warn_suppresses_every_warning() {
    let (ok, stdout) = run_with_args(&["--no-color", "--no-warn", &sample_path("test6.log")]);
    assert!(ok);
    assert!(!stdout.contains('▲'), "bare --no-warn should suppress every warning");
}

#[test]
fn no_warn_with_package_value_suppresses_only_that_package() {
    // `pdf backend` diagnostics come from the engine, not a LaTeX package,
    // so the `silence` package can't hide them - `--no-warn=pdf-backend`
    // needs to scope down to just those, without touching other warnings.
    let (ok, without_flag) = run_with_args(&["--no-color", &sample_path("test6.log")]);
    assert!(ok);
    assert!(without_flag.contains("pdf backend"), "test6.log should contain pdf backend warnings");

    let (ok, with_flag) = run_with_args(&["--no-color", "--no-warn=pdf-backend", &sample_path("test6.log")]);
    assert!(ok);
    assert!(!with_flag.contains("pdf backend"), "pdf backend warnings should be suppressed");
    assert!(with_flag.contains('▲'), "other warnings should still be shown");
}

#[test]
fn no_warn_takes_a_comma_separated_package_list() {
    let (ok, stdout) =
        run_with_args(&["--no-color", "--no-warn=pdf-backend,LaTeX", &sample_path("test6.log")]);
    assert!(ok);
    assert!(!stdout.contains("pdf backend"));
    assert!(!stdout.contains("▲ LaTeX:"));
    assert!(stdout.contains("▲ Package scrbook:"), "unrelated package warnings should still be shown");
}

#[test]
fn width_zero_behaves_like_unset_and_auto_detects() {
    // `--width=0` is an explicit "auto-detect" request, not a literal
    // zero-width terminal - it should produce the same output as omitting
    // the flag entirely (both fall back to 80 columns when not run in a
    // real terminal).
    let (ok_unset, stdout_unset) = run_with_args(&["--no-color", &sample_path("test.log")]);
    let (ok_zero, stdout_zero) = run_with_args(&["--no-color", "--width=0", &sample_path("test.log")]);
    assert!(ok_unset);
    assert!(ok_zero);
    assert_eq!(stdout_unset, stdout_zero);
}

#[test]
fn lua_runtime_error_is_reported() {
    let (ok, stdout) = run_with_args(&["--no-color", &sample_path("test7.log")]);
    assert!(ok);
    assert!(stdout.contains("[\\directlua]"), "Lua chunk-name header should appear, got tail:\n{}", &stdout[stdout.len().saturating_sub(500)..]);
    assert!(stdout.contains("')' expected near '.'."), "Lua error text should be shown");
    assert!(stdout.contains("1 errors"), "summary footer should count the Lua error, got tail:\n{}", &stdout[stdout.len().saturating_sub(300)..]);
}
