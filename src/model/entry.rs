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
