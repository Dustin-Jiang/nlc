//! Inline lexer.
//!
//! Inline Markdown (`*emphasis*`, `[links](url)`, `` `code` ``, ...) is not
//! context-free, so this module performs all of the messy, stateful analysis:
//!
//! * backslash escapes
//! * code spans (matching backtick runs)
//! * links / images (bracket matching, inline + reference destinations)
//! * autolinks / inline raw HTML
//! * hard / soft line breaks
//! * emphasis & strong emphasis, using CommonMark's flanking + delimiter-stack
//!   algorithm
//!
//! It first scans the raw text into a flat list of [`N`] nodes (with delimiter
//! runs left unresolved), runs [`process_emphasis`] to build a real `Vec<Inline>`
//! tree, and finally [`linearize`]s that tree into a balanced token stream of
//! `EmphOpen`/`EmphClose`/... markers. That token stream is what the
//! `parser_inline` LALRPOP grammar consumes to rebuild (and validate) the tree.

use crate::ast::{FileRefTarget, Inline};
use crate::inline::token::{FileRefData, ImageData, LinkData, Spanned, Token};

use std::rc::Rc;

/// A first-pass node. `Delim` runs are resolved later by `process_emphasis`;
/// `Done` holds an already-built inline subtree (used after wrapping).
#[derive(Clone)]
enum N {
    Text(String),
    Code(String),
    Link(LinkData),
    Image(ImageData),
    FileRef(FileRefData),
    Autolink(String),
    Html(String),
    HardBreak,
    SoftBreak,
    Delim(Delim),
    Done(Inline),
}

#[derive(Clone)]
struct Delim {
    c: char,
    count: usize,
    can_open: bool,
    can_close: bool,
}

// ---------------------------------------------------------------------------

pub fn lex(text: &str) -> Vec<Spanned> {
    let inlines = parse_to_inlines(text);
    let mut out = Vec::new();
    let mut loc = 0usize;
    linearize(&inlines, &mut out, &mut loc);
    out
}

fn parse_to_inlines(text: &str) -> Vec<Inline> {
    let nodes = scan(text);
    coalesce(process_emphasis(nodes))
}

// ----------------------------- scanning ----------------------------------

