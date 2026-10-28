//! ENML (Evernote's XHTML) → Markdown, in the dialect Memoir's editor reads
//! and writes (see app/src/lib/editor/markdown.test.ts): `**bold**`,
//! `*italic*`, `~~struck~~`, `- [x]` checklists, GFM tables, fenced code,
//! `![name](attachment:<id>)` images.
//!
//! Evernote formats this importer understands:
//! - checklists as `<en-todo checked="true"/>` at the start of a line (older
//!   notes) or `<ul style="--en-todo:true">` / `<li style="--en-checked:true">`;
//! - code blocks as `<div style="--en-codeblock:true">`;
//! - attachments as `<en-media hash="<md5>">`, resolved by the caller;
//! - encrypted text (`<en-crypt>`) can't be imported: a marker says so.
//!
//! Anything else (web clips with arbitrary HTML) is reduced to text,
//! paragraphs, links and the formatting above.

use std::collections::HashSet;

use quick_xml::Reader;
use quick_xml::XmlVersion;
use quick_xml::escape::resolve_html5_entity;
use quick_xml::events::{BytesStart, Event};

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Converted {
    pub markdown: String,
    /// `<en-media>` hashes the content refers to.
    pub used_media: HashSet<String>,
    /// Encrypted sections that couldn't be imported.
    pub encrypted: usize,
}

/// Converts a note's ENML. `media(hash)` returns the Markdown for an
/// attachment (`![name](attachment:…)`), or None if it's missing.
pub fn to_markdown(enml: &str, media: &mut dyn FnMut(&str) -> Option<String>) -> Converted {
    let root = parse(enml);
    let mut c = Converter {
        media,
        used: HashSet::new(),
        encrypted: 0,
    };
    let blocks = c.blocks(&root.children);
    // As the editor writes it: no newline at the end.
    let markdown = render_blocks(&blocks);
    Converted {
        markdown,
        used_media: c.used,
        encrypted: c.encrypted,
    }
}

// ---- Tree ------------------------------------------------------------------

#[derive(Debug, Clone)]
enum Node {
    El(Element),
    Text(String),
}

#[derive(Debug, Clone, Default)]
struct Element {
    name: String,
    attrs: Vec<(String, String)>,
    children: Vec<Node>,
}

impl Element {
    fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }

    /// The `style` attribute without whitespace, lowercased.
    fn style(&self) -> String {
        self.attr("style")
            .unwrap_or("")
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect::<String>()
            .to_ascii_lowercase()
    }
}

fn element(e: &BytesStart<'_>) -> Element {
    let attrs = e
        .attributes()
        .flatten()
        .map(|a| {
            let key = a.key.as_ref().to_ascii_lowercase();
            let value = a
                .normalized_value_with(XmlVersion::Implicit1_0, 16, resolve_html5_entity)
                .map(|v| v.into_owned())
                .unwrap_or_else(|_| a.value.clone().into_owned());
            (key, value)
        })
        .collect();
    Element {
        name: e.name().as_ref().to_ascii_lowercase(),
        attrs,
        children: Vec::new(),
    }
}

/// Parses leniently: mismatched end tags close what they can, and parsing
/// errors keep what was read so far.
fn parse(enml: &str) -> Element {
    let mut reader = Reader::from_str(enml);
    let config = reader.config_mut();
    config.check_end_names = false;
    config.allow_unmatched_ends = true;

    let mut stack = vec![Element::default()];
    let push_text = |stack: &mut Vec<Element>, s: &str| {
        let top = stack.last_mut().expect("root stays");
        match top.children.last_mut() {
            Some(Node::Text(t)) => t.push_str(s),
            _ => top.children.push(Node::Text(s.to_owned())),
        }
    };
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => stack.push(element(&e)),
            Ok(Event::Empty(e)) => {
                let el = element(&e);
                stack.last_mut().expect("root").children.push(Node::El(el));
            }
            Ok(Event::End(e)) => {
                let name = e.name().as_ref().to_ascii_lowercase();
                if let Some(pos) = stack.iter().skip(1).rposition(|el| el.name == name) {
                    while stack.len() > pos + 1 {
                        let done = stack.pop().expect("above root");
                        stack
                            .last_mut()
                            .expect("root")
                            .children
                            .push(Node::El(done));
                    }
                }
            }
            Ok(Event::Text(e)) => push_text(&mut stack, &e.xml10_content()),
            Ok(Event::CData(e)) => push_text(&mut stack, &e.xml10_content()),
            Ok(Event::GeneralRef(e)) => {
                if let Ok(Some(c)) = e.resolve_char_ref() {
                    push_text(&mut stack, c.encode_utf8(&mut [0; 4]));
                } else {
                    let name = e.into_inner();
                    let text = resolve_html5_entity(&name)
                        .map(str::to_owned)
                        .unwrap_or_else(|| format!("&{name};"));
                    push_text(&mut stack, &text);
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            Ok(_) => {}
        }
    }
    while stack.len() > 1 {
        let done = stack.pop().expect("above root");
        stack
            .last_mut()
            .expect("root")
            .children
            .push(Node::El(done));
    }
    stack.pop().expect("root")
}

