//! Inline-level lexing, tokens, and the public inline entry point.

pub mod lexer;
pub mod token;

use crate::ast::Inline;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

/// Normalized link-label -> `(destination, title)`.
pub(crate) type RefMap = HashMap<String, (String, Option<String>)>;

thread_local! {
    static REFS: RefCell<Rc<RefMap>> = RefCell::new(Rc::new(HashMap::new()));
}

/// Install a reference table for the duration of `f`.
pub(crate) fn with_refs<T>(refs: Rc<RefMap>, f: impl FnOnce() -> T) -> T {
    let prev = REFS.with(|c| c.replace(refs));
    let res = f();
    REFS.with(|c| {
        c.replace(prev);
    });
    res
}

pub(crate) fn lookup_ref(label: &str) -> Option<(String, Option<String>)> {
    let norm = crate::ast::normalize_label(label);
    REFS.with(|c| c.borrow().get(&norm).cloned())
}

/// Parse a raw inline string into AST nodes via the `parser_inline` grammar.
pub fn parse(text: &str) -> Vec<Inline> {
    let tokens = lexer::lex(text);
    crate::parser_inline::InlinesParser::new()
        .parse(tokens)
        .unwrap_or_default()
}

/// Helpers used by the inline grammar actions.
pub fn link_to_inline(d: token::LinkData) -> Inline {
    Inline::Link {
        text: d.text,
        destination: d.destination,
        title: d.title,
    }
}

pub fn image_to_inline(d: token::ImageData) -> Inline {
    Inline::Image {
        alt: d.alt,
        destination: d.destination,
        title: d.title,
    }
}