fn scan(text: &str) -> Vec<N> {
    let bytes = text.as_bytes();
    let n = text.len();
    let mut i = 0usize;
    let mut buf = String::new();
    let mut nodes: Vec<N> = Vec::new();

    macro_rules! flush {
        () => {{
            if !buf.is_empty() {
                nodes.push(N::Text(std::mem::take(&mut buf)));
            }
        }};
    }

    while i < n {
        let b = bytes[i];
        match b {
            b'\\' => {
                // Backslash + newline => hard line break.
                if i + 1 < n && bytes[i + 1] == b'\n' {
                    flush!();
                    nodes.push(N::HardBreak);
                    i += 2;
                    continue;
                }
                // Backslash + ASCII punctuation => literal of that punctuation.
                if let Some(c) = cur_char(text, i + 1)
                    && is_ascii_punct(c) {
                        buf.push(c);
                        i += 1 + c.len_utf8();
                        continue;
                    }
                buf.push('\\');
                i += 1;
            }
            b'`' => {
                let (open, j) = count_run(text, i, b'`');
                // Find a closing run of equal length.
                let mut k = j;
                let mut found = None;
                while k < n {
                    if bytes[k] == b'`' {
                        let (c2, kk) = count_run(text, k, b'`');
                        if c2 == open {
                            found = Some((k, kk));
                            break;
                        }
                        k = kk;
                    } else {
                        k += 1;
                    }
                }
                match found {
                    Some((s, e)) => {
                        flush!();
                        let content = strip_code_spaces(&text[j..s]);
                        nodes.push(N::Code(content.to_string()));
                        i = e;
                    }
                    None => {
                        buf.push_str(&text[i..j]);
                        i = j;
                    }
                }
            }
            b'!' if i + 1 < n && bytes[i + 1] == b'[' => {
                if let Some((len, node)) = try_link(text, i + 1, true) {
                    flush!();
                    nodes.push(node);
                    i += 1 + len; // skip '!' + the link portion
                } else {
                    buf.push('!');
                    i += 1;
                }
            }
            b'[' if i + 1 < n && bytes[i + 1] == b'[' => {
                // Wikilink: [[ ... ]]
                if let Some((len, node)) = try_file_ref(text, i) {
                    flush!();
                    nodes.push(node);
                    i += len;
                } else {
                    buf.push('[');
                    i += 1;
                }
            }
            b'[' => {
                if let Some((len, node)) = try_link(text, i, false) {
                    flush!();
                    nodes.push(node);
                    i += len;
                } else {
                    buf.push('[');
                    i += 1;
                }
            }
            b'<' => {
                let mut j = i + 1;
                while j < n {
                    if bytes[j] == b'\\' {
                        j += 1;
                    }
                    if bytes[j] == b'>' {
                        break;
                    }
                    j += 1;
                }
                if j < n {
                    let content = &text[i + 1..j];
                    if let Some(url) = autolink_url(content) {
                        flush!();
                        nodes.push(N::Autolink(url));
                        i = j + 1;
                        continue;
                    }
                    if is_html_tag(content) {
                        flush!();
                        nodes.push(N::Html(format!("<{}>", content)));
                        i = j + 1;
                        continue;
                    }
                }
                buf.push('<');
                i += 1;
            }
            b'\n' => {
                let trailing = buf.bytes().rev().take_while(|&c| c == b' ').count();
                buf.truncate(buf.len() - trailing);
                flush!();
                if trailing >= 2 {
                    nodes.push(N::HardBreak);
                } else {
                    nodes.push(N::SoftBreak);
                }
                i += 1;
            }
            b'*' | b'_' => {
                let (count, j) = count_run(text, i, b);
                let prev = if i == 0 { None } else { prev_char(text, i) };
                let next = cur_char(text, j);
                let (can_open, can_close) = classify(b as char, prev, next);
                flush!();
                nodes.push(N::Delim(Delim {
                    c: b as char,
                    count,
                    can_open,
                    can_close,
                }));
                i = j;
            }
            b'&' => {
                // Pass HTML entities through verbatim as text.
                buf.push('&');
                i += 1;
            }
            _ => {
                let c = cur_char(text, i).unwrap();
                buf.push(c);
                i += c.len_utf8();
            }
        }
    }
    flush!();
    nodes
}

/// Try to parse a link/image starting at `start` (the index of `[`).
/// Returns `(consumed_bytes_from_start, node)` on success.
fn try_link(text: &str, start: usize, is_image: bool) -> Option<(usize, N)> {
    let bytes = text.as_bytes();
    let n = text.len();
    // Find the matching `]`, balancing nested brackets and honouring escapes.
    let mut depth = 1usize;
    let mut j = start + 1;
    while j < n {
        match bytes[j] {
            b'\\' => j += 2,
            b'[' => {
                depth += 1;
                j += 1;
            }
            b']' => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
                j += 1;
            }
            _ => j += 1,
        }
    }
    if j >= n {
        return None;
    }
    let inner = &text[start + 1..j];
    let after = j + 1;

    if after < n && bytes[after] == b'(' {
        let (dest, title, end) = parse_inline_dest(text, after)?;
        return Some((end - start, make_link_node(inner, dest, title, is_image)));
    }

    if after < n && bytes[after] == b'[' {
        // Full reference: `[text][label]`.
        let lab_start = after + 1;
        let mut k = lab_start;
        while k < n {
            if bytes[k] == b'\\' {
                k += 2;
            } else if bytes[k] == b']' {
                break;
            } else {
                k += 1;
            }
        }
        if k >= n {
            return None;
        }
        let raw_label = &text[lab_start..k];
        let label = if raw_label.trim().is_empty() { inner } else { raw_label };
        if let Some((dest, title)) = crate::inline::lookup_ref(label) {
            return Some(((k + 1) - start, make_link_node(inner, dest, title, is_image)));
        }
        return None;
    }

    // Collapsed / shortcut reference: `[text]` or `[text][]`.
    if let Some((dest, title)) = crate::inline::lookup_ref(inner) {
        return Some(((j + 1) - start, make_link_node(inner, dest, title, is_image)));
    }
    None
}

