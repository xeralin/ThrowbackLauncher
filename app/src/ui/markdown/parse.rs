use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::HashMap;

use gpui::SharedString;
use html5gum::{Token, Tokenizer};
use markdown::mdast::{AlignKind, Node};
use markdown::{ParseOptions, to_mdast};

const SHA_LENGTH: usize = 7;
const USER_LENGTH: usize = 39;
const BLOCKS: [&str; 30] = [
    "details",
    "summary",
    "hr",
    "p",
    "div",
    "ul",
    "ol",
    "li",
    "dl",
    "dt",
    "dd",
    "table",
    "thead",
    "tbody",
    "tfoot",
    "tr",
    "th",
    "td",
    "caption",
    "blockquote",
    "pre",
    "center",
    "section",
    "article",
    "header",
    "footer",
    "figure",
    "figcaption",
    "address",
    "main",
];
const DROPPED: [&str; 9] = [
    "script",
    "style",
    "iframe",
    "title",
    "textarea",
    "xmp",
    "noembed",
    "noframes",
    "plaintext",
];

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Style {
    pub strong: bool,
    pub emphasis: bool,
    pub code: bool,
    pub strike: bool,
    pub inserted: bool,
    pub link: Option<usize>,
    pub image: Option<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Alert {
    Note,
    Tip,
    Important,
    Warning,
    Caution,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Align {
    Start,
    Center,
    End,
}

#[derive(Debug, PartialEq)]
pub struct Item {
    pub checked: Option<bool>,
    pub blocks: Vec<Block>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Picture {
    pub source: SharedString,
    pub width: Option<f32>,
}

#[derive(Debug, Default, PartialEq)]
pub struct Inline {
    pub text: String,
    pub spans: Vec<(usize, Style)>,
    pub links: Vec<SharedString>,
    pub images: Vec<Picture>,
}

#[derive(Debug, PartialEq)]
pub enum Block {
    Heading(u8, Inline),
    Paragraph(Inline),
    List {
        start: Option<u32>,
        tight: bool,
        items: Vec<Item>,
    },
    Quote(Vec<Block>),
    Alert(Alert, Vec<Block>),
    Code(SharedString),
    Rule,
    Table {
        align: Vec<Align>,
        rows: Vec<Vec<Inline>>,
    },
    Details {
        summary: Inline,
        open: bool,
        blocks: Vec<Block>,
    },
    Footnotes(Vec<Vec<Block>>),
}

impl Inline {
    fn span(&mut self, length: usize, style: Style) {
        if length == 0 {
            return;
        }
        match self.spans.last_mut() {
            Some((run, last)) if *last == style => *run += length,
            _ => self.spans.push((length, style)),
        }
    }

    fn push(&mut self, text: &str, style: Style) {
        let start = self.text.len();
        for letter in text.chars() {
            if !letter.is_ascii_whitespace() {
                self.text.push(letter);
            } else if !self.text.is_empty() && !self.text.ends_with([' ', '\n']) {
                self.text.push(' ');
            }
        }
        self.span(self.text.len() - start, style);
    }

    fn line_break(&mut self) {
        if self.text.ends_with(' ') {
            self.text.pop();
            match self.spans.last_mut() {
                Some((run, _)) if *run > 1 => *run -= 1,
                _ => drop(self.spans.pop()),
            }
        }
        self.text.push('\n');
        self.span(1, Style::default());
    }

    fn link(&mut self, url: SharedString) -> usize {
        self.links.push(url);
        self.links.len() - 1
    }

    fn blank(&self) -> bool {
        self.text.trim().is_empty()
    }
}

#[derive(Default)]
struct Tags {
    base: Style,
    open: Vec<(String, Style)>,
    dropped: Option<String>,
}

impl Tags {
    fn style(&self) -> Style {
        self.open.last().map_or(self.base, |(_, style)| *style)
    }
}

enum Mark {
    Open(String, bool),
    Close(String),
}

struct Frame {
    summary: Inline,
    open: bool,
    blocks: Vec<Block>,
}

#[derive(Default)]
struct Flow {
    done: Vec<Block>,
    frames: Vec<Frame>,
    line: Inline,
    tags: Tags,
    heading: Option<u8>,
    summary: bool,
}

impl Flow {
    fn push(&mut self, block: Block) {
        let blocks = match self.frames.last_mut() {
            Some(frame) => &mut frame.blocks,
            None => &mut self.done,
        };
        blocks.push(block);
    }

    fn close_line(&mut self) {
        let line = std::mem::take(&mut self.line);
        let (heading, summary) = (self.heading.take(), std::mem::take(&mut self.summary));
        if line.blank() {
            return;
        }
        match heading {
            Some(level) => self.push(Block::Heading(level, line)),
            None if summary => {
                if let Some(frame) = self.frames.last_mut() {
                    frame.summary = line;
                }
            }
            None => self.push(Block::Paragraph(line)),
        }
    }

    fn close_frame(&mut self) {
        self.close_line();
        if let Some(frame) = self.frames.pop() {
            self.push(Block::Details {
                summary: frame.summary,
                open: frame.open,
                blocks: frame.blocks,
            });
        }
    }

    fn mark(&mut self, mark: Mark) {
        self.close_line();
        match mark {
            Mark::Open(name, open) => match name.as_str() {
                "details" => self.frames.push(Frame {
                    summary: Inline::default(),
                    open,
                    blocks: Vec::new(),
                }),
                "summary" => self.summary = true,
                "hr" => self.push(Block::Rule),
                _ => self.heading = heading(&name),
            },
            Mark::Close(name) if name == "details" => self.close_frame(),
            Mark::Close(_) => {}
        }
    }
}

struct Context<'a> {
    repository: &'a str,
    definitions: HashMap<String, String>,
    footnotes: HashMap<String, &'a [Node]>,
    cited: RefCell<Vec<String>>,
}

pub fn parse(body: &str, repository: &str) -> Vec<Block> {
    let Ok(root) = to_mdast(body, &ParseOptions::gfm()) else {
        return Vec::new();
    };
    let mut context = Context {
        repository,
        definitions: HashMap::new(),
        footnotes: HashMap::new(),
        cited: RefCell::default(),
    };
    context.collect(&root);
    let mut blocks = context.blocks(children(&root));
    let mut notes = Vec::new();
    while notes.len() < context.cited.borrow().len() {
        let name = context.cited.borrow()[notes.len()].clone();
        let body = context.footnotes.get(&name).copied().unwrap_or_default();
        notes.push(context.blocks(body));
    }
    if !notes.is_empty() {
        blocks.push(Block::Footnotes(notes));
    }
    blocks
}

fn heading(name: &str) -> Option<u8> {
    match name.as_bytes() {
        [b'h', level @ b'1'..=b'6'] => Some(level - b'0'),
        _ => None,
    }
}

fn emoji(line: &str) -> Cow<'_, str> {
    if !line.contains(':') {
        return Cow::Borrowed(line);
    }
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(start) = rest.find(':') {
        let tail = &rest[start + 1..];
        let named = |byte: &u8| byte.is_ascii_alphanumeric() || b"_+-".contains(byte);
        let length = tail.bytes().take_while(named).count();
        let closed = tail[length..].starts_with(':');
        let found = closed.then(|| emojis::get_by_shortcode(&tail[..length]));
        match found.flatten() {
            Some(emoji) => {
                out.push_str(&rest[..start]);
                out.push_str(emoji.as_str());
                rest = &tail[length + 1..];
            }
            None => {
                out.push_str(&rest[..=start]);
                rest = tail;
            }
        }
    }
    out.push_str(rest);
    Cow::Owned(out)
}

fn alert(quote: &[Node]) -> Option<(Alert, Vec<Node>)> {
    let Some(Node::Paragraph(paragraph)) = quote.first() else {
        return None;
    };
    let Some(Node::Text(text)) = paragraph.children.first() else {
        return None;
    };
    let (marker, rest) = match text.value.split_once('\n') {
        Some((marker, rest)) => (marker, rest),
        None => (text.value.as_str(), ""),
    };
    let kind = match marker.trim().to_ascii_uppercase().as_str() {
        "[!NOTE]" => Alert::Note,
        "[!TIP]" => Alert::Tip,
        "[!IMPORTANT]" => Alert::Important,
        "[!WARNING]" => Alert::Warning,
        "[!CAUTION]" => Alert::Caution,
        _ => return None,
    };
    let mut first = paragraph.clone();
    match first.children.first_mut() {
        Some(Node::Text(text)) if !rest.is_empty() => text.value = rest.to_owned(),
        _ => {
            first.children.remove(0);
            if matches!(first.children.first(), Some(Node::Break(_))) {
                first.children.remove(0);
            }
        }
    }
    let mut body = Vec::with_capacity(quote.len());
    if !first.children.is_empty() {
        body.push(Node::Paragraph(first));
    }
    body.extend(quote[1..].iter().cloned());
    (!body.is_empty()).then_some((kind, body))
}

fn children(node: &Node) -> &[Node] {
    node.children().map_or(&[], Vec::as_slice)
}

fn openable(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    lower.starts_with("http://") || lower.starts_with("https://")
}

fn has_scheme(url: &str) -> bool {
    let Some((scheme, _)) = url.split_once(':') else {
        return false;
    };
    scheme.starts_with(|first: char| first.is_ascii_alphabetic())
        && scheme
            .chars()
            .all(|letter| letter.is_ascii_alphanumeric() || "+.-".contains(letter))
}

struct Reference {
    end: usize,
    url: String,
    mention: bool,
}

fn word(letter: char) -> bool {
    letter.is_ascii_alphanumeric() || letter == '_'
}

fn user_length(text: &str) -> usize {
    if text.starts_with('-') {
        return 0;
    }
    text.bytes()
        .take(USER_LENGTH)
        .take_while(|byte| byte.is_ascii_alphanumeric() || *byte == b'-')
        .count()
}

fn number_length(text: &str) -> usize {
    if text.starts_with('0') {
        return 0;
    }
    text.bytes().take_while(u8::is_ascii_digit).count()
}

fn mention(line: &str, at: usize) -> Option<Reference> {
    let name = &line[at + 1..];
    let mut length = user_length(name);
    if length > 0 && name[length..].starts_with('/') {
        let team = user_length(&name[length + 1..]);
        if team > 0 {
            length += 1 + team;
        }
    }
    let end = at + 1 + length;
    let blocked = |letter: char| word(letter) || letter == '`';
    let before = line[..at].chars().next_back();
    let after = line[end..].chars().next();
    let free =
        !before.is_some_and(blocked) && !after.is_some_and(|next| blocked(next) || next == '/');
    (length > 0 && free).then(|| Reference {
        end,
        url: format!("https://github.com/{}", &name[..length]),
        mention: true,
    })
}

fn issue(line: &str, at: usize, repository: &str) -> Option<Reference> {
    let before = line[..at].chars().next_back();
    let tail = &line[at..];
    let short = tail
        .get(..3)
        .is_some_and(|head| head.eq_ignore_ascii_case("gh-"));
    let (project, number_at) = if tail.starts_with('#') {
        (repository.to_owned(), at + 1)
    } else if short {
        (repository.to_owned(), at + 3)
    } else {
        if before.is_some_and(|previous| !"\t\n\r (@[{".contains(previous)) {
            return None;
        }
        let user = user_length(tail);
        if user == 0 {
            return None;
        }
        let mut cursor = user;
        let mut project = None;
        if tail[cursor..].starts_with('/') {
            let length = tail[cursor + 1..]
                .bytes()
                .take_while(|byte| word(char::from(*byte)) || *byte == b'-' || *byte == b'.')
                .count();
            if length == 0 {
                return None;
            }
            project = Some(&tail[cursor + 1..cursor + 1 + length]);
            cursor += 1 + length;
        }
        if !tail[cursor..].starts_with('#') {
            return None;
        }
        let (_, fallback) = repository.split_once('/').unwrap_or_default();
        let project = format!("{}/{}", &tail[..user], project.unwrap_or(fallback));
        (project, at + cursor + 1)
    };
    let digits = number_length(&line[number_at..]);
    let end = number_at + digits;
    let after = line[end..].chars().next();
    let free = !before.is_some_and(word) && !after.is_some_and(word);
    (digits > 0 && free).then(|| Reference {
        end,
        url: format!(
            "https://github.com/{project}/issues/{}",
            &line[number_at..end]
        ),
        mention: false,
    })
}

fn reference(line: &str, at: usize, repository: &str) -> Option<Reference> {
    if line[at..].starts_with('@') {
        mention(line, at)
    } else {
        issue(line, at, repository)
    }
}

struct GithubLink<'a> {
    user: &'a str,
    project: &'a str,
    issue: bool,
    reference: &'a str,
    comment: bool,
}

