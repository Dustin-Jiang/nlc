//! End-to-end tests for the Markdown parser. Each test parses a snippet and
//! asserts against the expected AST.

use nlc_parser::ast::*;
use nlc_parser::parse;

fn text(s: &str) -> Inline {
    Inline::Text(s.to_string())
}

#[test]
fn atx_heading() {
    let doc = parse("### Hello *world*").unwrap();
    assert_eq!(
        doc.blocks,
        vec![Block::Heading {
            level: 3,
            inlines: vec![text("Hello "), Inline::Emphasis(vec![text("world")])],
        }]
    );
}

#[test]
fn setext_heading() {
    let doc = parse("Title\n=====\n\nSub\n---\n").unwrap();
    assert_eq!(
        doc.blocks,
        vec![
            Block::Heading {
                level: 1,
                inlines: vec![text("Title")],
            },
            Block::Heading {
                level: 2,
                inlines: vec![text("Sub")],
            },
        ]
    );
}

#[test]
fn paragraph_and_inline() {
    let doc = parse("a **b** `c` _d_ e").unwrap();
    assert_eq!(
        doc.blocks,
        vec![Block::Paragraph(vec![
            text("a "),
            Inline::Strong(vec![text("b")]),
            text(" "),
            Inline::Code("c".into()),
            text(" "),
            Inline::Emphasis(vec![text("d")]),
            text(" e"),
        ])]
    );
}

#[test]
fn nested_emphasis() {
    let doc = parse("**a *b* c**").unwrap();
    assert_eq!(
        doc.blocks,
        vec![Block::Paragraph(vec![Inline::Strong(vec![
            text("a "),
            Inline::Emphasis(vec![text("b")]),
            text(" c"),
        ])])]
    );
}

#[test]
fn intra_word_underscore_is_literal() {
    let doc = parse("foo_bar_baz").unwrap();
    assert_eq!(doc.blocks, vec![Block::Paragraph(vec![text("foo_bar_baz")])]);
}

#[test]
fn inline_link() {
    let doc = parse("[x](http://u \"t\")").unwrap();
    assert_eq!(
        doc.blocks,
        vec![Block::Paragraph(vec![Inline::Link {
            text: vec![text("x")],
            destination: "http://u".into(),
            title: Some("t".into()),
        }])]
    );
}

#[test]
fn reference_link_full_and_shortcut() {
    let doc = parse("[a]: u1\n\nSee [b][a] and [a].\n").unwrap();
    assert_eq!(
        doc.blocks,
        vec![Block::Paragraph(vec![
            text("See "),
            Inline::Link {
                text: vec![text("b")],
                destination: "u1".into(),
                title: None,
            },
            text(" and "),
            Inline::Link {
                text: vec![text("a")],
                destination: "u1".into(),
                title: None,
            },
            text("."),
        ])]
    );
    assert_eq!(doc.references.len(), 1);
}

#[test]
fn image() {
    let doc = parse("![alt](/i.png)").unwrap();
    assert_eq!(
        doc.blocks,
        vec![Block::Paragraph(vec![Inline::Image {
            alt: "alt".into(),
            destination: "/i.png".into(),
            title: None,
        }])]
    );
}

#[test]
fn autolink() {
    let doc = parse("<http://x.io>").unwrap();
    assert_eq!(
        doc.blocks,
        vec![Block::Paragraph(vec![Inline::Autolink("http://x.io".into())])]
    );
}

#[test]
fn thematic_break_variants() {
    for src in ["---\n", "***\n", "_ _ _\n", "  *  *  *\n"] {
        let doc = parse(src).unwrap();
        assert_eq!(doc.blocks, vec![Block::ThematicBreak], "failed for {src:?}");
    }
}

#[test]
fn fenced_code_with_info() {
    let doc = parse("```rust\nfn main() {}\n```\n").unwrap();
    assert_eq!(
        doc.blocks,
        vec![Block::CodeBlock {
            info: "rust".into(),
            code: "fn main() {}".into(),
        }]
    );
}

