//! Block-level lexer.
//!
//! Markdown's block structure is line-oriented and somewhat context-sensitive
//! (indentation-based lists, container nesting, setext headings, ...). This
//! lexer performs that messy analysis and produces a flat token stream that
//! uses explicit `Open`/`Close` markers for container blocks (block quotes and
//! lists). The `parser_block` LALRPOP grammar then consumes that stream to
//! build the nested [`Block`] tree.
//!
//! Inline content is *not* parsed here: leaf blocks carry their raw inline
//! text as a `String`, and the inline parser runs afterwards.

use crate::block::token::{
    CodeData, HeadingData, ItemData, ListData, RefData, Spanned, Token,
};

/// A frame on the container stack tracked while lexing.
#[derive(Clone, Debug)]
enum Frame {
    Bq,
    List {
        ordered: bool,
        marker_col: usize,
        tight: bool,
    },
    Item {
        /// Absolute column (byte index into the line) where item content begins.
        content_col: usize,
    },
}

/// A leaf block currently being accumulated across lines.
#[derive(Debug)]
enum Leaf {
    Para(Vec<String>),
    Indented(Vec<String>),
}

pub struct Lexer {
    lines: Vec<String>,
    i: usize,
    out: Vec<Spanned>,
    loc: usize,
    frames: Vec<Frame>,
    leaf: Option<Leaf>,
    /// True if the previous line was blank. Used for tight/loose list detection.
    saw_blank: bool,
}

impl Lexer {
    fn new(input: &str) -> Self {
        // Expand tabs to 4 spaces up front so column arithmetic is simple.
        let input = input.replace('\t', "    ");
        let mut lines: Vec<String> = input.split('\n').map(|s| s.to_string()).collect();
        // Drop a single trailing empty line produced by a final '\n'.
        if lines.last().map(|s| s.is_empty()).unwrap_or(false) {
            lines.pop();
        }
        Lexer {
            lines,
            i: 0,
            out: Vec::new(),
            loc: 0,
            frames: Vec::new(),
            leaf: None,
            saw_blank: false,
        }
    }

    fn emit(&mut self, t: Token) {
        self.out.push((self.loc, t, self.loc + 1));
        self.loc += 1;
    }

    fn flush(&mut self) {
        if let Some(leaf) = self.leaf.take() {
            match leaf {
                Leaf::Para(lines) => {
                    let text = lines.join("\n");
                    self.emit(Token::Paragraph(text));
                }
                Leaf::Indented(lines) => {
                    let code = strip_trailing_blank_lines(&lines).join("\n");
                    self.emit(Token::CodeBlock(CodeData {
                        info: String::new(),
                        code,
                    }));
                }
            }
        }
    }

    fn close_top(&mut self) {
        match self.frames.pop().unwrap() {
            Frame::Bq => self.emit(Token::BlockquoteClose),
            Frame::List { tight, .. } => self.emit(Token::ListClose(tight)),
            Frame::Item { .. } => self.emit(Token::ItemClose),
        }
    }

    fn close_to(&mut self, target: usize) {
        if target < self.frames.len() {
            self.flush();
        }
        while self.frames.len() > target {
            self.close_top();
        }
    }

    /// Mark the innermost open list as loose.
    fn loosen_innermost_list(&mut self) {
        for f in self.frames.iter_mut().rev() {
            if let Frame::List { tight, .. } = f {
                *tight = false;
                break;
            }
        }
    }

    fn run(input: &str) -> Vec<Spanned> {
        let mut lx = Lexer::new(input);
        while lx.i < lx.lines.len() {
            let line = lx.lines[lx.i].clone();
            lx.i += 1;

            if line.trim().is_empty() {
                lx.handle_blank();
                continue;
            }
            lx.handle_line(&line);
        }
        lx.flush();
        while !lx.frames.is_empty() {
            lx.close_top();
        }
        lx.emit(Token::Eof);
        lx.out
    }

    fn handle_blank(&mut self) {
        match &mut self.leaf {
            Some(Leaf::Indented(v)) => v.push(String::new()),
            Some(Leaf::Para(_)) => {
                self.flush();
                self.saw_blank = true;
            }
            None => self.saw_blank = true,
        }
    }

