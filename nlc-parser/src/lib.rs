//! `nlc` — a Markdown parser built with LALRPOP.
//!
//! Architecture:
//!
//! * [`block::lexer`] turns the raw text into a flat block-level token stream,
//!   using explicit `Open`/`Close` markers for container blocks.
//! * [`parser_block`] (a LALRPOP grammar) consumes that stream and builds the
//!   nested [`Document`] / [`Block`] tree.
//! * [`inline::lexer`] turns each leaf block's raw inline text into a balanced
//!   inline token stream (emphasis/strong already paired up).
//! * [`parser_inline`] (a LALRPOP grammar) rebuilds the `Vec<Inline>` tree.
//!
//! The two-phase lexer+grammar split is necessary because Markdown is not
//! context-free: the lexers do the context-sensitive work (indentation,
//! container nesting, emphasis flanking, bracket matching) and LALRPOP handles
//! the clean, CFG-shaped structure building on top.

pub mod ast;
pub mod block;
pub mod inline;
mod parser_block;
mod parser_inline;

pub use ast::*;

use std::collections::HashMap;
use std::rc::Rc;

/// Internal helper used by the block grammar to separate reference definitions
/// and frontmatter from regular blocks while collecting top-level parts.
pub(crate) enum Part {
    Block(Block),
    Ref(Reference),
    FrontMatter(String),
}

/// Parse a Markdown document into a [`Document`] AST.
///
/// Returns `Err` only if the LALRPOP block parser rejects the token stream —
/// which, since the lexer is designed to always emit a well-formed stream,
/// should not happen in practice and indicates a bug.
pub fn parse(input: &str) -> Result<Document, String> {
    let tokens = block::lexer::lex(input);
    let refs = collect_refs(&tokens);
    inline::with_refs(refs, || match parser_block::DocParser::new().parse(tokens) {
        Ok(d) => Ok(d),
        Err(e) => Err(format!("parse error: {:?}", e)),
    })
}

fn collect_refs(tokens: &[(usize, block::token::Token, usize)]) -> Rc<inline::RefMap> {
    let mut map: inline::RefMap = HashMap::new();
    for (_, t, _) in tokens {
        if let block::token::Token::ReferenceDef(rd) = t {
            map.insert(
                normalize_label(&rd.label),
                (rd.destination.clone(), rd.title.clone()),
            );
        }
    }
    Rc::new(map)
}
