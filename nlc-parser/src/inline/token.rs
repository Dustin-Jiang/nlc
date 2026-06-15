//! Token types emitted by the inline lexer and consumed by the
//! `parser_inline` LALRPOP grammar.

/// Fully resolved link data. The inner `text` has already been parsed into
/// inline nodes by the lexer (links/images are not context-free, so the lexer
/// handles bracket matching and recursion).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkData {
    pub text: Vec<crate::ast::Inline>,
    pub destination: String,
    pub title: Option<String>,
}

/// Fully resolved image data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageData {
    pub alt: String,
    pub destination: String,
    pub title: Option<String>,
}

/// Fully resolved `[[...]]` cross-file reference data (mirrors [`FileRef`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileRefData {
    pub path: Option<String>,
    pub target: crate::ast::FileRefTarget,
    pub alias: Option<String>,
}

/// A token in the inline token stream.
///
/// Emphasis/strong delimiter runs have already been paired up by the lexer
/// (using CommonMark's flanking algorithm) and are emitted as balanced
/// `*Open`/`*Close` markers, so the grammar only has to build the tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Token {
    Text(String),
    Code(String),
    Link(LinkData),
    Image(ImageData),
    FileRef(FileRefData),
    Autolink(String),
    RawHtml(String),
    HardBreak,
    SoftBreak,
    EmphOpen,
    EmphClose,
    StrongOpen,
    StrongClose,
}

pub type Spanned = (usize, Token, usize);