    fn handle_line(&mut self, line: &str) {
        // ---- 1. Strip container continuations, closing dead containers. ----
        let mut pos = 0usize;
        let mut k = 0usize;
        let close_target;
        loop {
            if k >= self.frames.len() {
                close_target = self.frames.len();
                break;
            }
            match &self.frames[k] {
                Frame::Bq => {
                    if let Some(adv) = strip_one_bq(&line[pos..]) {
                        pos += adv;
                        k += 1;
                    } else {
                        close_target = k;
                        break;
                    }
                }
                Frame::List { .. } => {
                    k += 1;
                }
                Frame::Item { content_col } => {
                    let required = content_col.saturating_sub(pos);
                    let ls = leading_spaces(line, pos);
                    if ls >= required {
                        pos += required;
                        k += 1;
                    } else {
                        // Maybe a sibling list item at the parent list's column.
                        let mut sibling = false;
                        if k >= 1
                            && let Frame::List { marker_col, ordered, .. } = &self.frames[k - 1]
                                && pos == *marker_col
                                    && let Some(m) = list_marker(line, pos)
                                        && m.ordered == *ordered {
                                            sibling = true;
                                        }
                        close_target = if sibling { k } else { k.saturating_sub(1) };
                        break;
                    }
                }
            }
        }
        self.close_to(close_target);

        // ---- 2. Open new containers (block quotes / lists). ----
        loop {
            let rest = &line[pos..];

            // Block quote.
            if rest.starts_with('>') {
                self.flush();
                let adv = strip_one_bq(rest).unwrap();
                self.emit(Token::BlockquoteOpen);
                self.frames.push(Frame::Bq);
                pos += adv;
                continue;
            }

            // List item (but not if this line is really a thematic break).
            if !is_thematic_break(rest)
                && let Some(m) = list_marker(line, pos) {
                    let para_open = matches!(self.leaf, Some(Leaf::Para(_)));
                    let interrupts = !(para_open && m.ordered && m.start != 1);
                    if interrupts {
                        let marker_col = pos;
                        let reuse = matches!(
                            self.frames.last(),
                            Some(Frame::List { marker_col: mc, ordered, .. }) if *mc == marker_col && *ordered == m.ordered
                        );
                        if reuse {
                            if self.saw_blank {
                                self.loosen_innermost_list();
                            }
                        } else {
                            self.flush();
                            self.emit(Token::ListOpen(ListData {
                                ordered: m.ordered,
                                start: m.start,
                            }));
                            self.frames.push(Frame::List {
                                ordered: m.ordered,
                                marker_col,
                                tight: true,
                            });
                        }
                        self.emit(Token::ItemOpen(ItemData { task: m.task }));
                        self.frames.push(Frame::Item {
                            content_col: m.content_pos,
                        });
                        self.saw_blank = false;
                        pos = m.content_pos;
                        continue;
                    }
                }
            break;
        }

        // ---- 3. Classify the leaf content remaining at `pos`. ----
        let content = &line[pos..];
        if content.trim().is_empty() {
            // Line had only container markers (e.g. `>` or `-`). Flush, but do
            // not register a "blank" for tight/loose purposes.
            self.flush();
            return;
        }

        // Indented-code continuation takes priority.
        if let Some(Leaf::Indented(v)) = &mut self.leaf {
            let indent = leading_spaces(content, 0);
            if indent >= 4 {
                v.push(content[4..].to_string());
                return;
            } else {
                self.flush();
            }
        }

        let indent = leading_spaces(content, 0);
        let body = &content[indent..];

        // A new block starting after a blank line inside a list => loose.
        if self.saw_blank && matches!(self.frames.last(), Some(Frame::Item { .. })) {
            self.loosen_innermost_list();
        }

        // Setext heading underline (only interrupts a paragraph).
        if matches!(self.leaf, Some(Leaf::Para(_)))
            && let Some(level) = setext_level(body) {
                if let Some(Leaf::Para(lines)) = self.leaf.take() {
                    let text = lines.join("\n");
                    self.emit(Token::Heading(HeadingData { level, text }));
                }
                self.saw_blank = false;
                return;
            }

        if is_thematic_break(body) {
            self.flush();
            self.emit(Token::ThematicBreak);
            self.saw_blank = false;
            return;
        }

        if let Some((level, text)) = atx_heading(body) {
            self.flush();
            self.emit(Token::Heading(HeadingData { level, text }));
            self.saw_blank = false;
            return;
        }

        if let Some((fc, n, info)) = fence(body) {
            self.flush();
            let mut code_lines: Vec<String> = Vec::new();
            while self.i < self.lines.len() {
                let l = self.lines[self.i].clone();
                if is_close_fence(&l, fc, n) {
                    self.i += 1;
                    break;
                }
                self.i += 1;
                code_lines.push(l);
            }
            let stripped: Vec<String> = code_lines
                .iter()
                .map(|l| {
                    let s = leading_spaces(l, 0).min(3);
                    l[s..].to_string()
                })
                .collect();
            let code = strip_trailing_blank_lines(&stripped).join("\n");
            self.emit(Token::CodeBlock(CodeData { info, code }));
            self.saw_blank = false;
            return;
        }

        if is_html_block_start(body) {
            self.flush();
            let mut lines = vec![content.to_string()];
            while self.i < self.lines.len() {
                if self.lines[self.i].trim().is_empty() {
                    break;
                }
                let l = self.lines[self.i].clone();
                self.i += 1;
                lines.push(l);
            }
            self.emit(Token::HtmlBlock(lines.join("\n")));
            self.saw_blank = false;
            return;
        }

        if !matches!(self.leaf, Some(Leaf::Para(_)))
            && let Some(rd) = reference_def(body) {
                self.flush();
                self.emit(Token::ReferenceDef(rd));
                self.saw_blank = false;
                return;
            }

        if indent >= 4 && !matches!(self.leaf, Some(Leaf::Para(_))) {
            let entry = content[4..].to_string();
            match &mut self.leaf {
                Some(Leaf::Indented(v)) => v.push(entry),
                _ => {
                    self.flush();
                    self.leaf = Some(Leaf::Indented(vec![entry]));
                }
            }
            return;
        }

        // Default: paragraph text.
        match &mut self.leaf {
            Some(Leaf::Para(v)) => v.push(body.to_string()),
            _ => {
                self.flush();
                self.leaf = Some(Leaf::Para(vec![body.to_string()]));
            }
        }
        self.saw_blank = false;
    }
}

