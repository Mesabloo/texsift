# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

texsift is a Rust CLI that filters and colorizes LaTeX/latexmk build logs,
grouping errors, warnings, and box diagnostics by the source file they
occurred in. Primary use case is piping live from `latexmk -pvc`; it also
works pointed at a saved `.log` file.

## Commands

- Build: `cargo build --release` — binary lands at `target/release/texsift`
- Run the full test suite: `cargo test` (CI uses `cargo test --release --verbose`)
- Run a single test: `cargo test <substring-of-test-name>`, e.g.
  `cargo test warning_continuation_stops_at_a_page_close_bracket` — cargo
  matches by substring against the fully-qualified test path, so a unique
  fragment of the name is enough
- Lint: `cargo clippy --release` — the codebase is kept at zero warnings;
  treat any new warning as something to fix, not suppress
- Manually exercise the CLI against a real log:
  `./target/release/texsift --no-color tests/fixtures/test6.log`

Unit tests live inline in each `src/` module behind `#[cfg(test)] mod
tests`. `tests/cli.rs` holds integration tests that spawn the *compiled
binary* as a subprocess and assert on its stdout — useful for anything that
depends on end-to-end CLI flag parsing rather than internal parser/renderer
behavior.

## Architecture

The pipeline, in order, spans four files that need to be read together to
understand how a line of raw log text becomes a rendered diagnostic:

```
src/bin/main.rs (BufRead::lines())
  -> src/parser/mod.rs        LogParser::feed - coordinator
      -> src/parser/line_joiner.rs   LineJoiner   - rejoin hard-wrapped physical lines
      -> src/parser/file_stack.rs    FileStack    - track (path/) nesting -> current file
      -> src/parser/message.rs       MessageMatcher - state machine -> RawMessage
  -> src/model/entry.rs        Event (Message | PassBoundary | OutputBuilt)
  -> src/output/colored.rs     Renderer - streaming, colored, word-wrapped output
```

- **`main.rs`** reads stdin or a file synchronously via `std::io::BufRead`
  (no async runtime - the whole pipeline is single-threaded and strictly
  sequential, one line in / one set of events out, so tokio was removed as
  dead weight). It also owns the `Filter` struct that implements
  `--no-warn`/`--no-boxes`.
- **`LogParser`** (`parser/mod.rs`) is a thin coordinator, not where the real
  logic lives: it hands each already-line-joined logical line to
  `MessageMatcher::feed`, then attaches the current file from `FileStack` to
  produce an `Event`. Pass-boundary lines (`Run number ... of rule '...'`,
  or an engine banner as a fallback when there's no latexmk wrapper) and
  `Output written on ...` lines are intercepted here, before message
  matching ever sees them.
- **`LineJoiner`** reassembles TeX's hard-wrapped physical lines *before*
  anything else sees them. TeX wraps at exactly 79 characters, but LuaTeX's
  own Lua-originated output (module-loading banners, `pdf backend:`
  messages) wraps one character later, at 80 - both are treated as wrap
  points, since real-world logs from the same run mix both.
- **`FileStack`** tracks `(`/`)` file-open nesting purely to answer "what
  file is currently open" for message attribution - it does not drive any
  display indentation (see below).
- **`MessageMatcher`** (`message.rs`, the largest file by far) is a
  hand-rolled state machine recognizing errors, package/class/engine
  warnings, box diagnostics, and `\show` output, several of which continue
  across multiple physical lines. `PLAN.md` documents the exact state
  machine and the "recognized event prefix" list this code implements -
  check it before re-deriving parsing rules from scratch.
- **`Renderer`** (`output/colored.rs`) consumes events fully streaming
  (no buffering a whole pass), so output keeps pace with `latexmk -pvc`.

### Invariants that aren't obvious from a single file

- File headers always render flush-left; messages are indented exactly one
  level under their header regardless of how deeply TeX's real `(`/`)` stack
  is nested at that point. A message with no enclosing file (e.g. a
  `pdf backend` warning issued after every file has closed) skips both the
  header line and its own indentation.
- `--no-warn` optionally takes a comma-separated package list; bare
  `--no-warn` suppresses every warning. Matching is verbatim except for one
  hardcoded alias (`pdf-backend` -> the engine's `pdf backend` label, which
  has a literal space) - it does *not* generically treat `-` as
  interchangeable with a space, since `-` is valid in real package names.
- Unicode glyphs are deliberately picked from font-safe blocks (Geometric
  Shapes, Dingbats, Mathematical Operators) rather than characters in
  Unicode's emoji data set - emoji-eligible characters (like the `⚠`
  warning sign, which this project moved away from) often get rendered wide
  by terminal fonts, which would break the renderer's column-width math (it
  assumes one `char` == one display column). ASCII (`--ascii`) glyphs are a
  separate, independently chosen set, not a decomposition of the Unicode
  ones - see the glyph table in `PLAN.md`.
- `build.rs` embeds the compile-time target triple via `env!("TARGET")`,
  combined with `CARGO_PKG_VERSION`, for `--version` output.
- Pushing a `Cargo.toml` version bump to `main` triggers
  `.github/workflows/release.yml`, which tags a release and uploads
  `texsift-<version>-<triple>.tar.gz`/`.zip` archives (binary + a generated
  `INSTALL.txt`) for Linux/macOS/Windows.
