//! Helpers for deriving plain text and slugs from the inline AST.
//!
//! These are used to:
//!  * produce human-readable section titles, and
//!  * generate stable identifiers (slugs) so that `[[file#Section]]`
//!    references can be matched against heading text in a CommonMark/GFM-ish
//!    way.

use nlc_parser::ast::Inline;

/// Flatten a slice of inline nodes into a single `String`, recursing into
/// emphasis/strong/link children. Whitespace-only structural nodes (breaks)
/// become a single space so the result stays readable.
pub fn inline_text(inlines: &[Inline]) -> String {
    let mut out = String::new();
    inline_into(inlines, &mut out);
    out
}

fn inline_into(inlines: &[Inline], out: &mut String) {
    for node in inlines {
        match node {
            Inline::Text(s) => out.push_str(s),
            Inline::Code(s) => out.push_str(s),
            Inline::Autolink(s) => out.push_str(s),
            Inline::Emphasis(inner) | Inline::Strong(inner) | Inline::Link { text: inner, .. } => {
                inline_into(inner, out);
            }
            Inline::Image { alt, .. } => out.push_str(alt),
            Inline::RawHtml(s) => out.push_str(s),
            Inline::HardBreak | Inline::SoftBreak => out.push(' '),
            Inline::FileRef(_) => {}
        }
    }
}

/// Convert arbitrary text to a GFM-style anchor slug.
///
/// Rules (deliberately simple and stable):
///  * lowercase (ASCII-aware),
///  * drop every character that is not a letter, digit, underscore, or space,
///  * collapse runs of whitespace into a single `-`,
///  * trim leading/trailing `-`.
pub fn slugify(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut prev_dash = true; // start in "trim" mode
    for c in text.chars() {
        if c.is_alphanumeric() || c == '_' {
            for lc in c.to_lowercase() {
                out.push(lc);
            }
            prev_dash = false;
        } else if c.is_whitespace() && !prev_dash {
            out.push('-');
            prev_dash = true;
        }
        // any other punctuation is dropped
    }
    while out.ends_with('-') {
        out.pop();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_basic() {
        assert_eq!(slugify("Hello, World!"), "hello-world");
        assert_eq!(slugify("Setup / Install"), "setup-install");
        assert_eq!(slugify("  leading & trailing  "), "leading-trailing");
        assert_eq!(slugify("C++ / Rust?"), "c-rust");
        assert_eq!(slugify("___"), "___");
        assert_eq!(slugify(""), "");
    }

    #[test]
    fn slug_unicode_lowercased() {
        assert_eq!(slugify("Éléphant"), "éléphant");
    }
}