/// Lex a Markdown document into a block-level token stream.
pub fn lex(input: &str) -> Vec<Spanned> {
    Lexer::run(input)
}

// -------------------- helper functions --------------------

fn leading_spaces(s: &str, start: usize) -> usize {
    let b = s.as_bytes();
    let mut n = 0;
    let mut i = start;
    while i < b.len() && b[i] == b' ' {
        n += 1;
        i += 1;
    }
    n
}

fn strip_one_bq(rest: &str) -> Option<usize> {
    let b = rest.as_bytes();
    if b.is_empty() || b[0] != b'>' {
        return None;
    }
    let mut adv = 1;
    if adv < b.len() && b[adv] == b' ' {
        adv += 1;
    }
    Some(adv)
}

fn is_thematic_break(s: &str) -> bool {
    let t = s.trim();
    if t.is_empty() {
        return false;
    }
    let c = t.chars().next().unwrap();
    if c != '-' && c != '*' && c != '_' {
        return false;
    }
    let mut count = 0;
    for ch in t.chars() {
        if ch == c {
            count += 1;
        } else if !ch.is_whitespace() {
            return false;
        }
    }
    count >= 3
}

fn atx_heading(s: &str) -> Option<(u8, String)> {
    let b = s.as_bytes();
    let mut level = 0;
    while level < b.len() && b[level] == b'#' {
        level += 1;
    }
    if level == 0 || level > 6 {
        return None;
    }
    if level < b.len() && b[level] != b' ' {
        return None;
    }
    let rest = s[level..].trim();
    Some((level as u8, strip_trailing_hashes(rest).to_string()))
}

fn strip_trailing_hashes(s: &str) -> &str {
    let t = s.trim_end();
    let b = t.as_bytes();
    let mut i = b.len();
    while i > 0 && b[i - 1] == b'#' {
        i -= 1;
    }
    if i == 0 {
        ""
    } else if b[i - 1] == b' ' {
        t[..i - 1].trim_end()
    } else {
        t
    }
}

fn setext_level(s: &str) -> Option<u8> {
    let t = s.trim();
    if t.is_empty() {
        return None;
    }
    if t.chars().all(|c| c == '=') {
        return Some(1);
    }
    if t.chars().all(|c| c == '-') {
        return Some(2);
    }
    None
}

fn fence(s: &str) -> Option<(char, usize, String)> {
    let b = s.as_bytes();
    if b.is_empty() {
        return None;
    }
    let c = b[0];
    if c != b'`' && c != b'~' {
        return None;
    }
    let mut n = 0;
    while n < b.len() && b[n] == c {
        n += 1;
    }
    if n < 3 {
        return None;
    }
    let info = s[n..].trim().to_string();
    if c == b'`' && info.contains('`') {
        return None;
    }
    Some((c as char, n, info))
}

