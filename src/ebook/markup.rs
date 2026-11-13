//! A forgiving reader for the HTML and XML inside ebooks.
//!
//! Book markup is often sloppy — Kindle books are tag soup, not XML — so this
//! never fails: it tokenises whatever it's given and turns it into blocks of
//! styled text, ignoring what it doesn't understand.

use std::collections::HashMap;

use super::{Block, Run, Style};

#[derive(Debug, PartialEq)]
pub enum Token {
    Start { name: String, attrs: Vec<(String, String)>, closed: bool },
    End { name: String },
    Text(String),
}

/// Splits markup into tags and text. Comments, doctypes and processing
/// instructions are dropped; CDATA becomes text.
pub fn tokenize(src: &str) -> Vec<Token> {
    let mut out = Vec::new();
    let b = src.as_bytes();
    let mut i = 0;
    let mut text_start = 0;
    let flush = |out: &mut Vec<Token>, from: usize, to: usize| {
        if to > from {
            out.push(Token::Text(decode_entities(&src[from..to])));
        }
    };
    while i < b.len() {
        if b[i] != b'<' {
            i += 1;
            continue;
        }
        let rest = &src[i..];
        if rest.starts_with("<!--") {
            flush(&mut out, text_start, i);
            i = rest.find("-->").map_or(b.len(), |e| i + e + 3);
            text_start = i;
        } else if rest.starts_with("<![CDATA[") {
            flush(&mut out, text_start, i);
            let end = rest.find("]]>").map_or(b.len(), |e| i + e);
            out.push(Token::Text(src[i + 9..end.max(i + 9)].to_string()));
            i = (end + 3).min(b.len());
            text_start = i;
        } else if rest.starts_with("<!") || rest.starts_with("<?") {
            flush(&mut out, text_start, i);
            i = rest.find('>').map_or(b.len(), |e| i + e + 1);
            text_start = i;
        } else if rest.len() > 1 && (b[i + 1] == b'/' || b[i + 1].is_ascii_alphabetic()) {
            flush(&mut out, text_start, i);
            let end = tag_end(b, i + 1);
            let inner = &src[i + 1..end.min(b.len())];
            if let Some(name) = inner.strip_prefix('/') {
                out.push(Token::End { name: tag_name(name) });
            } else {
                let closed = inner.trim_end().ends_with('/');
                let inner = inner.trim_end().trim_end_matches('/');
                let name = tag_name(inner);
                let attrs = parse_attrs(&inner[name.len().min(inner.len())..]);
                out.push(Token::Start { name, attrs, closed });
            }
            i = (end + 1).min(b.len());
            text_start = i;
        } else {
            i += 1; // a stray "<"
        }
    }
    flush(&mut out, text_start, b.len());
    out
}

/// The `>` that ends a tag, skipping any inside quoted attribute values.
fn tag_end(b: &[u8], mut i: usize) -> usize {
    let mut quote = 0u8;
    while i < b.len() {
        let c = b[i];
        if quote != 0 {
            if c == quote {
                quote = 0;
            }
        } else if c == b'"' || c == b'\'' {
            quote = c;
        } else if c == b'>' {
            return i;
        }
        i += 1;
    }
    b.len()
}

/// Lower-case local name: "xhtml:P" and "P" are both "p"; keeps "l:href"-style
/// prefixes off element names but leaves "mbp:pagebreak" recognisable.
fn tag_name(s: &str) -> String {
    let raw: String = s
        .chars()
        .take_while(|c| !c.is_whitespace() && *c != '/' && *c != '>')
        .collect();
    let raw = raw.to_ascii_lowercase();
    if raw == "mbp:pagebreak" {
        return raw;
    }
    raw.rsplit(':').next().unwrap_or("").to_string()
}

