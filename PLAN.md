# texsift — Implementation Plan

## Overview

A Rust CLI tool that reads a LaTeX/latexmk log (from stdin or a file), extracts
errors, warnings, and box diagnostics, and prints them to the terminal colored
and organized by source file, scoped to the file in which they occurred.

Designed for use with `latexmk -pvc` (continuous rebuild mode): the tool stays
alive, processing lines as they stream in from stdin, until stdin is closed
(i.e., latexmk quits). For file input, reads to EOF and exits.

---

## Invocation

```sh
# Stream from latexmk (primary use case)
latexmk -pvc main.tex 2>&1 | texsift

# Read a saved log file
texsift build/main.log
```

---

## CLI flags

Parsed using `clap` (derive API for brevity).

```
texsift [OPTIONS] [FILE]

Arguments:
  [FILE]              Log file to read; if omitted, reads from stdin

Options:
  --no-warn       Suppress warnings; show only errors and box diagnostics
  --no-boxes          Suppress all Overfull/Underfull box diagnostics
  --no-color          Disable all terminal colors
  --ascii               Use ASCII fallback symbols instead of Unicode glyphs
  --width=N           Override terminal width used for message wrapping and
                      pass separators (default: auto-detected, fallback 80;
                      0 also means auto-detect); useful for testing
```

---

## Output symbols

Instead of text badges like `[warn]`, the tool uses a single glyph per message
kind, colored appropriately. Two themes are supported:

**Default (Unicode)**

| Kind | Glyph | Color |
|---|---|---|
| Error | `✕` | Bright red, bold |
| Warning | `⚠` | Yellow |
| Overfull | `»` | Magenta |
| Underfull | `«` | Magenta |
| Missing char | `⚠` | Yellow |
| Show output | `⊢` | Blue |

The `»`/`«` glyphs suggest overflow and underflow directionally.

**ASCII (`--ascii`)**

| Kind | Glyph | Color |
|---|---|---|
| Error | `x` | Bright red, bold |
| Warning | `!` | Yellow |
| Overfull | `>` | Magenta |
| Underfull | `<` | Magenta |
| Missing char | `!` | Yellow |
| Show output | `?` | Blue |

The two flags are aliases. Colors remain the same regardless of theme.

---

## Sample output

### Warnings and box diagnostics (theme A)

```
./intro.tex
  ⚠ Package natbib: Citation `compcert' undefined  (line 8, page 1)
  ⚠ Package natbib: Citation `cakeml' undefined  (line 8, page 1)
  ⚠ LaTeX: Reference `lastpage' undefined  (line 13, page 1)
  » Overfull \hbox 13.30pt too wide  (output active)
  « Underfull \hbox badness 10000  (output active)

  ./PingPongs.tla
    » Overfull \hbox 53.33pt too wide  (lines 2–3)
    » Overfull \hbox 34.43pt too wide  (lines 3–5)

./guarded.tex
  ⚠ FiXme: 'A channel serving a process involving several instances…'  (line 48)

── bibtex ──────────────────────────────────────────────────────────────────────

generic.bib
  ⚠ BibTeX: entry type for "rocq" isn't style-file defined  (line 322)

── pdflatex ────────────────────────────────────────────────────────────────────

./main.aux
  ⚠ LaTeX: Label `ex:algorithm_semantics' multiply defined
```

No summary footer is printed when reading from stdin.

### Hard errors (`!`)

LaTeX hard errors begin with `!` in the raw log. Two formats observed:

**Simple error** — raw log:
```
! Missing $ inserted.
<inserted text> 
                $
l.1145 
       
I've inserted a begin-math/end-math symbol since I think
you left one out. Proceed, with fingers crossed.
```
Rendered (`<inserted text> $` kept; snippet blank so skipped):
```
./listings/TPC-c2-3.pcal
  ✕ Missing $ inserted.  (line 1145)
  │ <inserted text> $
  │
  │ I've inserted a begin-math/end-math symbol since I think
  │ you left one out. Proceed, with fingers crossed.
```