fn github_link(url: &str) -> Option<GithubLink<'_>> {
    let lower = url.to_ascii_lowercase();
    let scheme = ["https://github.com/", "http://github.com/"]
        .into_iter()
        .find(|scheme| lower.starts_with(scheme))?;
    let (path, fragment) = match url[scheme.len()..].split_once('#') {
        Some((path, fragment)) => (path, Some(fragment)),
        None => (&url[scheme.len()..], None),
    };
    let path = path.split_once('?').map_or(path, |(path, _)| path);
    let mut parts = path.splitn(4, '/');
    let (user, project, page, reference) =
        (parts.next()?, parts.next()?, parts.next()?, parts.next()?);
    let reference = reference.strip_suffix('/').unwrap_or(reference);
    let issue = match page.to_ascii_lowercase().as_str() {
        "issues" | "pull" => true,
        "commit" => false,
        _ => return None,
    };
    let valid = if issue {
        !reference.is_empty() && reference.bytes().all(|byte| byte.is_ascii_digit())
    } else {
        (4..=40).contains(&reference.len())
            && reference.bytes().all(|byte| byte.is_ascii_hexdigit())
    };
    let named = !user.is_empty() && user_length(user) == user.len() && !project.is_empty();
    (valid && named).then_some(GithubLink {
        user,
        project,
        issue,
        reference,
        comment: fragment.is_some_and(|fragment| fragment.starts_with("issuecomment-")),
    })
}