// ---- Blocks ----------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
enum Block {
    Para(String),
    Heading(usize, String),
    List(List),
    Code(String),
    Quote(Vec<Block>),
    Table(Vec<Vec<String>>),
    Rule,
}

#[derive(Debug, Clone, PartialEq)]
struct List {
    ordered: bool,
    start: u32,
    task: bool,
    items: Vec<Item>,
}

#[derive(Debug, Clone, PartialEq)]
struct Item {
    checked: bool,
    blocks: Vec<Block>,
}

const BLOCK_TAGS: &[&str] = &[
    "div",
    "p",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "ul",
    "ol",
    "li",
    "blockquote",
    "pre",
    "table",
    "thead",
    "tbody",
    "tfoot",
    "tr",
    "td",
    "th",
    "hr",
    "en-note",
    "en-crypt",
    "section",
    "article",
    "header",
    "footer",
    "nav",
    "aside",
    "main",
    "center",
    "figure",
    "figcaption",
    "dl",
    "dt",
    "dd",
    "address",
];

fn is_block(node: &Node) -> bool {
    match node {
        Node::Text(_) => false,
        Node::El(el) => BLOCK_TAGS.contains(&el.name.as_str()) || el.children.iter().any(is_block),
    }
}

fn is_blank(node: &Node) -> bool {
    matches!(node, Node::Text(t) if t.trim().is_empty())
}

fn is_todo(node: &Node) -> bool {
    matches!(node, Node::El(el) if el.name == "en-todo")
}

/// Hard line break inside a paragraph, until the final rendering.
const BREAK: char = '\u{1}';

struct Converter<'a> {
    media: &'a mut dyn FnMut(&str) -> Option<String>,
    used: HashSet<String>,
    encrypted: usize,
}