**Undefined control sequence** — when the CS is used directly in source, the
raw log is:
```
! Undefined control sequence.
l.42 \foobar
             some more context
The control sequence at the end of the top line
of your error message was never \def'ed.
```
Rendered (no context blocks; snippet from `l.42` is non-blank):
```
./main.tex
  ✕ Undefined control sequence.  (line 42)
  │ \foobar
  │     some more context
  │
  │ The control sequence at the end of the top line of your error message was
  │ never \def'ed.
```

When the CS appears inside a macro expansion, TeX instead shows the offending
token (or the expansion trace) on its own line before `l.N`:
```
! Undefined control sequence.
\foobar
        some more context
l.42 \callsite
              
The control sequence at the end of the top line
of your error message was never \def'ed.
```
Rendered (untagged context line collected; snippet from `l.42` may be blank):
```
./main.tex
  ✕ Undefined control sequence.  (line 42)
  │ \foobar
  │     some more context
  │
  │ The control sequence at the end of the top line of your error message was
  │ never \def'ed.
```

**Display math error** — raw log:
```
! Display math should end with $$.
<to be read again> 
                   \tex_par:D 
l.1145 
       
The `$' that I just saw supposedly matches a previous `$$'.
So I shall assume that you typed `$$' both times.
```
Rendered (`<to be read again> \tex_par:D` kept; snippet blank so skipped):
```
./listings/TPC-c2-3.pcal
  ✕ Display math should end with $$.  (line 1145)
  │ <to be read again> \tex_par:D
  │
  │ The `$' that I just saw supposedly matches a previous `$$'. So I shall
  │ assume that you typed `$$' both times.
```

**GCC-style error** (emitted by some engines/packages) — raw log:
```
./chapters/intro.tex:42: Package babel Error: Unknown option `latin'.
You might have misspelled `latin' or the language is not loaded.
```
Rendered (file and line extracted from prefix; read-ahead for hint proceeds as normal):
```
./chapters/intro.tex
  ✕ Package babel: Unknown option `latin'.  (line 42)
  │
  │ You might have misspelled `latin' or the language is not loaded.
```

**Show output** — raw log:
```
> \textbf=macro:
#1->\protect \textbf  {#1}.
l.10 \show\textbf
```
Rendered (first `> ` line becomes the main text; remaining lines under `│`):
```
./main.tex
  ⊢ \textbf=macro:  (line 10)
  │ #1->\protect \textbf  {#1}.
```

The parser extracts:
- Error description from the `!` line (stripping only the leading `! `,
  `! LaTeX Error: `, `! Package <X> Error: ` prefixes; all trailing punctuation
  is preserved), or from the GCC-style `file:N: message` pattern.
- Source line number from `l.N` (for `!` errors) or the `N` field (GCC-style).
- `<inserted text>`, `<to be read again>`, `<read *>` and similar `<…>` context
  lines are collected (each joined with its indented continuation) and displayed
  with `│` prefix before the snippet.
- The `l.N` snippet line and its indented continuation are shown with `│` prefix
  when either contains non-whitespace content; both are skipped when blank.