/// Parse an inline link destination: `(...)` where `p` points at `(`.
fn parse_inline_dest(text: &str, p: usize) -> Option<(String, Option<String>, usize)> {
    let bytes = text.as_bytes();
    let n = text.len();
    let mut k = p + 1;
    k = skip_ws(text, k);
    if k >= n {
        return None;
    }
    let dest;
    if bytes[k] == b'<' {
        let mut j = k + 1;
        while j < n {
            if bytes[j] == b'\\' {
                j += 1;
            }
            if bytes[j] == b'>' {
                break;
            }
            j += 1;
        }
        if j >= n {
            return None;
        }
        dest = unescape(&text[k + 1..j]);
        k = j + 1;
    } else {
        let mut j = k;
        let mut depth = 0i32;
        while j < n {
            let c = bytes[j];
            if c == b'\\' {
                j += 2;
                continue;
            }
            if c == b' ' || c == b'\t' || c == b'\n' || c == b'\r' {
                break;
            }
            if c == b'(' {
                depth += 1;
            } else if c == b')' {
                if depth == 0 {
                    break;
                }
                depth -= 1;
            }
            j += 1;
        }
        dest = unescape(&text[k..j]);
        k = j;
    }

    let mut k2 = skip_ws(text, k);
    let title;
    if k2 < n && (bytes[k2] == b'"' || bytes[k2] == b'\'' || bytes[k2] == b'(') {
        let close = match bytes[k2] {
            b'"' => b'"',
            b'\'' => b'\'',
            _ => b')',
        };
        let mut j = k2 + 1;
        while j < n {
            if bytes[j] == b'\\' {
                j += 2;
                continue;
            }
            if bytes[j] == close {
                break;
            }
            j += 1;
        }
        if j >= n {
            return None;
        }
        title = Some(unescape(&text[k2 + 1..j]));
        k2 = j + 1;
    } else {
        title = None;
    }
    let k2 = skip_ws(text, k2);
    if k2 < n && bytes[k2] == b')' {
        Some((dest, title, k2 + 1))
    } else {
        None
    }
}

fn make_link_node(inner: &str, dest: String, title: Option<String>, is_image: bool) -> N {
    if is_image {
        N::Image(ImageData {
            alt: plain_alt(inner),
            destination: dest,
            title,
        })
    } else {
        N::Link(LinkData {
            text: parse_to_inlines(inner),
            destination: dest,
            title,
        })
    }
}

/// Try to parse a `[[...]]` wikilink starting at `start` (the first `[`).
/// Returns `(consumed_bytes_from_start, node)` on success, or `None` if there
/// is no closing `]]` or the body is empty/invalid (in which case the caller
/// falls back to a literal `[`).
fn try_file_ref(text: &str, start: usize) -> Option<(usize, N)> {
    let bytes = text.as_bytes();
    let n = text.len();
    // body starts after `[[`
    let body_start = start + 2;
    // scan to the first `]]` (no nesting)
    let mut j = body_start;
    while j + 1 < n {
        if bytes[j] == b']' && bytes[j + 1] == b']' {
            break;
        }
        j += 1;
    }
    // need at least one `]` followed by `]`
    if j + 1 >= n || bytes[j] != b']' || bytes[j + 1] != b']' {
        return None;
    }
    let body = &text[body_start..j];
    let data = parse_file_ref(body)?;
    Some(((j + 2) - start, N::FileRef(data)))
}

/// Parse a wikilink body into structured data. Returns `None` for an empty
/// body (which would be an invalid reference).
///
/// Grammar: `[path][ '#' locator ]][ '|' alias ]`
/// where `path` defaults to the current document when empty, and `locator` is
/// either a heading text, `L<line>`, or `L<a>-L<b>` / `L<a>-<b>`.
fn parse_file_ref(body: &str) -> Option<FileRefData> {
    let body = body.trim();
    if body.is_empty() {
        return None;
    }

    // Split off an optional alias on the first '|'.
    let (body, alias) = match body.find('|') {
        Some(idx) => (body[..idx].trim_end(), Some(body[idx + 1..].trim().to_string())),
        None => (body, None),
    };
    let alias = alias.filter(|a| !a.is_empty());

    // Split off the locator on the first '#'.
    let (path_part, locator_part) = match body.find('#') {
        Some(idx) => (&body[..idx], Some(&body[idx + 1..])),
        None => (body, None),
    };

    let path = {
        let p = path_part.trim();
        if p.is_empty() {
            None
        } else {
            Some(p.to_string())
        }
    };

    let target = match locator_part {
        None => FileRefTarget::Document,
        Some(loc) => {
            let loc = loc.trim();
            if loc.is_empty() {
                FileRefTarget::Document
            } else if let Some((a, b)) = parse_line_range(loc) {
                match b {
                    Some(end) => FileRefTarget::LineRange(a, end),
                    None => FileRefTarget::Line(a),
                }
            } else {
                FileRefTarget::Section(loc.to_string())
            }
        }
    };

    Some(FileRefData {
        path,
        target,
        alias,
    })
}

