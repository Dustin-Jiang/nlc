//! Token types emitted by the block-level lexer and consumed by the
//! `parser_block` LALRPOP grammar.

/// Auxiliary data carried by block tokens with more than one field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeadingData {
    pub level: u8,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeData {
    pub info: String,
    pub code: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefData {
    pub label: String,
    pub destination: String,
    pub title: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListData {
    pub ordered: bool,
    pub start: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemData {
    pub task: Option<bool>,
}

/// Raw body of a YAML frontmatter block (the text between the opening `---`
/// and the closing `---` / `...` fence, excluding the fence lines themselves).
/// Only emitted when the document opens with a frontmatter fence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrontMatterData {
    pub body: String,
}

/// A token in the block-level token stream.
///
/// The stream is "flat" but carries explicit open/close markers for container
/// blocks (block quotes and lists), which lets the LALRPOP grammar build the
/// nested tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Token {
    // Leaf blocks (already fully lexed, including their raw inline text where
    // relevant; inline parsing happens afterwards over the carried string).
    Paragraph(String),
    Heading(HeadingData),
    CodeBlock(CodeData),
    ThematicBreak,
    HtmlBlock(String),
    ReferenceDef(RefData),
    FrontMatter(FrontMatterData),

    // Container blocks.
    BlockquoteOpen,
    BlockquoteClose,
    ListOpen(ListData),
    /// Carries the resolved `tight` flag of the list.
    ListClose(bool),
    ItemOpen(ItemData),
    ItemClose,

    // End sentinel.
    Eof,
}

pub type Spanned = (usize, Token, usize);