impl<'a> Context<'a> {
    fn collect(&mut self, node: &'a Node) {
        match node {
            Node::Definition(definition) => {
                self.definitions
                    .entry(definition.identifier.clone())
                    .or_insert_with(|| definition.url.clone());
            }
            Node::FootnoteDefinition(note) => {
                self.footnotes
                    .entry(note.identifier.clone())
                    .or_insert(&note.children);
            }
            _ => {}
        }
        for child in children(node) {
            self.collect(child);
        }
    }

    fn resolve(&self, url: &str, kind: &str) -> Option<SharedString> {
        if url.is_empty() || url.starts_with('#') {
            return None;
        }
        let resolved = if has_scheme(url) {
            url.to_owned()
        } else if let Some(rest) = url.strip_prefix("//") {
            format!("https://{rest}")
        } else if let Some(rest) = url.strip_prefix('/') {
            format!("https://github.com/{rest}")
        } else {
            format!("https://github.com/{}/{kind}/HEAD/{url}", self.repository)
        };
        openable(&resolved).then(|| resolved.into())
    }

    fn text(&self, line: &str, style: Style, out: &mut Inline) {
        let line = &*emoji(line);
        if style.link.is_some() {
            return out.push(line, style);
        }
        let (mut rest, mut at) = (0, 0);
        while at < line.len() {
            let Some(found) = reference(line, at, self.repository) else {
                at += line[at..].chars().next().map_or(1, char::len_utf8);
                continue;
            };
            out.push(&line[rest..at], style);
            let link = out.link(found.url.into());
            let linked = Style {
                strong: style.strong || found.mention,
                link: Some(link),
                ..style
            };
            out.push(&line[at..found.end], linked);
            rest = found.end;
            at = found.end;
        }
        out.push(&line[rest..], style);
    }