impl Converter<'_> {
    fn blocks(&mut self, nodes: &[Node]) -> Vec<Block> {
        let mut out = Vec::new();
        let mut inline: Vec<Node> = Vec::new();
        for node in nodes {
            if is_block(node) {
                self.flush(&mut inline, &mut out);
                if let Node::El(el) = node {
                    for block in self.block(el) {
                        push_block(&mut out, block);
                    }
                }
            } else {
                inline.push(node.clone());
            }
        }
        self.flush(&mut inline, &mut out);
        out
    }

    fn flush(&mut self, inline: &mut Vec<Node>, out: &mut Vec<Block>) {
        if inline.is_empty() {
            return;
        }
        let nodes = std::mem::take(inline);
        if nodes.iter().any(is_todo) {
            for block in self.todo_items(&nodes) {
                push_block(out, block);
            }
            return;
        }
        let text = self.inline(&nodes);
        if !text.is_empty() {
            push_block(out, Block::Para(text));
        }
    }

    fn block(&mut self, el: &Element) -> Vec<Block> {
        let style = el.style();
        match el.name.as_str() {
            h @ ("h1" | "h2" | "h3" | "h4" | "h5" | "h6") => {
                let level = (h.as_bytes()[1] - b'0') as usize;
                let text = self.inline(&el.children).replace(BREAK, " ");
                if text.trim().is_empty() {
                    vec![]
                } else {
                    vec![Block::Heading(level, text)]
                }
            }
            "ul" | "ol" => vec![self.list(el)],
            "blockquote" => {
                let inner = self.blocks(&el.children);
                if inner.is_empty() {
                    vec![]
                } else {
                    vec![Block::Quote(inner)]
                }
            }
            "pre" => code_block(&el.children),
            _ if style.contains("--en-codeblock:true") => code_block(&el.children),
            "table" => {
                let rows = self.table(el);
                if rows.is_empty() {
                    vec![]
                } else {
                    vec![Block::Table(rows)]
                }
            }
            "hr" => vec![Block::Rule],
            "en-crypt" => {
                self.encrypted += 1;
                vec![Block::Para(
                    "*\\[Encrypted text from Evernote, not imported\\]*".into(),
                )]
            }
            _ => self.blocks(&el.children),
        }
    }

    /// Older checklists: `<en-todo/>` starts each item, within one line or
    /// across `<br/>`s.
    fn todo_items(&mut self, nodes: &[Node]) -> Vec<Block> {
        let mut blocks = Vec::new();
        let mut before: Vec<Node> = Vec::new();
        let mut items: Vec<Item> = Vec::new();
        let mut current: Option<(bool, Vec<Node>)> = None;
        for node in nodes {
            if let Node::El(el) = node
                && el.name == "en-todo"
            {
                if let Some((checked, content)) = current.take() {
                    items.push(self.todo_item(checked, &content));
                }
                current = Some((el.attr("checked") == Some("true"), Vec::new()));
                continue;
            }
            match current.as_mut() {
                Some((_, content)) => content.push(node.clone()),
                None => before.push(node.clone()),
            }
        }
        if let Some((checked, content)) = current.take() {
            items.push(self.todo_item(checked, &content));
        }
        let lead = self.inline(&before);
        if !lead.is_empty() {
            blocks.push(Block::Para(lead));
        }
        blocks.push(Block::List(List {
            ordered: false,
            start: 1,
            task: true,
            items,
        }));
        blocks
    }

    fn todo_item(&mut self, checked: bool, content: &[Node]) -> Item {
        let text = self.inline(content);
        let text = text.trim_matches(BREAK).replace(BREAK, " ");
        Item {
            checked,
            blocks: if text.is_empty() {
                vec![]
            } else {
                vec![Block::Para(text)]
            },
        }
    }

    fn list(&mut self, el: &Element) -> Block {
        let ordered = el.name == "ol";
        let task = el.style().contains("--en-todo:true");
        let start = el
            .attr("start")
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(1);
        let mut items: Vec<Item> = Vec::new();
        for child in &el.children {
            match child {
                Node::El(li) if li.name == "li" => {
                    let checked = li.style().contains("--en-checked:true");
                    let blocks = self.blocks(&li.children);
                    items.push(Item { checked, blocks });
                }
                node if is_blank(node) => {}
                // Stray content (e.g. a nested list outside any <li>): it
                // belongs to the item before it.
                node => {
                    let extra = self.blocks(std::slice::from_ref(node));
                    match items.last_mut() {
                        Some(item) => item.blocks.extend(extra),
                        None => items.push(Item {
                            checked: false,
                            blocks: extra,
                        }),
                    }
                }
            }
        }
        Block::List(List {
            ordered,
            start,
            task,
            items,
        })
    }

    fn table(&mut self, el: &Element) -> Vec<Vec<String>> {
        let mut rows = Vec::new();
        collect_rows(el, &mut |tr: &Element| {
            let cells: Vec<String> = tr
                .children
                .iter()
                .filter_map(|c| match c {
                    Node::El(cell) if cell.name == "td" || cell.name == "th" => Some(
                        self.inline(&cell.children)
                            .replace(BREAK, " ")
                            .replace('|', "\\|"),
                    ),
                    _ => None,
                })
                .collect();
            if !cells.is_empty() {
                rows.push(cells);
            }
        });
        rows
    }

    /// Inline Markdown for a whole line or cell (BREAK for hard line breaks).
    fn inline(&mut self, nodes: &[Node]) -> String {
        let raw = self.inline_part(nodes);
        tidy_inline(&raw, true)
    }

    /// Inline Markdown for part of a line: spaces at its edges are kept, so
    /// `<b>bold </b>next` keeps its space (outside the markers).
    fn inline_part(&mut self, nodes: &[Node]) -> String {
        let mut out = String::new();
        for node in nodes {
            match node {
                Node::Text(t) => out.push_str(&escape_text(&collapse_whitespace(t))),
                Node::El(el) => out.push_str(&self.inline_element(el)),
            }
        }
        tidy_inline(&out, false)
    }

    fn inline_element(&mut self, el: &Element) -> String {
        let style = el.style();
        match el.name.as_str() {
            "br" => BREAK.to_string(),
            "b" | "strong" => wrap("**", &self.inline_part(&el.children)),
            "i" | "em" | "cite" | "dfn" => wrap("*", &self.inline_part(&el.children)),
            "s" | "strike" | "del" => wrap("~~", &self.inline_part(&el.children)),
            "code" | "tt" | "kbd" | "samp" => code_span(&plain_text(&el.children)),
            "a" => {
                let part = self.inline_part(&el.children);
                match el.attr("href").map(str::trim).filter(|h| !h.is_empty()) {
                    Some(href) => {
                        // Spaces at the edges of the link text go outside it.
                        let lead = if part.starts_with(' ') { " " } else { "" };
                        let tail = if part.ends_with(' ') { " " } else { "" };
                        let inner = tidy_inline(&part, true);
                        let text = if inner.is_empty() {
                            escape_text(href)
                        } else {
                            inner
                        };
                        format!("{lead}[{text}]({}){tail}", link_destination(href))
                    }
                    None => part,
                }
            }
            "en-media" => {
                let Some(hash) = el.attr("hash").map(str::to_ascii_lowercase) else {
                    return String::new();
                };
                let markdown = (self.media)(&hash);
                self.used.insert(hash);
                markdown.unwrap_or_else(|| "\\[missing attachment\\]".into())
            }
            "img" => {
                // Memoir shows only its own (encrypted) images; web images
                // become links.
                let src = el.attr("src").unwrap_or("").trim();
                if src.starts_with("http://") || src.starts_with("https://") {
                    let alt = el.attr("alt").map(str::trim).filter(|a| !a.is_empty());
                    format!(
                        "[{}]({})",
                        escape_text(alt.unwrap_or("Image")),
                        link_destination(src)
                    )
                } else {
                    String::new()
                }
            }
            "en-todo" => {
                if el.attr("checked") == Some("true") {
                    "☑ ".into()
                } else {
                    "☐ ".into()
                }
            }
            "en-crypt" => {
                self.encrypted += 1;
                "*\\[Encrypted text from Evernote, not imported\\]*".into()
            }
            "script" | "style" | "head" | "title" | "object" | "embed" => String::new(),
            _ => {
                let mut inner = self.inline_part(&el.children);
                if style.contains("font-weight:bold") || style.contains("font-weight:700") {
                    inner = wrap("**", &inner);
                }
                if style.contains("font-style:italic") {
                    inner = wrap("*", &inner);
                }
                if style.contains("line-through") {
                    inner = wrap("~~", &inner);
                }
                if BLOCK_TAGS.contains(&el.name.as_str()) {
                    inner.push(' ');
                }
                inner
            }
        }
    }
}