/// Parse `L<n>` or `L<a>-L<b>` / `L<a>-<b>` (1-indexed). Returns
/// `(start, Some(end))` for a range, `(start, None)` for a single line, or
/// `None` if the input is not a line locator.
fn parse_line_range(s: &str) -> Option<(u32, Option<u32>)> {
    let rest = s.strip_prefix('L').or_else(|| s.strip_prefix('l'))?;
    if let Some((a, b)) = rest.split_once('-') {
        let start: u32 = a.parse().ok()?;
        let b = b.strip_prefix('L').or_else(|| b.strip_prefix('l')).unwrap_or(b);
        let end: u32 = b.parse().ok()?;
        if end < start {
            return None;
        }
        Some((start, Some(end)))
    } else {
        let v: u32 = rest.parse().ok()?;
        Some((v, None))
    }
}

// ------------------------- emphasis algorithm ----------------------------

fn process_emphasis(mut items: Vec<N>) -> Vec<Inline> {
    loop {
        let delim_idxs: Vec<usize> = items
            .iter()
            .enumerate()
            .filter_map(|(i, n)| match n {
                N::Delim(_) => Some(i),
                _ => None,
            })
            .collect();

        let mut found = None;
        'outer: for &closer_i in &delim_idxs {
            let cc = match &items[closer_i] {
                N::Delim(d) => d.clone(),
                _ => continue,
            };
            if !cc.can_close {
                continue;
            }
            for &opener_i in delim_idxs.iter().rev().filter(|&&idx| idx < closer_i) {
                let od = match &items[opener_i] {
                    N::Delim(d) => d.clone(),
                    _ => continue,
                };
                if od.can_open && od.c == cc.c {
                    found = Some((opener_i, closer_i, od, cc));
                    break 'outer;
                }
            }
        }

        let (opener_i, closer_i, opener, closer) = match found {
            Some(x) => x,
            None => break,
        };

        let strong = opener.count >= 2 && closer.count >= 2;
        let length = if strong { 2 } else { 1 };

        let inner_nodes: Vec<N> = items.drain(opener_i + 1..closer_i).collect();
        let inner = process_emphasis(inner_nodes);
        let node = if strong {
            Inline::Strong(inner)
        } else {
            Inline::Emphasis(inner)
        };

        // After the drain, the closer now sits at `opener_i + 1`.
        let closer_now = opener_i + 1;
        if let N::Delim(d) = &mut items[opener_i] {
            d.count -= length;
        }
        if let N::Delim(d) = &mut items[closer_now] {
            d.count -= length;
        }
        // Insert the wrapped node between opener and closer.
        items.insert(opener_i + 1, N::Done(node));

        let opener_zero = matches!(&items[opener_i], N::Delim(d) if d.count == 0);
        let closer_zero = matches!(&items[opener_i + 2], N::Delim(d) if d.count == 0);
        if opener_zero {
            items.remove(opener_i);
            if closer_zero {
                items.remove(opener_i + 1);
            }
        } else if closer_zero {
            items.remove(opener_i + 2);
        }
    }

    items.into_iter().map(n_to_inline).collect()
}

/// Merge runs of adjacent `Inline::Text` nodes (recursing into emphasis/strong)
/// so the AST stays tidy, e.g. after unresolved delimiter runs become literal.
fn coalesce(inlines: Vec<Inline>) -> Vec<Inline> {
    let mut out: Vec<Inline> = Vec::with_capacity(inlines.len());
    for n in inlines {
        let n = match n {
            Inline::Emphasis(v) => Inline::Emphasis(coalesce(v)),
            Inline::Strong(v) => Inline::Strong(coalesce(v)),
            other => other,
        };
        match (n, out.last_mut()) {
            (Inline::Text(s), Some(Inline::Text(prev))) => prev.push_str(&s),
            (n, _) => out.push(n),
        }
    }
    out
}