    fn shortened(&self, url: &str, style: Style, out: &mut Inline) {
        let Some(link) = github_link(url) else {
            return out.push(url, style);
        };
        let (user, project) = self.repository.split_once('/').unwrap_or_default();
        let base = if !link.project.eq_ignore_ascii_case(project) {
            format!("{}/{}", link.user, link.project)
        } else if link.user.eq_ignore_ascii_case(user) {
            String::new()
        } else {
            link.user.to_owned()
        };
        let comment = if link.comment { " (comment)" } else { "" };
        if link.issue {
            return out.push(&format!("{base}#{}{comment}", link.reference), style);
        }
        if !base.is_empty() {
            out.push(&format!("{base}@"), style);
        }
        let sha = &link.reference[..SHA_LENGTH.min(link.reference.len())];
        out.push(
            sha,
            Style {
                code: true,
                ..style
            },
        );
        out.push(comment, style);
    }

    fn blocks(&self, nodes: &[Node]) -> Vec<Block> {
        let mut flow = Flow::default();
        for node in nodes {
            match node {
                Node::Html(html) => {
                    for Ok(token) in Tokenizer::new(html.value.as_str()) {
                        let Flow { tags, line, .. } = &mut flow;
                        if let Some(mark) = self.token(token, tags, line) {
                            flow.mark(mark);
                        }
                    }
                }
                Node::Definition(_) | Node::FootnoteDefinition(_) => {}
                other => {
                    flow.close_line();
                    if let Some(block) = self.block(other) {
                        flow.push(block);
                    }
                }
            }
        }
        while !flow.frames.is_empty() {
            flow.close_frame();
        }
        flow.close_line();
        flow.done
    }

    fn block(&self, node: &Node) -> Option<Block> {
        Some(match node {
            Node::Heading(heading) => Block::Heading(heading.depth, self.inline(&heading.children)),
            Node::List(list) => {
                let loose = |item: &Node| matches!(item, Node::ListItem(item) if item.spread);
                let item = |item: &Node| Item {
                    checked: match item {
                        Node::ListItem(item) => item.checked,
                        _ => None,
                    },
                    blocks: self.blocks(children(item)),
                };
                Block::List {
                    start: list.ordered.then(|| list.start.unwrap_or(1)),
                    tight: !list.spread && !list.children.iter().any(loose),
                    items: list.children.iter().map(item).collect(),
                }
            }
            Node::Blockquote(quote) => match alert(&quote.children) {
                Some((kind, body)) => Block::Alert(kind, self.blocks(&body)),
                None => Block::Quote(self.blocks(&quote.children)),
            },
            Node::Code(code) => {
                let source = code.value.replace("\r\n", "\n").replace('\r', "\n");
                Block::Code(source.into())
            }
            Node::ThematicBreak(_) => Block::Rule,
            Node::Table(table) => {
                let side = |kind: &AlignKind| match kind {
                    AlignKind::Center => Align::Center,
                    AlignKind::Right => Align::End,
                    AlignKind::Left | AlignKind::None => Align::Start,
                };
                let row = |row: &Node| {
                    let cells = children(row).iter();
                    cells.map(|cell| self.inline(children(cell))).collect()
                };
                Block::Table {
                    align: table.align.iter().map(side).collect(),
                    rows: table.children.iter().map(row).collect(),
                }
            }
            other => {
                let content = self.inline(std::slice::from_ref(other));
                if content.text.is_empty() {
                    return None;
                }
                Block::Paragraph(content)
            }
        })
    }

    fn inline(&self, nodes: &[Node]) -> Inline {
        let mut out = Inline::default();
        self.spans(nodes, Style::default(), &mut out);
        out
    }

    fn spans(&self, nodes: &[Node], style: Style, out: &mut Inline) {
        let mut tags = Tags {
            base: style,
            ..Tags::default()
        };
        for node in nodes {
            match node {
                Node::Html(html) => {
                    for Ok(token) in Tokenizer::new(html.value.as_str()) {
                        self.token(token, &mut tags, out);
                    }
                }
                _ => self.span(node, tags.style(), out),
            }
        }
    }

