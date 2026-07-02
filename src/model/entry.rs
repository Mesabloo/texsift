#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PassKind {
    Pdflatex,
    Bibtex,
    Other(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum MessageKind {
    LatexError,
    PackageWarning { package: String },
    PackageError { package: String },
    OverfullHbox { pt: f32 },
    UnderfullHbox { badness: u32 },
    OverfullVbox { pt: f32 },
    MissingChar,
    BibtexWarning,
    ShowOutput { command: String },
}

/// The broad class a [`MessageKind`] falls into, for callers that only care
/// about the error/overfull/underfull/warning grouping (e.g. CLI filtering,
/// summary tallying) rather than the specific variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    Error,
    OverfullBox,
    UnderfullBox,
    Warning,
}

impl MessageKind {
    pub fn category(&self) -> Category {
        match self {
            MessageKind::LatexError | MessageKind::PackageError { .. } => Category::Error,
            MessageKind::OverfullHbox { .. } | MessageKind::OverfullVbox { .. } => Category::OverfullBox,
            MessageKind::UnderfullHbox { .. } => Category::UnderfullBox,
            MessageKind::PackageWarning { .. } | MessageKind::BibtexWarning | MessageKind::MissingChar | MessageKind::ShowOutput { .. } => {
                Category::Warning
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct LogMessage {
    pub kind: MessageKind,
    pub text: String,
    pub file: String,
    /// The chain of real file-open ancestors enclosing `file`, outermost
    /// first (e.g. `["./main.tex", "./compiler.tex"]` for a file opened from
    /// within `compiler.tex`). Empty for the top-level document, and for
    /// messages that carry their own file (GCC-style errors, BibTeX
    /// warnings) rather than inheriting one from the file-open stack.
    ///
    /// This is a chain rather than a plain depth count so the renderer can
    /// collapse "invisible" wrapper files - ones with no messages of their
    /// own - out of the displayed indentation.
    pub ancestors: Vec<String>,
    pub line_range: Option<(u32, u32)>,
    pub page: Option<u32>,
    pub context: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    Message(LogMessage),
    PassBoundary(PassKind),
    PdfBuilt { path: String },
}