/// Consecutive checklists (one per line in older notes) form one list.
fn push_block(out: &mut Vec<Block>, block: Block) {
    if let (Some(Block::List(prev)), Block::List(next)) = (out.last_mut(), &block)
        && prev.task
        && next.task
    {
        prev.items.extend(next.items.iter().cloned());
        return;
    }
    out.push(block);
}

fn collect_rows(el: &Element, each: &mut dyn FnMut(&Element)) {
    for child in &el.children {
        if let Node::El(c) = child {
            match c.name.as_str() {
                "tr" => each(c),
                "thead" | "tbody" | "tfoot" => collect_rows(c, each),
                _ => {}
            }
        }
    }
}

/// A code block's text: one line per line-level element or `<br/>`.
fn code_block(nodes: &[Node]) -> Vec<Block> {
    fn walk(nodes: &[Node], out: &mut String) {
        for node in nodes {
            match node {
                Node::Text(t) => out.push_str(t),
                Node::El(el) if el.name == "br" => out.push('\n'),
                Node::El(el) if matches!(el.name.as_str(), "div" | "p" | "li" | "pre") => {
                    walk(&el.children, out);
                    if !out.ends_with('\n') {
                        out.push('\n');
                    }
                }
                Node::El(el) => walk(&el.children, out),
            }
        }
    }
    let mut text = String::new();
    walk(nodes, &mut text);
    let text = text.replace('\u{a0}', " ");
    let text = text.trim_matches('\n');
    if text.trim().is_empty() {
        vec![]
    } else {
        vec![Block::Code(text.to_owned())]
    }
}