    fn token(&self, token: Token, tags: &mut Tags, out: &mut Inline) -> Option<Mark> {
        let named = |bytes: &[u8]| String::from_utf8_lossy(bytes).into_owned();
        match token {
            Token::StartTag(_) | Token::String(_) if tags.dropped.is_some() => None,
            Token::StartTag(tag) => {
                let name = named(&tag.name);
                let attribute = |key: &str| {
                    let value = tag.attributes.get(key.as_bytes());
                    value.map(|value| named(value)).unwrap_or_default()
                };
                let style = tags.style();
                let look = match name.as_str() {
                    "br" => {
                        out.line_break();
                        return None;
                    }
                    "img" => {
                        let width = attribute("width").parse().ok();
                        self.image(&attribute("src"), &attribute("alt"), width, style, out);
                        return None;
                    }
                    "b" | "strong" => Style {
                        strong: true,
                        ..style
                    },
                    "i" | "em" | "cite" | "var" | "dfn" => Style {
                        emphasis: true,
                        ..style
                    },
                    "code" | "kbd" | "mark" | "tt" | "samp" => Style {
                        code: true,
                        ..style
                    },
                    "del" | "s" | "strike" => Style {
                        strike: true,
                        ..style
                    },
                    "ins" | "u" => Style {
                        inserted: true,
                        ..style
                    },
                    "a" => {
                        let target = self.resolve(&attribute("href"), "blob");
                        let link = target.map(|url| out.link(url)).or(style.link);
                        Style { link, ..style }
                    }
                    dropped if DROPPED.contains(&dropped) => {
                        tags.dropped = Some(name);
                        return None;
                    }
                    block if BLOCKS.contains(&block) || heading(block).is_some() => {
                        let open = tag.attributes.contains_key("open".as_bytes());
                        return Some(Mark::Open(name, open));
                    }
                    _ => return None,
                };
                if !tag.self_closing {
                    tags.open.push((name, look));
                }
                None
            }
            Token::EndTag(tag) => {
                let name = named(&tag.name);
                if tags.dropped.is_some() {
                    if tags.dropped.as_deref() == Some(name.as_str()) {
                        tags.dropped = None;
                    }
                    return None;
                }
                if let Some(at) = tags.open.iter().rposition(|(open, _)| *open == name) {
                    tags.open.truncate(at);
                    return None;
                }
                let block = BLOCKS.contains(&name.as_str()) || heading(&name).is_some();
                block.then_some(Mark::Close(name))
            }
            Token::String(text) => {
                self.text(&String::from_utf8_lossy(&text), tags.style(), out);
                None
            }
            _ => None,
        }
    }

    fn image(&self, url: &str, alt: &str, width: Option<f32>, style: Style, out: &mut Inline) {
        let Some(source) = self.resolve(url, "raw") else {
            return;
        };
        let label = if alt.is_empty() {
            source.to_string()
        } else {
            alt.to_owned()
        };
        out.images.push(Picture {
            source: source.clone(),
            width,
        });
        let image = Some(out.images.len() - 1);
        let link = Some(style.link.unwrap_or_else(|| out.link(source)));
        out.push(
            &label,
            Style {
                link,
                image,
                ..style
            },
        );
    }