fn n_to_inline(n: N) -> Inline {
    match n {
        N::Text(s) => Inline::Text(s),
        N::Code(s) => Inline::Code(s),
        N::Link(d) => Inline::Link {
            text: d.text,
            destination: d.destination,
            title: d.title,
        },
        N::Image(d) => Inline::Image {
            alt: d.alt,
            destination: d.destination,
            title: d.title,
        },
        N::FileRef(d) => Inline::FileRef(crate::ast::FileRef {
            path: d.path,
            target: d.target,
            alias: d.alias,
        }),
        N::Autolink(s) => Inline::Autolink(s),
        N::Html(s) => Inline::RawHtml(s),
        N::HardBreak => Inline::HardBreak,
        N::SoftBreak => Inline::SoftBreak,
        N::Delim(d) => Inline::Text(d.c.to_string().repeat(d.count)),
        N::Done(i) => i,
    }
}

// ----------------------------- linearize ---------------------------------

fn linearize(inlines: &[Inline], out: &mut Vec<Spanned>, loc: &mut usize) {
    let push = |tok: Token, out: &mut Vec<_>, loc: &mut usize| {
        out.push((*loc, tok, *loc + 1));
        *loc += 1;
    };
    for n in inlines {
        match n {
            Inline::Emphasis(v) => {
                push(Token::EmphOpen, out, loc);
                linearize(v, out, loc);
                push(Token::EmphClose, out, loc);
            }
            Inline::Strong(v) => {
                push(Token::StrongOpen, out, loc);
                linearize(v, out, loc);
                push(Token::StrongClose, out, loc);
            }
            Inline::Text(s) => push(Token::Text(s.clone()), out, loc),
            Inline::Code(s) => push(Token::Code(s.clone()), out, loc),
            Inline::Link {
                text,
                destination,
                title,
            } => push(
                Token::Link(LinkData {
                    text: text.clone(),
                    destination: destination.clone(),
                    title: title.clone(),
                }),
                out,
                loc,
            ),
            Inline::Image {
                alt,
                destination,
                title,
            } => push(
                Token::Image(ImageData {
                    alt: alt.clone(),
                    destination: destination.clone(),
                    title: title.clone(),
                }),
                out,
                loc,
            ),
            Inline::Autolink(s) => push(Token::Autolink(s.clone()), out, loc),
            Inline::RawHtml(s) => push(Token::RawHtml(s.clone()), out, loc),
            Inline::HardBreak => push(Token::HardBreak, out, loc),
            Inline::SoftBreak => push(Token::SoftBreak, out, loc),
            Inline::FileRef(f) => push(
                Token::FileRef(FileRefData {
                    path: f.path.clone(),
                    target: f.target.clone(),
                    alias: f.alias.clone(),
                }),
                out,
                loc,
            ),
        }
    }
}

// ------------------------------ helpers ----------------------------------

fn cur_char(s: &str, i: usize) -> Option<char> {
    s.get(i..)?.chars().next()
}

fn prev_char(s: &str, i: usize) -> Option<char> {
    s.get(..i)?.chars().next_back()
}

fn count_run(text: &str, i: usize, c: u8) -> (usize, usize) {
    let bytes = text.as_bytes();
    let mut n = 0usize;
    let mut j = i;
    while j < bytes.len() && bytes[j] == c {
        n += 1;
        j += 1;
    }
    (n, j)
}

fn skip_ws(text: &str, mut k: usize) -> usize {
    let bytes = text.as_bytes();
    while k < bytes.len() && matches!(bytes[k], b' ' | b'\t' | b'\n' | b'\r') {
        k += 1;
    }
    k
}

fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 1 < bytes.len()
            && let Some(c) = cur_char(s, i + 1)
                && is_ascii_punct(c) {
                    out.push(c);
                    i += 1 + c.len_utf8();
                    continue;
                }
        let c = cur_char(s, i).unwrap();
        out.push(c);
        i += c.len_utf8();
    }
    out
}

