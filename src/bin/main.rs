use std::io;
use std::path::PathBuf;

use clap::Parser;
use tokio::io::AsyncRead;
use tokio_stream::StreamExt;
use tokio_util::codec::{FramedRead, LinesCodec};

use texsift::output::{RenderOptions, Renderer};
use texsift::parser::LogParser;
use texsift::{Event, MessageKind};

/// Filter and colorize a LaTeX/latexmk build log, scoped to the file each
/// diagnostic occurred in.
#[derive(Parser)]
#[command(name = "texsift")]
struct Cli {
    /// Log file to read; if omitted, reads from stdin
    file: Option<PathBuf>,

    /// Suppress warnings; show only errors and box diagnostics
    #[arg(long)]
    no_warn: bool,

    /// Suppress all Overfull/Underfull box diagnostics
    #[arg(long)]
    no_boxes: bool,

    /// Disable all terminal colors
    #[arg(long)]
    no_color: bool,

    /// Use ASCII fallback symbols instead of Unicode glyphs
    #[arg(long, alias = "no-unicode")]
    ascii: bool,

    /// Override terminal width used for message wrapping and pass
    /// separators (default: auto-detected, fallback 80; 0 also means
    /// auto-detect)
    #[arg(long)]
    width: Option<usize>,
}

fn should_show(kind: &MessageKind, no_warn: bool, no_boxes: bool) -> bool {
    use MessageKind::*;
    match kind {
        LatexError | PackageError { .. } => true,
        OverfullHbox { .. } | UnderfullHbox { .. } | OverfullVbox { .. } => !no_boxes,
        PackageWarning { .. } | BibtexWarning | MissingChar | ShowOutput { .. } => !no_warn,
    }
}

fn dispatch(event: Event, renderer: &mut Renderer<io::LineWriter<io::Stdout>>, no_warn: bool, no_boxes: bool) {
    if let Event::Message(m) = &event {
        if !should_show(&m.kind, no_warn, no_boxes) {
            return;
        }
    }
    renderer.handle(event);
}

async fn drive<R: AsyncRead + Unpin>(
    reader: R,
    parser: &mut LogParser,
    renderer: &mut Renderer<io::LineWriter<io::Stdout>>,
    no_warn: bool,
    no_boxes: bool,
) -> io::Result<()> {
    let mut framed = FramedRead::new(reader, LinesCodec::new());
    while let Some(line) = framed.next().await {
        let line = line.map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        for event in parser.feed(&line) {
            dispatch(event, renderer, no_warn, no_boxes);
        }
    }
    for event in parser.finish() {
        dispatch(event, renderer, no_warn, no_boxes);
    }
    Ok(())
}

#[tokio::main]
async fn main() -> io::Result<()> {
    let cli = Cli::parse();
    let is_file = cli.file.is_some();

    // `--width=0` is treated the same as omitting the flag: an explicit
    // "auto-detect" request rather than a literal zero-width terminal.
    let width = cli
        .width
        .filter(|&w| w != 0)
        .unwrap_or_else(|| terminal_size::terminal_size().map(|(w, _)| w.0 as usize).unwrap_or(80));
    let opts = RenderOptions { ascii: cli.ascii, color: !cli.no_color, width };

    let mut parser = LogParser::new();
    let mut renderer = Renderer::new(io::LineWriter::new(io::stdout()), opts);

    if let Some(path) = &cli.file {
        let file = tokio::fs::File::open(path).await?;
        drive(file, &mut parser, &mut renderer, cli.no_warn, cli.no_boxes).await?;
    } else {
        drive(tokio::io::stdin(), &mut parser, &mut renderer, cli.no_warn, cli.no_boxes).await?;
    }

    renderer.finish();
    if is_file {
        renderer.render_summary();
    }
    renderer.flush()?;

    Ok(())
}