    fn span(&self, node: &Node, style: Style, out: &mut Inline) {
        let nested = |style: Style, out: &mut Inline| self.spans(children(node), style, out);
        match node {
            Node::Text(text) => {
                for (index, line) in text.value.split('\n').enumerate() {
                    if index > 0 {
                        out.line_break();
                    }
                    self.text(line, style, out);
                }
            }
            Node::InlineCode(code) => out.push(
                &code.value,
                Style {
                    code: true,
                    ..style
                },
            ),
            Node::Break(_) => out.line_break(),
            Node::Strong(_) => nested(
                Style {
                    strong: true,
                    ..style
                },
                out,
            ),
            Node::Emphasis(_) => nested(
                Style {
                    emphasis: true,
                    ..style
                },
                out,
            ),
            Node::Delete(_) => nested(
                Style {
                    strike: true,
                    ..style
                },
                out,
            ),
            Node::Link(link) => {
                let target = self.resolve(&link.url, "blob").map(|url| out.link(url));
                let style = Style {
                    link: target.or(style.link),
                    ..style
                };
                match link.children.as_slice() {
                    [Node::Text(text)] if text.value == link.url => {
                        self.shortened(&link.url, style, out)
                    }
                    _ => nested(style, out),
                }
            }
            Node::LinkReference(reference) => {
                let url = self.definitions.get(&reference.identifier);
                let target = url.and_then(|url| self.resolve(url, "blob"));
                let link = target.map(|url| out.link(url)).or(style.link);
                nested(Style { link, ..style }, out);
            }
            Node::Image(image) => self.image(&image.url, &image.alt, None, style, out),
            Node::ImageReference(reference) => {
                if let Some(url) = self.definitions.get(&reference.identifier) {
                    self.image(url, &reference.alt, None, style, out);
                }
            }
            Node::FootnoteReference(reference) => {
                let name = &reference.identifier;
                let mut cited = self.cited.borrow_mut();
                let known = cited.iter().position(|cited| cited == name);
                let number = known.unwrap_or_else(|| {
                    cited.push(name.clone());
                    cited.len() - 1
                });
                out.push(&format!("[{}]", number + 1), style);
            }
            _ => nested(style, out),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PLAIN: Style = Style {
        strong: false,
        emphasis: false,
        code: false,
        strike: false,
        inserted: false,
        link: None,
        image: None,
    };
    const STRONG: Style = Style {
        strong: true,
        ..PLAIN
    };
    const CODE: Style = Style {
        code: true,
        ..PLAIN
    };

    fn linked(link: usize) -> Style {
        Style {
            link: Some(link),
            ..PLAIN
        }
    }

    fn spans(content: &Inline) -> Vec<(&str, Style)> {
        let mut at = 0;
        let pieces = content.spans.iter().map(|(length, style)| {
            at += length;
            (&content.text[at - length..at], *style)
        });
        pieces.collect()
    }

    fn paragraph(markdown: &str, repository: &str) -> Inline {
        match parse(markdown, repository).into_iter().next() {
            Some(Block::Paragraph(content)) => content,
            other => panic!("paragraph expected, got {other:?}"),
        }
    }

    fn links(content: &Inline) -> Vec<&str> {
        content.links.iter().map(|url| url.as_ref()).collect()
    }

    #[test]
    fn a_release_body_becomes_headings_and_tight_lists() {
        let blocks = parse(
            "### Launcher\r\n\r\n- Added **Arcade** to `x`\r\n- Second\r\n  - Nested\r\n",
            "xeralin/ThrowbackLauncher",
        );
        let [
            Block::Heading(3, title),
            Block::List {
                start,
                tight,
                items,
            },
        ] = blocks.as_slice()
        else {
            panic!("heading and list expected, got {blocks:?}");
        };
        assert_eq!(spans(title), [("Launcher", PLAIN)]);
        assert!(start.is_none() && *tight && items.len() == 2);
        let [Block::Paragraph(first)] = items[0].blocks.as_slice() else {
            panic!("paragraph expected");
        };
        assert_eq!(
            spans(first),
            [
                ("Added ", PLAIN),
                ("Arcade", STRONG),
                (" to ", PLAIN),
                ("x", CODE)
            ]
        );
        assert!(matches!(
            items[1].blocks.as_slice(),
            [Block::Paragraph(_), Block::List { tight: true, .. }]
        ));
    }

    #[test]
    fn blank_lines_between_items_make_a_list_loose_and_numbers_keep_their_start() {
        let blocks = parse("3. one\n\n4. two\n", "");
        let [
            Block::List {
                start,
                tight,
                items,
            },
        ] = blocks.as_slice()
        else {
            panic!("list expected, got {blocks:?}");
        };
        assert_eq!((*start, *tight, items.len()), (Some(3), false, 2));
    }

    #[test]
    fn single_newlines_become_line_breaks_without_spaces_around_them() {
        let content = paragraph("one  \r\ntwo \n  three\\\nfour", "");
        assert_eq!(content.text, "one\ntwo\nthree\nfour");
        assert_eq!(content.spans, [(18, PLAIN)]);
    }

    #[test]
    fn runs_of_spaces_collapse_like_in_a_browser() {
        let content = paragraph("see  the   **two  words**  and `a  b`", "");
        assert_eq!(content.text, "see the two words and a b");
    }

    #[test]
    fn links_resolve_against_the_repository() {
        let link = |markdown: &str| links(&paragraph(markdown, "owner/repo")).join(" ");
        assert_eq!(link("[a](https://example.com/x)"), "https://example.com/x");
        assert_eq!(
            link("[a](docs/x.md)"),
            "https://github.com/owner/repo/blob/HEAD/docs/x.md"
        );
        assert_eq!(link("[a](/owner/other)"), "https://github.com/owner/other");
        assert_eq!(link("[a](//example.com/x)"), "https://example.com/x");
        assert_eq!(link("[a](#top)"), "");
        assert_eq!(link("[a](mailto:x@example.com)"), "");
        assert_eq!(
            link("[a][ref]\n\n[ref]: https://example.com/r"),
            "https://example.com/r"
        );
        let content = paragraph("[a **b**](https://example.com/x) c", "owner/repo");
        assert_eq!(
            spans(&content),
            [
                ("a ", linked(0)),
                (
                    "b",
                    Style {
                        strong: true,
                        ..linked(0)
                    }
                ),
                (" c", PLAIN)
            ]
        );
    }

    #[test]
    fn github_references_become_links() {
        let found = |markdown: &str| {
            let content = paragraph(markdown, "SteamRE/DepotDownloader");
            let texts: Vec<String> = spans(&content)
                .into_iter()
                .filter(|(_, style)| style.link.is_some())
                .map(|(text, _)| text.to_owned())
                .collect();
            (texts.join(" "), links(&content).join(" "))
        };
        let issue =
            |number: &str| format!("https://github.com/SteamRE/DepotDownloader/issues/{number}");
        assert_eq!(
            found("See issue #99 and (#103)"),
            (
                "#99 #103".to_owned(),
                format!("{} {}", issue("99"), issue("103"))
            )
        );
        assert_eq!(
            found("GH-7, a#b, x#0 and color #fff"),
            ("GH-7".to_owned(), issue("7"))
        );
        assert_eq!(found("für ää#1"), ("#1".to_owned(), issue("1")));
        assert_eq!(
            found("thanks @wooorm and a@b"),
            ("@wooorm".to_owned(), "https://github.com/wooorm".to_owned())
        );
        assert_eq!(
            found("dotnet/runtime#44686 fixed 1234567"),
            (
                "dotnet/runtime#44686".to_owned(),
                "https://github.com/dotnet/runtime/issues/44686".to_owned()
            )
        );
        let mention = paragraph("by @wooorm", "SteamRE/DepotDownloader");
        assert!(spans(&mention)[1].1.strong);
        assert!(found("[#99](https://example.com) @ #").1 == "https://example.com");
    }

    #[test]
    fn bare_github_urls_are_shortened() {
        let texts = |markdown: &str| {
            let content = paragraph(markdown, "SteamRE/DepotDownloader");
            let pieces = spans(&content).into_iter();
            pieces
                .map(|(text, style)| (text.to_owned(), style.code, style.link.is_some()))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            texts("see https://github.com/dotnet/runtime/issues/44686"),
            [
                ("see ".to_owned(), false, false),
                ("dotnet/runtime#44686".to_owned(), false, true)
            ]
        );
        assert_eq!(
            texts("https://github.com/SteamRE/DepotDownloader/pull/12#issuecomment-5"),
            [("#12 (comment)".to_owned(), false, true)]
        );
        assert_eq!(
            texts(
                "https://github.com/SteamRE/SteamKit/commit/627a0481a66314151622b09d3da31b7cea6d3940"
            ),
            [
                ("SteamRE/SteamKit@".to_owned(), false, true),
                ("627a048".to_owned(), true, true)
            ]
        );
        assert_eq!(
            texts("https://example.com/a"),
            [("https://example.com/a".to_owned(), false, true)]
        );
        assert_eq!(
            texts("[text](https://github.com/dotnet/runtime/issues/1)"),
            [("text".to_owned(), false, true)]
        );
    }

    #[test]
    fn images_keep_their_source_and_fall_back_to_a_link_with_the_alt_text() {
        let shown = |link: usize, image: usize| Style {
            image: Some(image),
            ..linked(link)
        };
        let content = paragraph(
            "![shot](docs/a.png) and <img src=\"b.png\" alt=\"wide\" width=\"300\">",
            "owner/repo",
        );
        assert_eq!(
            spans(&content),
            [
                ("shot", shown(0, 0)),
                (" and ", PLAIN),
                ("wide", shown(1, 1))
            ]
        );
        let sources = [
            "https://github.com/owner/repo/raw/HEAD/docs/a.png",
            "https://github.com/owner/repo/raw/HEAD/b.png",
        ];
        assert_eq!(links(&content), sources);
        let pictures = [(sources[0], None), (sources[1], Some(300.))];
        let listed = content.images.iter();
        let listed: Vec<_> = listed.map(|image| (&*image.source, image.width)).collect();
        assert_eq!(listed, pictures);
        let content = paragraph(
            "[![](https://example.com/a.png)](https://example.com/page)",
            "owner/repo",
        );
        assert_eq!(
            spans(&content),
            [("https://example.com/a.png", shown(0, 0))]
        );
        assert_eq!(links(&content), ["https://example.com/page"]);
        assert_eq!(content.images[0].source, "https://example.com/a.png");
        let content = paragraph("![shot][pic]\n\n[pic]: docs/a.png", "owner/repo");
        assert_eq!(spans(&content), [("shot", shown(0, 0))]);
        assert_eq!(content.images[0].source, sources[0]);
    }

    #[test]
    fn blocks_keep_their_kind_and_markup_without_text_is_dropped() {
        let blocks = parse(
            "```\r\n  a  b\r\n\r\nc\r\n```\r\n\r\n> quoted\r\n\r\n---\r\n\r\n<br>\r\n\r\n<!-- hidden -->\r\n",
            "",
        );
        let [Block::Code(code), Block::Quote(quote), Block::Rule] = blocks.as_slice() else {
            panic!("unexpected blocks {blocks:?}");
        };
        assert_eq!(code, "  a  b\n\nc");
        assert!(matches!(quote.as_slice(), [Block::Paragraph(_)]));
    }

    #[test]
    fn a_table_keeps_its_cells_and_the_side_of_each_column() {
        let blocks = parse(
            "| a | b | c |\n| :-- | :-: | --: |\n| 1 | **2** | `3` |\n",
            "",
        );
        let [Block::Table { align, rows }] = blocks.as_slice() else {
            panic!("table expected, got {blocks:?}");
        };
        assert_eq!(align, &[Align::Start, Align::Center, Align::End]);
        let cells = |row: &Vec<Inline>| {
            row.iter()
                .map(|cell| cell.text.to_string())
                .collect::<Vec<_>>()
        };
        assert_eq!(cells(&rows[0]), ["a", "b", "c"]);
        assert_eq!(cells(&rows[1]), ["1", "2", "3"]);
        assert_eq!(spans(&rows[1][1]), [("2", STRONG)]);
        assert_eq!(spans(&rows[1][2]), [("3", CODE)]);
    }

    #[test]
    fn task_items_carry_their_state() {
        let blocks = parse("- [x] done\n- [ ] open\n- plain\n", "");
        let [Block::List { items, .. }] = blocks.as_slice() else {
            panic!("list expected, got {blocks:?}");
        };
        let states: Vec<Option<bool>> = items.iter().map(|item| item.checked).collect();
        assert_eq!(states, [Some(true), Some(false), None]);
        let [Block::Paragraph(first)] = items[0].blocks.as_slice() else {
            panic!("paragraph expected");
        };
        assert_eq!(first.text, "done");
    }

    #[test]
    fn a_marked_quote_becomes_an_alert_without_its_marker() {
        let kind = |markdown: &str| match parse(markdown, "").as_slice() {
            [Block::Alert(kind, body)] => {
                let [Block::Paragraph(first), ..] = body.as_slice() else {
                    panic!("paragraph expected");
                };
                Some((*kind, first.text.to_string(), body.len()))
            }
            _ => None,
        };
        assert_eq!(
            kind("> [!NOTE]\n> Useful **text**\n>\n> more"),
            Some((Alert::Note, "Useful text".to_owned(), 2))
        );
        assert_eq!(
            kind("> [!warning]  \n> careful"),
            Some((Alert::Warning, "careful".to_owned(), 1))
        );
        assert_eq!(
            kind("> [!CAUTION]\n>\n> apart"),
            Some((Alert::Caution, "apart".to_owned(), 1))
        );
        let tips = ["TIP", "IMPORTANT"].map(|name| kind(&format!("> [!{name}]\n> x")));
        let kinds = tips.map(|found| found.map(|(kind, ..)| kind));
        assert_eq!(kinds, [Some(Alert::Tip), Some(Alert::Important)]);
        assert_eq!(kind("> [!NOTE]"), None);
        assert_eq!(kind("> [!OTHER]\n> x"), None);
        assert_eq!(kind("> text [!NOTE]"), None);
    }

    #[test]
    fn footnotes_are_numbered_by_first_use_and_listed_at_the_end() {
        let listed = |body: &str| {
            let blocks = parse(body, "");
            let [Block::Paragraph(content), Block::Footnotes(notes)] = blocks.as_slice() else {
                panic!("paragraph and footnotes expected, got {blocks:?}");
            };
            let notes: Vec<String> = notes
                .iter()
                .map(|note| match note.as_slice() {
                    [Block::Paragraph(content)] => content.text.clone(),
                    other => panic!("paragraph expected, got {other:?}"),
                })
                .collect();
            (content.text.clone(), notes)
        };
        let (text, notes) =
            listed("one[^b] two[^a] again[^b]\n\n[^a]: first *note*\n[^b]: second\n[^c]: unused\n");
        assert_eq!(text, "one[1] two[2] again[1]");
        assert_eq!(notes, ["second", "first note"]);
        let (text, notes) = listed("one[^a]\n\n[^a]: first[^b]\n[^b]: second\n");
        assert_eq!(text, "one[1]");
        assert_eq!(notes, ["first[2]", "second"]);
    }

    #[test]
    fn html_tags_style_the_text_between_them() {
        let content = paragraph(
            "<kbd>Ctrl</kbd> H<sub>2</sub>O <ins>new</ins> <del>old</del> <b>b</b><br><i>i</i> <mark>m</mark> <a href=\"https://example.com/x\">x</a>",
            "",
        );
        assert_eq!(content.text, "Ctrl H2O new old b\ni m x");
        let looks: Vec<(&str, Style)> = spans(&content)
            .into_iter()
            .filter(|(_, style)| *style != PLAIN)
            .collect();
        let inserted = Style {
            inserted: true,
            ..PLAIN
        };
        let struck = Style {
            strike: true,
            ..PLAIN
        };
        let slanted = Style {
            emphasis: true,
            ..PLAIN
        };
        assert_eq!(
            looks,
            [
                ("Ctrl", CODE),
                ("new", inserted),
                ("old", struck),
                ("b", STRONG),
                ("i", slanted),
                ("m", CODE),
                ("x", linked(0))
            ]
        );
        assert_eq!(links(&content), ["https://example.com/x"]);
    }

    #[test]
    fn details_fold_the_blocks_between_their_tags() {
        let blocks = parse(
            "<details open>\n<summary>More <b>here</b></summary>\n\n- one\n\n<details><summary>Inner</summary>text</details>\n\n</details>\n\nafter",
            "",
        );
        let [
            Block::Details {
                summary,
                open,
                blocks: inside,
            },
            Block::Paragraph(after),
        ] = blocks.as_slice()
        else {
            panic!("details and paragraph expected, got {blocks:?}");
        };
        assert_eq!(spans(summary), [("More ", PLAIN), ("here", STRONG)]);
        assert!(*open && after.text == "after");
        let [
            Block::List { .. },
            Block::Details {
                summary,
                open,
                blocks: inner,
            },
        ] = inside.as_slice()
        else {
            panic!("list and details expected, got {inside:?}");
        };
        assert!(summary.text == "Inner" && !*open);
        assert!(matches!(inner.as_slice(), [Block::Paragraph(text)] if text.text == "text"));
    }

    #[test]
    fn html_blocks_become_headings_rules_and_paragraphs() {
        let blocks = parse(
            "<h3>Title</h3>\n<p align=\"center\">Centered &amp; <em>kept</em></p>\n<hr>\n<script>alert(1)</script>\n<div>last</div>",
            "",
        );
        let [
            Block::Heading(3, title),
            Block::Paragraph(centered),
            Block::Rule,
            Block::Paragraph(last),
        ] = blocks.as_slice()
        else {
            panic!("unexpected blocks {blocks:?}");
        };
        assert_eq!(title.text, "Title");
        assert_eq!(centered.text, "Centered & kept");
        assert_eq!(last.text, "last");
    }

    #[test]
    fn short_codes_become_emoji_outside_of_code() {
        let content = paragraph(
            ":rocket: 10:30 :not-an-emoji: `:tada:` [:tada:](https://example.com)",
            "",
        );
        assert_eq!(
            content.text,
            "\u{1f680} 10:30 :not-an-emoji: :tada: \u{1f389}"
        );
    }

    #[test]
    fn nested_emphasis_keeps_both_styles() {
        let content = paragraph("*thanks to **aidn** :3* ~~gone~~", "");
        let styles: Vec<(bool, bool, bool)> = spans(&content)
            .into_iter()
            .map(|(_, style)| (style.emphasis, style.strong, style.strike))
            .collect();
        assert_eq!(
            styles,
            [
                (true, false, false),
                (true, true, false),
                (true, false, false),
                (false, false, false),
                (false, false, true)
            ]
        );
    }
}