fn plain_alt(s: &str) -> String {
    // For image alt text, drop emphasis/code markers and unescape.
    let mut out = String::new();
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' if i + 1 < bytes.len() => {
                if let Some(c) = cur_char(s, i + 1)
                    && is_ascii_punct(c) {
                        out.push(c);
                        i += 1 + c.len_utf8();
                        continue;
                    }
                out.push('\\');
                i += 1;
            }
            b'*' | b'_' | b'~' => i += 1,
            b'`' => i += 1,
            _ => {
                let c = cur_char(s, i).unwrap();
                out.push(c);
                i += c.len_utf8();
            }
        }
    }
    out
}

fn strip_code_spaces(s: &str) -> &str {
    let bytes = s.as_bytes();
    if bytes.len() >= 2 && bytes[0] == b' ' && bytes[bytes.len() - 1] == b' ' {
        // Only strip if the content is not all spaces.
        if !bytes.iter().all(|&c| c == b' ') {
            return &s[1..s.len() - 1];
        }
    }
    s
}

fn autolink_url(c: &str) -> Option<String> {
    if c.is_empty() || c.bytes().any(|b| matches!(b, b' ' | b'\t' | b'\n' | b'\r')) {
        return None;
    }
    if is_email(c) {
        return Some(format!("mailto:{}", c));
    }
    if let Some(colon) = c.find(':') {
        let scheme = &c[..colon];
        let mut chars = scheme.chars();
        let first = chars.next()?;
        if !first.is_ascii_alphabetic() {
            return None;
        }
        if !scheme
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '+' || ch == '-' || ch == '.')
        {
            return None;
        }
        return Some(c.to_string());
    }
    None
}

fn is_email(c: &str) -> bool {
    // Very small email heuristic: has exactly one '@' with non-empty local
    // and domain parts, and the domain contains a dot.
    let at = c.matches('@').count();
    if at != 1 {
        return false;
    }
    let (local, domain) = c.split_once('@').unwrap();
    !local.is_empty() && domain.contains('.') && !domain.starts_with('.') && !domain.ends_with('.')
}

fn is_html_tag(c: &str) -> bool {
    let first = match c.chars().next() {
        Some(ch) => ch,
        None => return false,
    };
    first.is_ascii_alphabetic() || first == '/' || first == '!' || first == '?'
}

fn is_ascii_punct(c: char) -> bool {
    matches!(
        c,
        '!' | '"'
            | '#'
            | '$'
            | '%'
            | '&'
            | '\''
            | '('
            | ')'
            | '*'
            | '+'
            | ','
            | '-'
            | '.'
            | '/'
            | ':'
            | ';'
            | '<'
            | '='
            | '>'
            | '?'
            | '@'
            | '['
            | '\\'
            | ']'
            | '^'
            | '_'
            | '`'
            | '{'
            | '|'
            | '}'
            | '~'
    )
}

fn is_ws(c: char) -> bool {
    c == ' ' || c == '\t' || c == '\n' || c == '\r' || c == '\x0c' || c == '\x0b'
}

fn is_punct(c: char) -> bool {
    !c.is_alphanumeric() && !is_ws(c)
}

/// CommonMark flanking classification. Returns `(can_open, can_close)`.
///
/// Matches cmark's `scan_delims`: at the start/end of the input the missing
/// neighbour is treated as a linefeed (whitespace); for `*` we use
/// `can_open = left_flanking`, `can_close = right_flanking`, while `_` adds
/// the extra punctuation clause to prevent intra-word emphasis.
fn classify(c: char, prev: Option<char>, next: Option<char>) -> (bool, bool) {
    let before = prev.unwrap_or('\n');
    let after = next.unwrap_or('\n');

    let before_ws = is_ws(before);
    let before_punct = is_punct(before);
    let after_ws = is_ws(after);
    let after_punct = is_punct(after);

    let left_flanking = !after_ws && (!after_punct || before_ws || before_punct);
    let right_flanking = !before_ws && (!before_punct || after_ws || after_punct);

    if c == '_' {
        (
            left_flanking && (!right_flanking || before_punct),
            right_flanking && (!left_flanking || after_punct),
        )
    } else {
        (left_flanking, right_flanking)
    }
}

// Suppress an unused-import warning when `Rc` is not otherwise referenced via
// this module (it is re-exported through `inline`).
#[allow(dead_code)]
type _UnusedRc = Rc<()>;