fn parse_attrs(s: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        while i < b.len() && b[i].is_ascii_whitespace() {
            i += 1;
        }
        let start = i;
        while i < b.len() && !b[i].is_ascii_whitespace() && b[i] != b'=' {
            i += 1;
        }
        if start == i {
            i += 1;
            continue;
        }
        let name = s[start..i].to_ascii_lowercase();
        while i < b.len() && b[i].is_ascii_whitespace() {
            i += 1;
        }
        let mut value = String::new();
        if i < b.len() && b[i] == b'=' {
            i += 1;
            while i < b.len() && b[i].is_ascii_whitespace() {
                i += 1;
            }
            if i < b.len() && (b[i] == b'"' || b[i] == b'\'') {
                let q = b[i];
                let vs = i + 1;
                i = vs;
                while i < b.len() && b[i] != q {
                    i += 1;
                }
                value = decode_entities(&s[vs..i.min(b.len())]);
                i += 1;
            } else {
                let vs = i;
                while i < b.len() && !b[i].is_ascii_whitespace() {
                    i += 1;
                }
                value = decode_entities(&s[vs..i]);
            }
        }
        out.push((name, value));
    }
    out
}

/// `&amp;`, `&#8212;`, `&#x2014;` and the common named entities.
pub fn decode_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(pos) = rest.find('&') {
        out.push_str(&rest[..pos]);
        let after = &rest[pos + 1..];
        let end = after.find(';').filter(|e| *e <= 10);
        let decoded = end.and_then(|e| {
            let name = &after[..e];
            let ch = if let Some(num) = name.strip_prefix("#x").or_else(|| name.strip_prefix("#X")) {
                u32::from_str_radix(num, 16).ok().and_then(char::from_u32)
            } else if let Some(num) = name.strip_prefix('#') {
                num.parse::<u32>().ok().and_then(char::from_u32)
            } else {
                named_entity(name)
            };
            ch.map(|c| (c, e))
        });
        match decoded {
            Some((c, e)) => {
                out.push(c);
                rest = &after[e + 1..];
            }
            None => {
                out.push('&');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

fn named_entity(name: &str) -> Option<char> {
    Some(match name {
        "amp" => '&',
        "lt" => '<',
        "gt" => '>',
        "quot" => '"',
        "apos" => '\'',
        "nbsp" => '\u{a0}',
        "mdash" => '—',
        "ndash" => '–',
        "hellip" => '…',
        "lsquo" => '‘',
        "rsquo" => '’',
        "ldquo" => '“',
        "rdquo" => '”',
        "laquo" => '«',
        "raquo" => '»',
        "copy" => '©',
        "reg" => '®',
        "trade" => '™',
        "shy" => '\u{ad}',
        "eacute" => 'é',
        "egrave" => 'è',
        "agrave" => 'à',
        "ccedil" => 'ç',
        "uuml" => 'ü',
        "ouml" => 'ö',
        "auml" => 'ä',
        "szlig" => 'ß',
        "deg" => '°',
        "middot" => '·',
        "bull" => '•',
        "times" => '×',
        "sect" => '§',
        "para" => '¶',
        "pound" => '£',
        "euro" => '€',
        _ => return None,
    })
}

/// Turns tokens into blocks.
///
/// `image` resolves an image reference (whatever attribute held it) to an
/// index in the book's image list. Element `id`s are recorded with the block
/// they start at, so tables of contents can point into the text.
pub struct Builder<'a> {
    pub blocks: Vec<Block>,
    pub anchors: HashMap<String, usize>,
    image: &'a mut dyn FnMut(&str) -> Option<usize>,
    runs: Vec<Run>,
    bold: u32,
    italic: u32,
    heading: Option<u8>,
    skip: u32,
    /// FB2: `<title>` is a section heading, and sections nest.
    fb2: bool,
    section_depth: u8,
    pending_ids: Vec<String>,
    /// span/p/div opened with an italic or bold style: (tag, bold, italic).
    styled: Vec<(String, bool, bool)>,
}

impl<'a> Builder<'a> {
    pub fn new(fb2: bool, image: &'a mut dyn FnMut(&str) -> Option<usize>) -> Self {
        Builder {
            blocks: Vec::new(),
            anchors: HashMap::new(),
            image,
            runs: Vec::new(),
            bold: 0,
            italic: 0,
            heading: None,
            skip: 0,
            fb2,
            section_depth: 0,
            pending_ids: Vec::new(),
            styled: Vec::new(),
        }
    }

    /// Marks a point (a file in an EPUB, say) as starting at the next block.
    pub fn anchor(&mut self, name: &str) {
        self.pending_ids.push(name.to_string());
    }

    fn style(&self) -> Style {
        Style { bold: self.bold > 0, italic: self.italic > 0 }
    }

    fn push_text(&mut self, text: &str) {
        // HTML whitespace: runs of it are one space.
        let mut t = String::with_capacity(text.len());
        let mut space = false;
        for c in text.chars() {
            if c.is_whitespace() && c != '\u{a0}' {
                space = true;
            } else {
                if space && !(t.is_empty() && self.runs.is_empty()) {
                    t.push(' ');
                }
                space = false;
                if c != '\u{ad}' {
                    t.push(c);
                }
            }
        }
        // Trailing whitespace counts, unless nothing precedes it at all.
        if space && (!t.is_empty() || !self.runs.is_empty()) {
            t.push(' ');
        }
        if t.is_empty() {
            return;
        }
        let style = self.style();
        match self.runs.last_mut() {
            Some(last) if last.style == style => {
                if last.text.ends_with(' ') && t.starts_with(' ') {
                    last.text.push_str(&t[1..]);
                } else {
                    last.text.push_str(&t);
                }
            }
            _ => self.runs.push(Run { text: t, style }),
        }
    }

    fn line_break(&mut self) {
        if let Some(last) = self.runs.last_mut() {
            let trimmed = last.text.trim_end().len();
            last.text.truncate(trimmed);
            last.text.push('\n');
        }
    }

    /// Ends the paragraph (or heading) being collected, if it has any text.
    fn flush(&mut self) {
        let mut runs = std::mem::take(&mut self.runs);
        // Trim the edges.
        while runs.first().is_some_and(|r| r.text.trim().is_empty()) {
            runs.remove(0);
        }
        while runs.last().is_some_and(|r| r.text.trim().is_empty()) {
            runs.pop();
        }
        if let Some(first) = runs.first_mut() {
            first.text = first.text.trim_start().to_string();
        }
        if let Some(last) = runs.last_mut() {
            last.text = last.text.trim_end().to_string();
        }
        if runs.is_empty() {
            return;
        }
        self.take_anchors();
        self.blocks.push(match self.heading {
            Some(level) => Block::Heading { level, runs },
            None => Block::Para { runs },
        });
    }

    fn take_anchors(&mut self) {
        let at = self.blocks.len();
        for id in self.pending_ids.drain(..) {
            self.anchors.entry(id).or_insert(at);
        }
    }

    fn push_block(&mut self, block: Block) {
        self.flush();
        self.take_anchors();
        self.blocks.push(block);
    }

    pub fn feed(&mut self, tokens: &[Token]) {
        for tok in tokens {
            match tok {
                Token::Text(t) => {
                    if self.skip == 0 {
                        self.push_text(t);
                    }
                }
                Token::Start { name, attrs, closed } => {
                    let name = name.as_str();
                    if self.skip > 0 {
                        if !closed && is_skipped(name, self.fb2) {
                            self.skip += 1;
                        }
                        continue;
                    }
                    if let Some((_, id)) = attrs.iter().find(|(k, _)| k == "id" || k == "name") {
                        if !id.is_empty() {
                            self.pending_ids.push(id.clone());
                        }
                    }
                    if is_skipped(name, self.fb2) {
                        if !closed {
                            self.skip += 1;
                        }
                        continue;
                    }
                    self.start(name, attrs, *closed);
                }
                Token::End { name } => {
                    let name = name.as_str();
                    if self.skip > 0 {
                        if is_skipped(name, self.fb2) {
                            self.skip -= 1;
                        }
                        continue;
                    }
                    self.end(name);
                }
            }
        }
    }

    fn start(&mut self, name: &str, attrs: &[(String, String)], closed: bool) {
        match name {
            "b" | "strong" => self.bold += u32::from(!closed),
            "i" | "em" | "emphasis" | "cite" | "dfn" | "var" => self.italic += u32::from(!closed),
            "br" => self.line_break(),
            "hr" => self.push_block(Block::Rule),
            "mbp:pagebreak" => self.push_block(Block::PageBreak),
            "empty-line" => self.flush(),
            "img" | "image" => {
                let src = ["src", "recindex", "href", "xlink:href", "l:href"]
                    .iter()
                    .find_map(|k| attrs.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone()));
                if let Some(id) = src.and_then(|s| (self.image)(&s)) {
                    self.push_block(Block::Image { id });
                }
            }
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                self.flush();
                self.heading = Some(name.as_bytes()[1] - b'0');
            }
            "section" if self.fb2 => {
                self.flush();
                self.section_depth = self.section_depth.saturating_add(1);
            }
            "title" if self.fb2 => {
                self.flush();
                self.heading = Some(self.section_depth.clamp(1, 4));
            }
            "subtitle" if self.fb2 => {
                self.flush();
                self.heading = Some(4);
            }
            _ if is_block(name) => self.flush(),
            _ => {}
        }
        // Some styling arrives as a class or inline style: honour the obvious.
        if !closed && matches!(name, "span" | "p" | "div") {
            let style = attrs
                .iter()
                .filter(|(k, _)| k == "style" || k == "class")
                .map(|(_, v)| v.to_ascii_lowercase())
                .collect::<String>();
            if style.contains("italic") {
                self.italic += 1;
                self.styled.push((name.to_string(), false, true));
            } else if style.contains("bold") {
                self.bold += 1;
                self.styled.push((name.to_string(), true, false));
            } else {
                self.styled.push((name.to_string(), false, false));
            }
        }
    }

    fn end(&mut self, name: &str) {
        match name {
            "b" | "strong" => self.bold = self.bold.saturating_sub(1),
            "i" | "em" | "emphasis" | "cite" | "dfn" | "var" => {
                self.italic = self.italic.saturating_sub(1)
            }
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                self.flush();
                self.heading = None;
            }
            "title" | "subtitle" if self.fb2 => {
                self.flush();
                self.heading = None;
            }
            "section" if self.fb2 => {
                self.flush();
                self.section_depth = self.section_depth.saturating_sub(1);
            }
            _ if is_block(name) => self.flush(),
            _ => {}
        }
        if matches!(name, "span" | "p" | "div") {
            if let Some(pos) = self.styled.iter().rposition(|(n, _, _)| n == name) {
                let (_, b, i) = self.styled.remove(pos);
                if b {
                    self.bold = self.bold.saturating_sub(1);
                }
                if i {
                    self.italic = self.italic.saturating_sub(1);
                }
            }
        }
    }

    /// Finishes off: the last paragraph, and any anchors at the very end.
    pub fn finish(mut self) -> (Vec<Block>, HashMap<String, usize>) {
        self.flush();
        let at = self.blocks.len();
        for id in self.pending_ids.drain(..) {
            self.anchors.entry(id).or_insert(at.saturating_sub(1));
        }
        (self.blocks, self.anchors)
    }
}