fn is_close_fence(line: &str, fc: char, n: usize) -> bool {
    let t = line.trim_start();
    let b = t.as_bytes();
    if b.is_empty() || b[0] != fc as u8 {
        return false;
    }
    let mut count = 0;
    while count < b.len() && b[count] == fc as u8 {
        count += 1;
    }
    count >= n && t[count..].trim().is_empty()
}

fn is_html_block_start(s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() < 2 || b[0] != b'<' {
        return false;
    }
    match b[1] {
        // Comments / declarations / CDATA / processing instructions.
        b'!' | b'?' => true,
        // Closing tag: </tagname ...>
        b'/' => b.len() >= 3 && b[2].is_ascii_alphabetic(),
        // Open tag: <tagname ...> — the char after the tag name must terminate it.
        c if c.is_ascii_alphabetic() => {
            let mut i = 1;
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'-') {
                i += 1;
            }
            // End of line, or whitespace / '>' / '/': a plausible tag.
            i >= b.len() || matches!(b[i], b' ' | b'\t' | b'>' | b'/')
        }
        _ => false,
    }
}

pub struct Marker {
    pub ordered: bool,
    pub start: u32,
    pub content_pos: usize,
    pub task: Option<bool>,
}

fn list_marker(line: &str, pos: usize) -> Option<Marker> {
    let b = line.as_bytes();
    if pos >= b.len() {
        return None;
    }
    let (ordered, start, mlen) = match b[pos] {
        b'-' | b'+' | b'*' => (false, 1u32, 1usize),
        d if d.is_ascii_digit() => {
            let mut i = pos;
            while i < b.len() && b[i].is_ascii_digit() {
                i += 1;
            }
            let num: u32 = line[pos..i].parse().ok()?;
            if num == 0 || num > 999_999_999 {
                return None;
            }
            if i >= b.len() {
                return None;
            }
            let delim = b[i];
            if delim != b'.' && delim != b')' {
                return None;
            }
            (true, num, i - pos + 1)
        }
        _ => return None,
    };
    let after = pos + mlen;
    if after < b.len() && b[after] != b' ' {
        return None;
    }
    let mut content_pos = (after + 1).min(line.len());
    let mut task = None;
    let rest = &line[content_pos..];
    if rest.starts_with("[ ] ") || rest == "[ ]" {
        task = Some(false);
        content_pos = (content_pos + 4).min(line.len());
    } else if rest.starts_with("[x] ") || rest.starts_with("[X] ") || rest == "[x]" || rest == "[X]" {
        task = Some(true);
        content_pos = (content_pos + 4).min(line.len());
    }
    Some(Marker {
        ordered,
        start,
        content_pos,
        task,
    })
}

fn reference_def(s: &str) -> Option<RefData> {
    let t = s.trim();
    if !t.starts_with('[') {
        return None;
    }
    let close = t.find(']')?;
    if close < 2 {
        return None;
    }
    let label = t[1..close].to_string();
    let rest = t[close + 1..].trim_start();
    let rest = rest.strip_prefix(':')?.trim_start();
    let (destination, rest) = if let Some(inner) = rest.strip_prefix('<') {
        let end = inner.find('>')?;
        (inner[..end].to_string(), inner[end + 1..].trim_start())
    } else {
        let end = rest.find(|c: char| c.is_whitespace()).unwrap_or(rest.len());
        if end == 0 {
            return None;
        }
        (rest[..end].to_string(), rest[end..].trim_start())
    };
    let title = parse_title(rest);
    Some(RefData {
        label,
        destination,
        title,
    })
}

fn parse_title(s: &str) -> Option<String> {
    let s = s.trim_end();
    if s.is_empty() {
        return None;
    }
    let close = match s.as_bytes()[0] {
        b'"' => b'"',
        b'\'' => b'\'',
        b'(' => b')',
        _ => return None,
    };
    if s.as_bytes().last() != Some(&close) || s.len() < 2 {
        return None;
    }
    Some(s[1..s.len() - 1].to_string())
}

fn strip_trailing_blank_lines(lines: &[String]) -> Vec<String> {
    let mut end = lines.len();
    while end > 0 && lines[end - 1].trim().is_empty() {
        end -= 1;
    }
    if end == 0 {
        Vec::new()
    } else {
        lines[..end].to_vec()
    }
}