#[test]
fn fenced_code_tilde() {
    let doc = parse("~~~\nx\ny\n~~~\n").unwrap();
    assert_eq!(
        doc.blocks,
        vec![Block::CodeBlock {
            info: "".into(),
            code: "x\ny".into(),
        }]
    );
}

#[test]
fn indented_code() {
    let doc = parse("    code\n    here\n").unwrap();
    assert_eq!(
        doc.blocks,
        vec![Block::CodeBlock {
            info: "".into(),
            code: "code\nhere".into(),
        }]
    );
}

#[test]
fn blockquote() {
    let doc = parse("> hello\n> world\n").unwrap();
    assert_eq!(
        doc.blocks,
        vec![Block::BlockQuote(vec![Block::Paragraph(vec![
            text("hello"),
            Inline::SoftBreak,
            text("world"),
        ])])]
    );
}

#[test]
fn blockquote_with_heading() {
    let doc = parse("> # hi\n").unwrap();
    assert_eq!(
        doc.blocks,
        vec![Block::BlockQuote(vec![Block::Heading {
            level: 1,
            inlines: vec![text("hi")],
        }])]
    );
}

#[test]
fn unordered_list() {
    let doc = parse("- a\n- b\n- c\n").unwrap();
    let items: Vec<Vec<Block>> = vec![
        vec![Block::Paragraph(vec![text("a")])],
        vec![Block::Paragraph(vec![text("b")])],
        vec![Block::Paragraph(vec![text("c")])],
    ];
    let items = items
        .into_iter()
        .map(|blocks| ListItem {
            blocks,
            task: None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        doc.blocks,
        vec![Block::List {
            items,
            ordered: false,
            start: 1,
            tight: true,
        }]
    );
}

#[test]
fn ordered_list_start() {
    let doc = parse("3. third\n4. fourth\n").unwrap();
    let items: Vec<ListItem> = ["third", "fourth"]
        .iter()
        .map(|s| ListItem {
            blocks: vec![Block::Paragraph(vec![text(s)])],
            task: None,
        })
        .collect();
    assert_eq!(
        doc.blocks,
        vec![Block::List {
            items,
            ordered: true,
            start: 3,
            tight: true,
        }]
    );
}

#[test]
fn nested_list() {
    let doc = parse("- a\n  - b\n").unwrap();
    let inner = Block::List {
        items: vec![ListItem {
            blocks: vec![Block::Paragraph(vec![text("b")])],
            task: None,
        }],
        ordered: false,
        start: 1,
        tight: true,
    };
    assert_eq!(
        doc.blocks,
        vec![Block::List {
            items: vec![ListItem {
                blocks: vec![Block::Paragraph(vec![text("a")]), inner],
                task: None,
            }],
            ordered: false,
            start: 1,
            tight: true,
        }]
    );
}

#[test]
fn loose_list() {
    let doc = parse("- a\n\n- b\n").unwrap();
    match &doc.blocks[0] {
        Block::List { tight, .. } => assert!(!*tight),
        other => panic!("expected list, got {other:?}"),
    }
}

#[test]
fn task_list() {
    let doc = parse("- [x] done\n- [ ] todo\n").unwrap();
    if let Block::List { items, .. } = &doc.blocks[0] {
        assert_eq!(items[0].task, Some(true));
        assert_eq!(items[1].task, Some(false));
    } else {
        panic!("expected list");
    }
}

#[test]
fn hard_and_soft_break() {
    let doc = parse("line1  \nline2\nline3\n").unwrap();
    assert_eq!(
        doc.blocks,
        vec![Block::Paragraph(vec![
            text("line1"),
            Inline::HardBreak,
            text("line2"),
            Inline::SoftBreak,
            text("line3"),
        ])]
    );
}

#[test]
fn backslash_escape() {
    let doc = parse(r"\*not emph\*").unwrap();
    assert_eq!(
        doc.blocks,
        vec![Block::Paragraph(vec![text("*not emph*")])]
    );
}

#[test]
fn html_block_and_raw_inline() {
    let doc = parse("<div>\nhello\n</div>\n").unwrap();
    match &doc.blocks[0] {
        Block::HtmlBlock(h) => assert!(h.contains("<div>")),
        other => panic!("expected html block, got {other:?}"),
    }
}

#[test]
fn code_span_strips_one_space() {
    let doc = parse("`` foo ``").unwrap();
    assert_eq!(
        doc.blocks,
        vec![Block::Paragraph(vec![Inline::Code("foo".into())])]
    );
}

#[test]
fn empty_input() {
    let doc = parse("").unwrap();
    assert!(doc.blocks.is_empty());
    assert!(doc.references.is_empty());
    assert!(doc.frontmatter.is_none());
}

// ---- YAML frontmatter ----

#[test]
fn frontmatter_basic_dashes() {
    let src = "---\ntitle: Hello\ntags: [a, b]\n---\n\n# Body\n";
    let doc = parse(src).unwrap();
    assert_eq!(
        doc.frontmatter.as_deref(),
        Some("title: Hello\ntags: [a, b]")
    );
    // Body is unaffected: frontmatter never enters the block list.
    assert_eq!(
        doc.blocks,
        vec![Block::Heading {
            level: 1,
            inlines: vec![text("Body")],
        }]
    );
}

#[test]
fn frontmatter_dots_close() {
    // Pandoc-style `...` closing fence.
    let src = "---\nkey: val\n...\ntext\n";
    let doc = parse(src).unwrap();
    assert_eq!(doc.frontmatter.as_deref(), Some("key: val"));
    assert_eq!(
        doc.blocks,
        vec![Block::Paragraph(vec![text("text")])]
    );
}

#[test]
fn frontmatter_empty_body() {
    let doc = parse("---\n---\nrest\n").unwrap();
    assert_eq!(doc.frontmatter.as_deref(), Some(""));
    assert_eq!(
        doc.blocks,
        vec![Block::Paragraph(vec![text("rest")])]
    );
}

#[test]
fn frontmatter_no_trailing_newline() {
    let doc = parse("---\nx: 1\n---").unwrap();
    assert_eq!(doc.frontmatter.as_deref(), Some("x: 1"));
    assert!(doc.blocks.is_empty());
}

#[test]
fn frontmatter_tolerates_trailing_whitespace_on_fence() {
    let doc = parse("---   \nx: 1\n---   \nbody\n").unwrap();
    assert_eq!(doc.frontmatter.as_deref(), Some("x: 1"));
    assert_eq!(
        doc.blocks,
        vec![Block::Paragraph(vec![text("body")])]
    );
}

#[test]
fn frontmatter_must_be_first_line() {
    // Leading blank line disqualifies the opening fence.
    let doc = parse("\n---\nx: 1\n---\n").unwrap();
    assert!(doc.frontmatter.is_none());
}

#[test]
fn unterminated_frontmatter_is_not_frontmatter() {
    // No closing fence -> the leading `---` is just a thematic break.
    let doc = parse("---\nx: 1\n").unwrap();
    assert!(doc.frontmatter.is_none());
    assert_eq!(doc.blocks, vec![Block::ThematicBreak, Block::Paragraph(vec![text("x: 1")])]);
}

#[test]
fn frontmatter_with_reference_definitions_after() {
    let src = "---\nmeta: yes\n---\n\n[a]: http://u\n\nSee [a].\n";
    let doc = parse(src).unwrap();
    assert_eq!(doc.frontmatter.as_deref(), Some("meta: yes"));
    assert_eq!(doc.references.len(), 1);
    assert_eq!(doc.references[0].destination, "http://u");
    assert_eq!(
        doc.blocks,
        vec![Block::Paragraph(vec![
            text("See "),
            Inline::Link {
                text: vec![text("a")],
                destination: "http://u".into(),
                title: None,
            },
            text("."),
        ])]
    );
}

// ---- `[[...]]` cross-file references ----

fn fref(path: Option<&str>, target: FileRefTarget, alias: Option<&str>) -> Inline {
    Inline::FileRef(FileRef {
        path: path.map(str::to_string),
        target,
        alias: alias.map(str::to_string),
    })
}

#[test]
fn file_ref_whole_file() {
    let doc = parse("see [[guide.md]] now").unwrap();
    assert_eq!(
        doc.blocks,
        vec![Block::Paragraph(vec![
            text("see "),
            fref(Some("guide.md"), FileRefTarget::Document, None),
            text(" now"),
        ])]
    );
}

#[test]
fn file_ref_section() {
    let doc = parse("[[guide.md#Installation]]").unwrap();
    assert_eq!(
        doc.blocks,
        vec![Block::Paragraph(vec![fref(
            Some("guide.md"),
            FileRefTarget::Section("Installation".into()),
            None
        )])]
    );
}

#[test]
fn file_ref_single_line() {
    let doc = parse("[[a.rs#L42]]").unwrap();
    assert_eq!(
        doc.blocks,
        vec![Block::Paragraph(vec![fref(
            Some("a.rs"),
            FileRefTarget::Line(42),
            None
        )])]
    );
}

#[test]
fn file_ref_line_range() {
    let doc = parse("[[b.rs#L10-L20]]").unwrap();
    assert_eq!(
        doc.blocks,
        vec![Block::Paragraph(vec![fref(
            Some("b.rs"),
            FileRefTarget::LineRange(10, 20),
            None
        )])]
    );
}

#[test]
fn file_ref_current_file_section_and_line() {
    let doc = parse("[[#Intro]] and [[#L7]]").unwrap();
    assert_eq!(
        doc.blocks,
        vec![Block::Paragraph(vec![
            fref(None, FileRefTarget::Section("Intro".into()), None),
            text(" and "),
            fref(None, FileRefTarget::Line(7), None),
        ])]
    );
}

#[test]
fn file_ref_alias() {
    let doc = parse("[[c.md|the doc]]").unwrap();
    assert_eq!(
        doc.blocks,
        vec![Block::Paragraph(vec![fref(
            Some("c.md"),
            FileRefTarget::Document,
            Some("the doc")
        )])]
    );
}

#[test]
fn file_ref_with_alias_and_section() {
    let doc = parse("[[c.md#Setup|setup guide]]").unwrap();
    assert_eq!(
        doc.blocks,
        vec![Block::Paragraph(vec![fref(
            Some("c.md"),
            FileRefTarget::Section("Setup".into()),
            Some("setup guide")
        )])]
    );
}

#[test]
fn file_ref_does_not_break_normal_links() {
    let doc = parse("[[x.md]] and [text](http://u) and ![i](/p.png)").unwrap();
    let blocks = &doc.blocks;
    assert!(matches!(blocks[0], Block::Paragraph(_)));
    if let Block::Paragraph(v) = &blocks[0] {
        assert!(v.iter().any(|n| matches!(n, Inline::FileRef(_))));
        assert!(v.iter().any(|n| matches!(n, Inline::Link { .. })));
        assert!(v.iter().any(|n| matches!(n, Inline::Image { .. })));
    }
}

#[test]
fn file_ref_invalid_falls_back_to_literal() {
    // No closing ]]  ->  literal "[".
    let doc = parse("a [[b c").unwrap();
    assert_eq!(
        doc.blocks,
        vec![Block::Paragraph(vec![text("a [[b c")])]
    );
    // Empty body -> literal (no FileRef produced).
    let doc = parse("x [[ ]] y").unwrap();
    if let Block::Paragraph(v) = &doc.blocks[0] {
        assert!(v.iter().all(|n| matches!(n, Inline::Text(_))));
        assert!(v.iter().all(|n| !matches!(n, Inline::FileRef(_))));
    } else {
        panic!("expected paragraph");
    }
}

#[test]
fn file_ref_reversed_range_invalid_becomes_section() {
    // L20-L10 is invalid as a range -> treated as a section text.
    let doc = parse("[[x#L20-L10]]").unwrap();
    if let Block::Paragraph(v) = &doc.blocks[0] {
        match &v[0] {
            Inline::FileRef(FileRef { target, .. }) => {
                assert!(matches!(target, FileRefTarget::Section(_)), "{target:?}");
            }
            other => panic!("expected FileRef, got {other:?}"),
        }
    }
}