fn is_skipped(name: &str, fb2: bool) -> bool {
    match name {
        "script" | "style" | "head" | "svg" | "noscript" | "math" => true,
        // In FB2 these hold metadata and base64 images, not text.
        "description" | "binary" if fb2 => true,
        "title" if !fb2 => true,
        _ => false,
    }
}

fn is_block(name: &str) -> bool {
    matches!(
        name,
        "p" | "div"
            | "li"
            | "dt"
            | "dd"
            | "blockquote"
            | "pre"
            | "section"
            | "article"
            | "aside"
            | "header"
            | "footer"
            | "figure"
            | "figcaption"
            | "tr"
            | "table"
            | "ul"
            | "ol"
            | "body"
            | "center"
            | "v"
            | "stanza"
            | "poem"
            | "epigraph"
            | "cite"
            | "text-author"
            | "annotation"
    )
}

/// Markup in, blocks out, for formats without images or anchors to resolve.
pub fn blocks_of(src: &str, fb2: bool) -> Vec<Block> {
    let mut none = |_: &str| None;
    let mut b = Builder::new(fb2, &mut none);
    b.feed(&tokenize(src));
    b.finish().0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(b: &Block) -> String {
        match b {
            Block::Heading { runs, .. } | Block::Para { runs } => {
                runs.iter().map(|r| r.text.as_str()).collect()
            }
            _ => String::new(),
        }
    }

    #[test]
    fn paragraphs_headings_and_styles() {
        let html = r#"<html><head><title>X</title><style>p{}</style></head><body>
            <h1 id="c1">Chapter&nbsp;One</h1>
            <p>It was a <i>dark</i> and <b>stormy</b>   night &amp; more&#8230;</p>
            <p class="calibre">Second <span style="font-style: italic">line</span></p>
            <hr/><mbp:pagebreak/></body></html>"#;
        let blocks = blocks_of(html, false);
        assert_eq!(blocks.len(), 5, "{blocks:?}");
        assert!(matches!(&blocks[0], Block::Heading { level: 1, .. }));
        assert_eq!(text(&blocks[0]), "Chapter\u{a0}One");
        assert_eq!(text(&blocks[1]), "It was a dark and stormy night & more…");
        let Block::Para { runs } = &blocks[1] else { panic!() };
        assert!(runs.iter().any(|r| r.text == "dark" && r.style.italic));
        assert!(runs.iter().any(|r| r.text == "stormy" && r.style.bold));
        let Block::Para { runs } = &blocks[2] else { panic!() };
        assert!(runs.iter().any(|r| r.text == "line" && r.style.italic));
        assert_eq!(blocks[3], Block::Rule);
        assert_eq!(blocks[4], Block::PageBreak);
    }

    #[test]
    fn survives_tag_soup() {
        let blocks = blocks_of("<p>open <b>never closed<p>next < 3 > 2 <!-- c --> end", false);
        assert_eq!(blocks.len(), 2);
        assert_eq!(text(&blocks[1]), "next < 3 > 2 end");
    }

    #[test]
    fn fb2_titles_are_headings() {
        let fb2 = r#"<FictionBook><description><title-info><book-title>B</book-title></title-info></description>
            <body><section><title><p>Part 1</p></title><p>Text <emphasis>here</emphasis>.</p>
            <section><title><p>Chapter A</p></title><empty-line/><p>More</p></section></section></body>
            <binary id="x">AAAA</binary></FictionBook>"#;
        let blocks = blocks_of(fb2, true);
        let kinds: Vec<String> = blocks
            .iter()
            .map(|b| match b {
                Block::Heading { level, .. } => format!("h{level}:{}", text(b)),
                _ => text(b),
            })
            .collect();
        assert_eq!(kinds, ["h1:Part 1", "Text here.", "h2:Chapter A", "More"]);
    }

    #[test]
    fn anchors_point_at_blocks() {
        let mut none = |_: &str| None;
        let mut b = Builder::new(false, &mut none);
        b.anchor("file1.html");
        b.feed(&tokenize("<p>one</p><div id='x'><p>two</p></div>"));
        let (blocks, anchors) = b.finish();
        assert_eq!(blocks.len(), 2);
        assert_eq!(anchors["file1.html"], 0);
        assert_eq!(anchors["x"], 1);
    }
}