fn plain_text(nodes: &[Node]) -> String {
    let mut out = String::new();
    for node in nodes {
        match node {
            Node::Text(t) => out.push_str(t),
            Node::El(el) => out.push_str(&plain_text(&el.children)),
        }
    }
    collapse_whitespace(&out)
}

// ---- Text --------------------------------------------------------------------

/// HTML whitespace rules: runs of spaces and newlines are one space.
fn collapse_whitespace(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut space = false;
    for c in text.chars() {
        if c.is_whitespace() || c == '\u{a0}' {
            if !space {
                out.push(' ');
                space = true;
            }
        } else {
            out.push(c);
            space = false;
        }
    }
    out
}

/// Escapes what Markdown would read as formatting, the way the editor does
/// (`<` and `>` as entities; `|` only matters in tables, see `table`).
fn escape_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\\' | '`' | '*' | '_' | '[' | ']' | '~' => {
                out.push('\\');
                out.push(c);
            }
            _ => out.push(c),
        }
    }
    out
}

/// Joins pieces: single spaces and no spaces around line breaks; with
/// `trim`, nothing at the ends either.
fn tidy_inline(s: &str, trim: bool) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if c == ' ' && ((trim && out.is_empty()) || out.ends_with(' ') || out.ends_with(BREAK)) {
            continue;
        }
        if c == BREAK && out.ends_with(' ') {
            out.pop();
        }
        out.push(c);
    }
    if trim {
        while out.ends_with(' ') || out.ends_with(BREAK) {
            out.pop();
        }
        while out.starts_with(BREAK) {
            out.remove(0);
        }
    }
    out
}

/// `**text**` with the spaces kept outside the markers (Markdown ignores
/// `** text**`).
fn wrap(marker: &str, inner: &str) -> String {
    let trimmed = inner.trim_matches(|c| c == ' ' || c == BREAK);
    if trimmed.is_empty() {
        return inner.to_owned();
    }
    let start = &inner[..inner.len() - inner.trim_start_matches([' ', BREAK]).len()];
    let end = &inner[inner.trim_end_matches([' ', BREAK]).len()..];
    format!("{start}{marker}{trimmed}{marker}{end}")
}

fn code_span(text: &str) -> String {
    let text = text.trim();
    if text.is_empty() {
        return String::new();
    }
    let longest = longest_run(text, '`');
    let ticks = "`".repeat(longest + 1);
    let pad = if text.starts_with('`') || text.ends_with('`') {
        " "
    } else {
        ""
    };
    format!("{ticks}{pad}{text}{pad}{ticks}")
}

fn longest_run(text: &str, ch: char) -> usize {
    let (mut best, mut run) = (0, 0);
    for c in text.chars() {
        run = if c == ch { run + 1 } else { 0 };
        best = best.max(run);
    }
    best
}

/// A link target Markdown reads back as one piece.
fn link_destination(href: &str) -> String {
    href.replace(' ', "%20")
        .replace('(', "%28")
        .replace(')', "%29")
}

// ---- Rendering -----------------------------------------------------------------