- The hint text — the prose block after the blank line following the snippet —
  is captured, reflowed into a single line (its own internal line breaks are
  TeX's hardcoded formatting, not meaningful structure), and displayed with
  `│` prefixes after a blank `│` separator.

### File input — summary footer

When reading from a file (not stdin), a summary line is appended after EOF:

```
───────────────────────────────────────────────────────────────────────────────
2 errors · 44 warnings · 8 overfull · 1 underfull
```

Error count is red if nonzero, green if zero. This footer is **not printed**
when reading from stdin, since in continuous mode it would appear mid-stream
and become stale.

---

## Architecture: async streaming pipeline

```
AsyncRead (stdin or tokio::fs::File)
  └─ tokio::io::BufReader
      └─ FramedRead<LinesCodec>     lines arrive as they are written; EOF closes stream
          └─ LineJoiner              stateful: rejoins 79-char-wrapped fragments
              └─ LogParser           stateful: drives file stack + message detector
                  └─ Event stream    Message | PassBoundary | Eof
                      └─ OutputRenderer   writes colored lines to stdout
```

The pipeline is fully streaming: output is emitted incrementally, not after full
buffering. For stdin (the primary case), each line from pdflatex can in principle
be processed and rendered immediately. EOF on stdin (latexmk quits) terminates
the stream.

Both stdin and file inputs use the same `FramedRead<LinesCodec>` mechanism. No
polling, `inotify`, or file-watching is required.

---

## Module structure

```
src/
  lib.rs                        public re-exports

  bin/
    main.rs                     CLI entry point: clap arg parsing, async runtime,
                                input dispatch, drives the pipeline

  parser/
    mod.rs                      LogParser: top-level coordinator; owns file stack;
                                emits Events
    line_joiner.rs              Reassemble 79-char-wrapped physical lines into
                                logical lines
    file_stack.rs               Character-level scan; maintains open/close stack
    message.rs                  Pattern-match logical lines → LogMessage

  model/
    mod.rs
    entry.rs                    LogMessage, MessageKind, Event, PassKind

  output/
    mod.rs
    colored.rs                  Terminal renderer (colored crate)
```

---

## Data model (`src/model/entry.rs`)

```rust
pub enum PassKind {
    Pdflatex,
    Bibtex,
    Other(String),
}

pub enum MessageKind {
    LatexError,
    PackageWarning { package: String },
    PackageError   { package: String },
    OverfullHbox   { pt: f32 },
    UnderfullHbox  { badness: u32 },
    OverfullVbox   { pt: f32 },
    MissingChar,
    BibtexWarning,
    ShowOutput     { command: String },  // \show / \showthe / \showbox output
}

pub struct LogMessage {
    pub kind:          MessageKind,
    pub text:          String,
    pub file:          String,             // innermost file at time of message
    pub line_range:    Option<(u32, u32)>, // (start, end); equal for single-line refs
    pub page:          Option<u32>,
    pub context:       Vec<String>,         // <…> blocks, joined with continuation; errors only
    pub hint:          Option<String>,     // prose hint after l.N; errors only
}

pub enum Event {
    Message(LogMessage),
    PassBoundary(PassKind),
    PdfBuilt { path: String },
}
```

`PassBoundary` events are emitted when the parser detects a latexmk pass header.
The renderer uses them to print the separator line. No messages are deduplicated
or suppressed — all events are always emitted.

---

## Key parsing details

### 79-character line wrapping

TeX hard-wraps all output at column 79, splitting mid-word and mid-path. This
also applies to LuaLaTeX logs, though LuaTeX's own Lua-originated output
(module-loading banners, `pdf backend:` messages) wraps one character later,
at 80 - both widths show up as genuine wraps in the same real-world log, so
the joiner treats both as wrap points. Examples:

```
(/usr/local/texlive/2025/texmf-dist/tex/generic/pgf/utilities/pgfutil-common.te
x)
```
```
Package natbib Warning: Citation `compcert' on page 1 undefined on input line 8
.
```
```
warning  (pdf backend): ignoring duplicate destination with the name 'equation.4
.1'
```

**LineJoiner algorithm**: maintain a `pending: Option<String>` buffer.

1. If `pending` holds a line and the new line does *not* start with a fresh-line
   marker, append the new line to `pending` and continue buffering.
2. If the new line starts a fresh line, emit `pending`, then process the new
   line normally (possibly setting it as the new `pending` if it is also 79 or
   80 chars).
3. After processing, if the current logical line is exactly 79 or 80 chars,
   move it into `pending`; otherwise emit it immediately.

**Fresh-line markers** (prefixes that unambiguously start a new logical line):
empty line, `Package `, `LaTeX `, `Class `, `Overfull `, `Underfull `, `! `,
`FiXme `, `Warning--`, `(`, `)`, `[`, `]`, `l.` (error context),
`Run number`, `Latexmk:`, `Running '`, `------------`.

### File stack tracking (`src/parser/file_stack.rs`)

TeX records file opens and closes as `(path` / `)` pairs embedded anywhere in
the log stream, including mid-line and nested.

**What counts as a file-open**: `(` is a file-open if the text immediately
following it looks like a file path. The rule:

> A token starting with `./` or `/` is always a path. A token starting with a
> letter or digit is a path if it contains a `.` somewhere in the token (the
> extension). A token that is just `(` alone, or whose content contains a space
> before a `.`, is not a path.

This correctly handles:
- `(./intro.tex` → path (`./` prefix)
- `(/usr/local/texlive/.../article.cls` → path (`/` prefix)
- `(article.cls` → path (letter prefix, contains `.`)
- `(build/main.aux)` → path (letter prefix, contains `.`)
- `(type1/urw/uhvb8a.pfb)` → path (letter prefix, contains `.`)
- `(output active)` → NOT a path (`output` has no `.` before a space)
- `(see manual)` → NOT a path

All files are tracked regardless of whether they are system packages or user
files (no path filtering at this stage; filtering can be added later via a flag
if needed).

**Algorithm** (applied character-by-character to each logical line):
- On `(`: peek ahead, apply the path rule above. If matched, push the path to
  the stack.
- On `)`: pop the top of the stack. Track nesting depth to handle consecutive
  closes like `)))`.

The current stack top is the file attributed to any message parsed on that line.

### Message detection (`src/parser/message.rs`)

Applied to logical lines (after joining). First match wins.

| Prefix | Emits | Extracted fields |
|---|---|---|
| `! Package <X> Error: ` | `PackageError { package: X }` | text; read ahead for `l.N` + hint |
| `! Class <X> Error: ` | `PackageError { package: X }` | text; read ahead for `l.N` + hint |
| `! ` (other, incl. `! LaTeX Error:`) | `LatexError` | text; read ahead for `l.N` + hint |
| `<file>:<N>: ` (GCC-style) | `LatexError` | file; line N; text; read ahead for context + hint (same state machine as `!` errors) |
| `Package <X> Warning: ` | `PackageWarning { package: X }` | text; `on input line N`; `on page N` |
| `Class <X> Warning: ` | `PackageWarning { package: X }` | text; `on input line N`; `on page N` |
| `<X> Warning: ` (catch-all) | `PackageWarning { package: X }` | text; `on input line N`; `on page N` |
| `[<engine> ]warning (<tag>): ` | `PackageWarning { package: X }` | engine-level warnings; engine prefix is optional (e.g. `pdfTeX warning (ext4): ...` vs `warning  (pdf backend): ...`); package rendered as `"<engine> (<tag>)"` or `"<tag>"` when engine is absent; multi-line continuation applies |
| `Missing character:` | `MissingChar` | full text |
| `Overfull \hbox (<f>pt too wide)` | `OverfullHbox { pt }` | `pt`; `at lines N--M` or `output active`; `[][]` character art lines are discarded |
| `Underfull \hbox (badness <n>)` | `UnderfullHbox { badness }` | `badness`; `at lines N--M` or `output active`; `[][]` character art lines are discarded |
| `Overfull \vbox (<f>pt too high)` | `OverfullVbox { pt }` | `pt` |
| `Warning--` | `BibtexWarning` | text; `--line N of file <path>` |
| `> ` (line-start) | `ShowOutput { command }` | definition/value dump from `\show`/`\showthe`/`\showbox`; read ahead collecting `> `-prefixed lines until `l.N \show<cmd>`; `command` extracted from the `\show` call on the `l.N` line; first `> ` line (stripped of `> ` prefix) becomes `text`; remaining `> ` lines (stripped) go into `context` |

**Explicitly ignored** (consumed silently, never emitted):
- `Package <X> Info:` and `Class <X> Info:` — informational messages, too
  noisy for the default output.
- `LaTeX Font Info:`, `luaotfload |`, `Lua module:`, `Inserting '...' in` —
  engine/font loader noise specific to LuaLaTeX.
- Latexmk wrapper lines: `Latexmk:`, `Rule '...':`, `Running '...`,
  `------------`, `====`.

**Error read-ahead state machine**: after matching a `!` line, the parser enters
a multi-phase read-ahead mode to extract the line number and hint:

```
ErrorPhase::ContextLines
  Collect every line that appears between `!` and `l.N` as a context entry,
  regardless of whether it is `<…>`-tagged or untagged:
    - `<…>` tagged line (e.g. <inserted text>, <to be read again>, <read *>):
      consume the next indented continuation; join by stripping the
      continuation's leading whitespace and appending it to the tag
      (e.g. "<inserted text> $", "<to be read again> \tex_par:D").
    - Untagged line (e.g. bare `\foobar` or `\macro ->\foobar `): consume the
      next indented continuation the same way; store as a context entry.
  In both cases, if the continuation is whitespace-only, store the first line
  alone.
  On l.N → extract N as source line number; store snippet (text after "l.N ",
    `l.N ` prefix stripped); transition to SourceContinuation.
  On any recognised event prefix → emit error (with collected context entries
    if any) without snippet/hint, resume.

ErrorPhase::SourceContinuation
  Consume the indented continuation line (the second half of the TeX snippet);
    append to snippet buffer.
  On blank line → transition to HintWaiting.
  On recognised event prefix → emit error (with snippet if non-blank), resume.

ErrorPhase::HintWaiting
  A blank line was seen after the snippet.
  On non-blank line that is NOT a recognised event prefix → transition to
    HintLines, add this line as first hint line.
  On recognised event prefix or another blank line → emit error (snippet if
    non-blank, no hint), resume normal.

ErrorPhase::HintLines
  Accumulate lines into hint buffer until blank line or recognised event prefix.
  On terminator → emit error with non-blank snippet (if any) and hint, resume.
```

When rendering, output in this order:
1. Each `<…>` context entry as `│  <tag> <continuation>` (always shown).
2. Snippet lines as `│  <snippet_line1>` / `│  <snippet_line2>`, only when
   either contains non-whitespace; skipped entirely if both are blank.
3. A blank `│` separator (always shown when a hint follows).
4. Hint lines as `│  <line>`.

"Recognised event prefix" means any of: `!`, `Overfull`, `Underfull`,
`LaTeX Warning`, `Package`, `Class`, `Missing character`, `(`, `[`, `]`, `l.`,
`Warning--`, `FiXme`.

GCC-style errors (`file:N: message`) extract file, line, and message from the
single line, then enter the same read-ahead state machine as `!` errors
(ContextLines → SourceContinuation → HintWaiting → HintLines) in case context
or hint lines follow.

**Multi-line warnings**: `LaTeX Warning:` and `Package X Warning:` messages
sometimes continue across physical lines (especially after 79-char joining).
Accumulate continuation lines until a blank line or a new message prefix is
encountered.

### Pass detection

Latexmk emits lines like `Run number N of rule 'pdflatex'` before each tool
invocation. The parser matches these lines and emits a `PassBoundary` event,
then discards the line (it does not reach the message detector or file stack).

Patterns:
- `Run number \d+ of rule 'pdflatex'` → `PassKind::Pdflatex`
- `Run number \d+ of rule 'bibtex .*'` → `PassKind::Bibtex`
- `Run number \d+ of rule '(.*)'` → `PassKind::Other(name)`

`Output written on <path> (...)` → `PdfBuilt { path }`. Emitted regardless of
whether errors occurred in the same pass. The renderer prints a short green
confirmation line, e.g. `✔ PDF written: build/main.pdf` (Unicode) or
`* PDF written: build/main.pdf` (ASCII).

No deduplication of any kind is performed. Every message is emitted as-is,
regardless of whether it appeared in a previous pass.

---

## Output format (`src/output/colored.rs`)

### Pass separator

A single line filling the terminal width:

```
── pdflatex ────────────────────────────────────────────────────────────────────
```

Printed before every pass, including the first. Terminal width is queried
via the `terminal_size` crate; fallback is 80 columns. The pass kind label is
inserted after `── `.

### Per-file block

Files appear in the order their first message is encountered. Nested includes
are indented 2 spaces per nesting level relative to their parent, with no
additional glyph — the indentation alone signals the relationship.

```
./intro.tex
  ⚠ Package natbib: Citation `compcert' undefined  (line 8, page 1)
  » Overfull \hbox 13.30pt too wide  (output active)

  ./PingPongs.tla
    » Overfull \hbox 53.33pt too wide  (lines 2–3)
```

**Color scheme** (same in both Unicode and ASCII modes):

| Element | Color |
|---|---|
| File path | Green |
| Nested file path | Green (same as top-level); indented by 2 spaces per nesting level |
| Error glyph (`✕` / `x`) | Bright red, bold |
| Warning glyph (`⚠` / `!`) | Yellow |
| Overfull glyph (`»` / `>`) | Magenta |
| Underfull glyph (`«` / `<`) | Magenta |
| `Package` / `Class` qualifier | Bright black (dimmed) |
| Package / class name | Bold |
| `│` / `|` hint continuation prefix | Bright red (matches error) |
| Show output glyph (`⊢` / `?`) | Blue |
| Location `(line N, page N)` | Bright black (dark gray) |
| Pass separator | Bright black (always, including first pass) |

**Hint display**: hint lines are printed after a blank `│` separator, keeping
the visual connection while giving the hint its own breathing room. TeX's own
help text is hardcoded to break across physical lines (often mid-sentence)
for historical width reasons rather than to mark a meaningful structural
break, so the hint is reflowed into one prose block before display - this
also matches how the error/warning body text itself never preserves the raw
log's line breaks:

```
  ✕ Missing $ inserted  (line 1145)
  │
  │ I've inserted a begin-math/end-math symbol since I think you left one out.
  │ Proceed, with fingers crossed.
```

The blank `│` line and all hint lines use the same color as the error glyph.
In `--ascii` mode, `│` is replaced with `|` throughout.

**Wrapping**: the message body, context, and hint are all word-wrapped to
`--width` columns (see below), since raw log lines - especially hint prose
and long package-warning text - regularly overflow a normal terminal.
Continuation lines are re-indented to align under where the text starts (past
the glyph and any `Package foo: ` label for the body; past the `│`/`|` prefix
for context and hint), so wrapped output keeps the same left margin as the
rest of the render rather than losing it to the terminal's own raw wrap.
A single token longer than the available width is hard-broken rather than
overflowing. Lines that already fit are left byte-for-byte untouched, so a
short context line's meaningful internal spacing (e.g. a snippet's
indentation lining up a caret under a token) survives; only lines that
actually need wrapping have their whitespace normalized as part of
reflowing.

### Summary footer (file input only)

Printed once when the input stream ends, **only when reading from a file**:

```
───────────────────────────────────────────────────────────────────────────────
2 errors · 44 warnings · 8 overfull · 1 underfull
```

Error count is red if nonzero, green if zero. Not printed for stdin.

---

## Dependencies

Current `Cargo.toml`:
- `tokio` (full)
- `tokio-util` (codec, io)
- `tokio-stream`
- `colored`

To add:
- `clap` (derive feature) — CLI argument parsing; also handles `--ascii` as aliases
- `terminal_size` — query terminal width for separator lines

---

## Implementation order

1. `model/entry.rs` — data types
2. `parser/message.rs` — pattern matching (pure, unit-testable)
3. `parser/line_joiner.rs` — 79-char rejoining (test against `test.log`)
4. `parser/file_stack.rs` — parenthesis tracking (hardest)
5. `parser/mod.rs` — coordinator: wires the three above, emits `Event` stream
6. `output/colored.rs` — renderer
7. `bin/main.rs` — CLI + async glue

---

## Open questions / risks

- **Parenthesis tracking** is inherently heuristic. Real-world edge cases will
  require tuning against additional log samples, especially with unusual package
  output. Further test logs (particularly error cases and BibTeX errors) should
  be tested as they become available.
- **LuaLaTeX specifics**: `test2.log` shows LuaLaTeX noise (`luaotfload |`,
  `Lua module:`, `Inserting '...'`) that does not appear in pdflatex logs. These
  lines are silently ignored. LuaLaTeX otherwise produces the same `!`/`Warning:`
  structure as pdflatex for user-visible diagnostics.
- **BibTeX hard errors** (`I couldn't open style file...`, etc.) have their own
  format, distinct from pdflatex errors. To be specified when a BibTeX error log
  is available.
