//! Abstract Syntax Tree for a parsed Markdown document.

/// A complete Markdown document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Document {
    /// Top-level block nodes, in source order.
    pub blocks: Vec<Block>,
    /// Link reference definitions collected during parsing. These are used
    /// to resolve `[text][label]` / `[label]` style links inside the text.
    pub references: Vec<Reference>,
    /// Raw YAML frontmatter body when the document opens with a `---` fence,
    /// without the surrounding fence lines. `None` when no frontmatter is
    /// present. The body is left unparsed; callers decide which YAML library
    /// (if any) to apply.
    pub frontmatter: Option<String>,
}

impl Document {
    pub fn new(blocks: Vec<Block>) -> Self {
        Self {
            blocks,
            references: Vec::new(),
            frontmatter: None,
        }
    }

    /// Look up a reference definition by (normalized) label.
    pub fn lookup(&self, label: &str) -> Option<&Reference> {
        let norm = normalize_label(label);
        self.references
            .iter()
            .find(|r| normalize_label(&r.label) == norm)
    }
}

/// Normalize a label for case-insensitive, whitespace-collapsing comparison,
/// matching the CommonMark procedure.
pub fn normalize_label(label: &str) -> String {
    let mut out = String::new();
    let mut prev_space = false;
    for c in label.trim().chars() {
        if c.is_whitespace() {
            if !prev_space {
                out.push(' ');
                prev_space = true;
            }
        } else {
            out.extend(c.to_lowercase());
            prev_space = false;
        }
    }
    out
}

/// A link reference definition: `[label]: destination "Optional title"`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reference {
    pub label: String,
    pub destination: String,
    pub title: Option<String>,
}

/// A block-level node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    Paragraph(Vec<Inline>),
    Heading {
        level: u8,
        inlines: Vec<Inline>,
    },
    /// A fenced (``` / ~~~) or indented code block. `info` is the info string
    /// (language hint) for fenced blocks; empty for indented blocks.
    CodeBlock {
        info: String,
        code: String,
    },
    ThematicBreak,
    BlockQuote(Vec<Block>),
    List {
        items: Vec<ListItem>,
        ordered: bool,
        start: u32,
        /// A "tight" list renders items without paragraph wrapping / spacing.
        tight: bool,
    },
    HtmlBlock(String),
}

/// A single list item, containing nested block content and an optional
/// GFM task-list checkbox marker.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ListItem {
    pub blocks: Vec<Block>,
    /// `Some(true)` = checked `[x]`, `Some(false)` = unchecked `[ ]`,
    /// `None` = no task marker.
    pub task: Option<bool>,
}

/// An inline-level node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Inline {
    /// A run of literal text.
    Text(String),
    /// `*emphasis*` / `_emphasis_`.
    Emphasis(Vec<Inline>),
    /// `**strong**` / `__strong__`.
    Strong(Vec<Inline>),
    /// `` `code` ``.
    Code(String),
    Link {
        text: Vec<Inline>,
        destination: String,
        title: Option<String>,
    },
    Image {
        alt: String,
        destination: String,
        title: Option<String>,
    },
    /// `<http://example.com>` / `<user@example.com>`.
    Autolink(String),
    /// Inline raw HTML such as `<span>`.
    RawHtml(String),
    /// A hard line break.
    HardBreak,
    /// A soft line break (a newline within a paragraph).
    SoftBreak,
    /// A cross-file wikilink: `[[file.md]]`, `[[file.md#Section]]`,
    /// `[[file.md#L42]]`, `[[#Section]]`, etc. with an optional `|alias`.
    FileRef(FileRef),
}

/// The target of a [`FileRef`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileRefTarget {
    /// The whole document.
    Document,
    /// A section, located by its heading text (a resolver normalizes to a slug).
    Section(String),
    /// A single source line.
    Line(u32),
    /// An inclusive range of source lines.
    LineRange(u32, u32),
}

/// A parsed `[[...]]` cross-file reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileRef {
    /// Target file path; `None` means the current document.
    pub path: Option<String>,
    /// What inside the file is being referenced.
    pub target: FileRefTarget,
    /// Optional display text (plain text for now).
    pub alias: Option<String>,
}