fn render_blocks(blocks: &[Block]) -> String {
    blocks
        .iter()
        .map(render_block)
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn render_block(block: &Block) -> String {
    match block {
        Block::Para(text) => escape_line_starts(&text.replace(BREAK, "  \n")),
        Block::Heading(level, text) => format!("{} {}", "#".repeat(*level), text.trim()),
        Block::Rule => "---".into(),
        Block::Code(code) => {
            let fence = "`".repeat(longest_run(code, '`').max(2) + 1);
            format!("{fence}\n{code}\n{fence}")
        }
        Block::Quote(inner) => render_blocks(inner)
            .lines()
            .map(|l| {
                if l.is_empty() {
                    ">".to_owned()
                } else {
                    format!("> {l}")
                }
            })
            .collect::<Vec<_>>()
            .join("\n"),
        Block::Table(rows) => {
            // Columns padded to their widest cell (at least 3), as the
            // editor writes tables.
            let columns = rows.iter().map(Vec::len).max().unwrap_or(0);
            fn cell(r: &[String], i: usize) -> &str {
                r.get(i).map(|c| c.trim()).unwrap_or("")
            }
            let widths: Vec<usize> = (0..columns)
                .map(|i| {
                    rows.iter()
                        .map(|r| cell(r, i).chars().count())
                        .max()
                        .unwrap_or(0)
                        .max(3)
                })
                .collect();
            let line = |parts: Vec<String>| format!("| {} |", parts.join(" | "));
            let row = |r: &[String]| {
                line(
                    (0..columns)
                        .map(|i| {
                            let c = cell(r, i);
                            format!("{c}{}", " ".repeat(widths[i] - c.chars().count()))
                        })
                        .collect(),
                )
            };
            let mut lines = vec![
                row(&rows[0]),
                line(widths.iter().map(|w| "-".repeat(*w)).collect()),
            ];
            lines.extend(rows[1..].iter().map(|r| row(r)));
            // The editor sets tables apart with an extra blank line.
            format!("\n{}\n", lines.join("\n"))
        }
        Block::List(list) => render_list(list),
    }
}

fn render_list(list: &List) -> String {
    let mut lines = Vec::new();
    for (i, item) in list.items.iter().enumerate() {
        let marker = if list.task {
            if item.checked { "- [x] " } else { "- [ ] " }.to_owned()
        } else if list.ordered {
            format!("{}. ", list.start as usize + i)
        } else {
            "- ".to_owned()
        };
        // Nested content lines up under the item's text (task items use the
        // bullet's indent, as the editor writes them).
        let indent = " ".repeat(if list.task { 2 } else { marker.len() });
        let body = render_item_blocks(&item.blocks);
        let mut body_lines = body.lines();
        let first = body_lines.next().unwrap_or("");
        lines.push(format!("{marker}{first}").trim_end().to_owned());
        for line in body_lines {
            lines.push(if line.is_empty() {
                String::new()
            } else {
                format!("{indent}{line}")
            });
        }
    }
    lines.join("\n")
}

/// A list item's content: paragraphs apart by a blank line, a nested list
/// straight after the line before it (as the editor writes them).
fn render_item_blocks(blocks: &[Block]) -> String {
    let mut out = String::new();
    for block in blocks {
        let text = render_block(block);
        if text.is_empty() {
            continue;
        }
        if !out.is_empty() {
            out.push_str(if matches!(block, Block::List(_)) {
                "\n"
            } else {
                "\n\n"
            });
        }
        out.push_str(&text);
    }
    out
}

/// Escapes what Markdown would read as a block at the start of a line
/// (`# heading`, `- item`, `1. item`…; `>` is already `&gt;`).
fn escape_line_starts(text: &str) -> String {
    text.split('\n')
        .map(|line| {
            let trimmed = line.trim_start();
            let lead = &line[..line.len() - trimmed.len()];
            let first = trimmed.chars().next();
            if matches!(first, Some('#' | '+' | '-' | '=')) {
                return format!("{lead}\\{trimmed}");
            }
            let digits = trimmed.chars().take_while(char::is_ascii_digit).count();
            if digits > 0 && matches!(trimmed[digits..].chars().next(), Some('.' | ')')) {
                return format!("{lead}{}\\{}", &trimmed[..digits], &trimmed[digits..]);
            }
            line.to_owned()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn md(enml: &str) -> String {
        let mut media = |hash: &str| match hash {
            "aaa" => Some("![photo.png](attachment:111)".to_owned()),
            "bbb" => Some("[report.pdf](attachment:222)".to_owned()),
            _ => None,
        };
        to_markdown(enml, &mut media).markdown
    }

    fn note(body: &str) -> String {
        md(&format!(
            r#"<?xml version="1.0" encoding="UTF-8"?><!DOCTYPE en-note SYSTEM "http://xml.evernote.com/pub/enml2.dtd"><en-note>{body}</en-note>"#
        ))
    }

    #[test]
    fn lines_paragraphs_and_formatting() {
        assert_eq!(
            note(
                "<div>Hello <b>bold</b> and <i>it</i>, <s>gone</s>, <code>x*y</code></div><div><br/></div><div>Second&nbsp;line</div>"
            ),
            "Hello **bold** and *it*, ~~gone~~, `x*y`\n\nSecond line"
        );
        assert_eq!(
            note(
                r#"<div><span style="font-weight: bold;">Loud</span> <span style="font-style:italic">soft</span></div>"#
            ),
            "**Loud** *soft*"
        );
        assert_eq!(note("<div><b>bold </b>next</div>"), "**bold** next");
        assert_eq!(note("<div>one<br/>two</div>"), "one  \ntwo");
        assert_eq!(
            note("<h1>Title</h1><h3>Small</h3><hr/>"),
            "# Title\n\n### Small\n\n---"
        );
    }

    #[test]
    fn text_that_looks_like_markdown_stays_text() {
        assert_eq!(
            note(
                "<div># not a heading</div><div>1. not a list</div><div>- nor this</div><div>a *b* [c] &lt;d&gt; ~e~ | f_g</div>"
            ),
            "\\# not a heading\n\n1\\. not a list\n\n\\- nor this\n\na \\*b\\* \\[c\\] &lt;d&gt; \\~e\\~ | f\\_g"
        );
    }

    #[test]
    fn links_and_web_images() {
        assert_eq!(
            note(
                r#"<div><a href="https://example.com/a b">Example</a> <a href="https://x.org"></a></div>"#
            ),
            "[Example](https://example.com/a%20b) [https://x.org](https://x.org)"
        );
        assert_eq!(
            note(r#"<div><img src="https://example.com/cat.png" alt="Cat"/></div>"#),
            "[Cat](https://example.com/cat.png)"
        );
    }

    #[test]
    fn lists() {
        assert_eq!(
            note("<ul><li>one</li><li>two<ul><li>two.a</li></ul></li></ul>"),
            "- one\n- two\n  - two.a"
        );
        assert_eq!(
            note(r#"<ol start="3"><li><div>third</div></li><li>fourth</li></ol>"#),
            "3. third\n4. fourth"
        );
    }

    #[test]
    fn checklists_old_and_new() {
        assert_eq!(
            note(
                r#"<div><en-todo checked="true"/>Milk</div><div><en-todo checked="false"/>Bread</div><div>After</div>"#
            ),
            "- [x] Milk\n- [ ] Bread\n\nAfter"
        );
        assert_eq!(
            note(r#"<div>Shop:<br/><en-todo/>eggs<br/><en-todo checked="true"/>tea</div>"#),
            "Shop:\n\n- [ ] eggs\n- [x] tea"
        );
        assert_eq!(
            note(
                r#"<ul style="--en-todo:true;"><li style="--en-checked:true;"><div>done</div></li><li style="--en-checked:false;"><div>open</div></li></ul>"#
            ),
            "- [x] done\n- [ ] open"
        );
    }

    #[test]
    fn code_quotes_and_tables() {
        assert_eq!(
            note(
                r#"<div style="box-sizing: border-box; --en-codeblock:true;"><div>fn main() {</div><div>  let x = 1; // *keep*</div><div>}</div></div>"#
            ),
            "```\nfn main() {\n  let x = 1; // *keep*\n}\n```"
        );
        assert_eq!(note("<pre>a ``` b</pre>"), "````\na ``` b\n````");
        assert_eq!(
            note("<blockquote><div>quoted</div><div>more</div></blockquote>"),
            "> quoted\n>\n> more"
        );
        assert_eq!(
            note(
                "<table><tbody><tr><td><b>Name</b></td><td>Qty</td></tr><tr><td>Apples<br/>red</td><td>3</td></tr><tr><td>Pears</td></tr></tbody></table>"
            ),
            "\n| **Name**   | Qty |\n| ---------- | --- |\n| Apples red | 3   |\n| Pears      |     |\n"
        );
    }

    #[test]
    fn attachments_and_encrypted_text() {
        let mut media = |hash: &str| (hash == "aaa").then(|| "![p.png](attachment:111)".to_owned());
        let c = to_markdown(
            r#"<en-note><div>See <en-media type="image/png" hash="AAA"/></div><en-media hash="zzz" type="application/pdf"/><en-crypt cipher="AES">abc</en-crypt></en-note>"#,
            &mut media,
        );
        assert_eq!(
            c.markdown,
            "See ![p.png](attachment:111)\n\n\\[missing attachment\\]\n\n*\\[Encrypted text from Evernote, not imported\\]*"
        );
        assert_eq!(c.encrypted, 1);
        assert!(c.used_media.contains("aaa") && c.used_media.contains("zzz"));
    }

    #[test]
    fn web_clip_html_degrades_to_text() {
        assert_eq!(
            note(
                r#"<div style="x"><span><font color="red">Clipped</font> <span>text</span></span><script>evil()</script><section><p>Para</p></section></div>"#
            ),
            "Clipped text\n\nPara"
        );
        // Broken markup keeps what can be read.
        assert_eq!(
            note("<div>unclosed <b>bold</div><div>next</div>"),
            "unclosed **bold**\n\nnext"
        );
    }
}
