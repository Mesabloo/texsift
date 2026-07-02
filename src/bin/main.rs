use std::io;
use std::path::PathBuf;

use clap::Parser;
use tokio::io::AsyncRead;
use tokio_stream::StreamExt;
use tokio_util::codec::{FramedRead, LinesCodec};

use texsift::output::{RenderOptions, Renderer};
use texsift::parser::LogParser;
use texsift::{Category, Event, MessageKind};

/// Filter and colorize a LaTeX/latexmk build log, scoped to the file each
/// diagnostic occurred in.
#[derive(Parser)]
#[command(name = "texsift")]
struct Cli {
    /// Log file to read; if omitted, reads from stdin
    file: Option<PathBuf>,

    /// Suppress warnings; bare flag suppresses all, or give a comma-separated
    /// package list (e.g. `--no-warn=pdf-backend,LaTeX`) to suppress only those
    #[arg(long, num_args = 0..=1, value_delimiter = ',', default_missing_value = "all", value_name = "PACKAGE,...")]
    no_warn: Vec<String>,

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

struct Filter {
    /// Bare `--no-warn` (no value): every diagnostic in `Category::Warning`
    /// is suppressed, regardless of package.
    no_warn_all: bool,
    /// `--no-warn=<package>` (repeatable): only `PackageWarning`s from
    /// these specific packages are suppressed. Matched against the
    /// package name as-is, or with hyphens read as spaces, so
    /// `pdf-backend` reaches the engine's `pdf backend` label without
    /// needing to quote a literal space on the command line.
    no_warn_packages: Vec<String>,
    no_boxes: bool,
}

impl Filter {
    fn new(no_warn: Vec<String>, no_boxes: bool) -> Self {
        let no_warn_all = no_warn.iter().any(|v| v == "all");
        let no_warn_packages = no_warn.into_iter().filter(|v| v != "all").collect();
        Self { no_warn_all, no_warn_packages, no_boxes }
    }

    fn should_show(&self, kind: &MessageKind) -> bool {
        match kind.category() {
            Category::Error => true,
            Category::OverfullBox | Category::UnderfullBox => !self.no_boxes,
            Category::Warning => {
                if self.no_warn_all {
                    return false;
                }
                if let MessageKind::PackageWarning { package } = kind {
                    let excluded = self.no_warn_packages.iter().any(|p| p == package || p.replace('-', " ") == *package);
                    if excluded {
                        return false;
                    }
                }
                true
            }
        }
    }
}

fn dispatch(event: Event, renderer: &mut Renderer<io::LineWriter<io::Stdout>>, filter: &Filter) {
    if let Event::Message(m) = &event {
        if !filter.should_show(&m.kind) {
            return;
        }
    }
    renderer.handle(event);
}

async fn drive<R: AsyncRead + Unpin>(
    reader: R,
    parser: &mut LogParser,
    renderer: &mut Renderer<io::LineWriter<io::Stdout>>,
    filter: &Filter,
) -> io::Result<()> {
    let mut framed = FramedRead::new(reader, LinesCodec::new());
    while let Some(line) = framed.next().await {
        let line = line.map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        for event in parser.feed(&line) {
            dispatch(event, renderer, filter);
        }
    }
    for event in parser.finish() {
        dispatch(event, renderer, filter);
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

    let filter = Filter::new(cli.no_warn, cli.no_boxes);
    let mut parser = LogParser::new();
    let mut renderer = Renderer::new(io::LineWriter::new(io::stdout()), opts);

    if let Some(path) = &cli.file {
        let file = tokio::fs::File::open(path).await?;
        drive(file, &mut parser, &mut renderer, &filter).await?;
    } else {
        drive(tokio::io::stdin(), &mut parser, &mut renderer, &filter).await?;
    }

    renderer.finish();
    if is_file {
        renderer.render_summary();
    }
    renderer.flush()?;

    Ok(())
}
