# texsift

A CLI tool that filters and colorizes LaTeX/latexmk build logs, grouping
errors, warnings, and box diagnostics by the source file they occurred in.

Works great piped from `latexmk -pvc` in continuous rebuild mode, or pointed
at a saved `.log` file.

## Build

Requires a recent stable [Rust toolchain](https://rustup.rs).

```sh
cargo build --release
```

The binary is produced at `target/release/texsift`.

To install it into `$HOME/.cargo/bin` (make sure that's on your `$PATH`):

```sh
cargo install --path .
```

## Usage

```sh
# Stream from latexmk (primary use case)
latexmk -pvc main.tex 2>&1 | texsift

# Read a saved log file
texsift build/main.log
```

Options:

```
--no-warn     Suppress warnings; show only errors and box diagnostics
--no-boxes    Suppress all Overfull/Underfull box diagnostics
--no-color    Disable all terminal colors
--ascii       Use ASCII fallback symbols instead of Unicode glyphs
--width <N>   Override terminal width used for pass separators
```

Run the test suite with `cargo test`.

## Example

Given a noisy `pdflatex` log full of box and math warnings, `texsift` collapses
it down to the diagnostics that matter, grouped by file:

```
$ texsift build/main.log

── pdflatex ────────────────────────────────────────────────────────────────────

./main.tex
  « Underfull \hbox badness 10000  (line 42)
  ✕ Missing $ inserted.  (line 87)
  │ <inserted text> $
  ✕ Display math should end with $$.  (line 87)
  │ <to be read again> \par

✔ PDF written: main.pdf
────────────────────────────────────────────────────────────────────────────────
2 errors · 0 warnings · 0 overfull · 1 underfull
```

Diagnostics are colored in an actual terminal (red errors, yellow warnings);
pass `--no-color` for plain output, e.g. when piping to a file.

## Releases

Pushing a version bump in `Cargo.toml` to `main` triggers CI to tag a release
and attach prebuilt binaries for Linux, macOS, and Windows.

## About

This project was entirely vibe-coded with Claude: [Cowork](https://claude.com/product/cowork)
handled the planning (see [PLAN.md](PLAN.md)), and Claude Code wrote all of the
source code, across multiple passes over several hours of prompting.
