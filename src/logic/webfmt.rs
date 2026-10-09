//! Beautify and minify HTML, CSS, JavaScript and TypeScript.
//!
//! JS / TS go through a real parser (dprint to format, swc to print compact),
//! CSS is formatted by malva, and HTML by a lenient formatter of our own:
//! pasted markup is often a fragment with unclosed tags, which strict
//! formatters refuse.

use std::path::Path;

use deno_ast::swc::codegen::{Config, Emitter, text_writer::JsWriter};
use deno_ast::swc::common::{SourceMap, sync::Lrc};
use deno_ast::{MediaType, ModuleSpecifier, ParseParams};
use dprint_plugin_typescript::configuration::{ConfigurationBuilder, UseBraces};

pub const LANGS: &[&str] = &["HTML", "CSS", "JavaScript", "TypeScript"];
/// The editor grammar for each of `LANGS`.
pub const GRAMMARS: &[&str] = &["html", "css", "javascript", "typescript"];
pub const INDENTS: &[&str] = &["2 spaces", "4 spaces", "Tab"];

const WIDTH: usize = 100;

#[derive(Clone, Copy)]
pub struct Opts {
    pub minify: bool,
    /// Indexes `INDENTS`.
    pub indent: usize,
}

impl Opts {
    fn unit(&self) -> &'static str {
        ["  ", "    ", "\t"][self.indent.min(2)]
    }
}

// ---------------------------------------------------------------- JS / TS

/// "SyntaxError: Expression expected\n  |\n1 | …\n    at file:///x.js:1:11"
/// → "Expression expected (line 1, column 11)".
fn tidy_error(e: &str) -> String {
    let first = e.lines().next().unwrap_or("").trim();
    let message = first.strip_prefix("SyntaxError: ").unwrap_or(first);
    let place = e.lines().rev().find_map(|l| {
        let mut parts = l.trim().rsplitn(3, ':');
        let (col, line) = (parts.next()?.parse::<usize>().ok()?, parts.next()?.parse::<usize>().ok()?);
        Some(format!(" (line {line}, column {col})"))
    });
    format!("{message}{}", place.unwrap_or_default())
}

fn script_pretty(src: &str, ext: &str, o: Opts) -> Result<String, String> {
    let config = ConfigurationBuilder::new()
        .line_width(WIDTH as u32)
        .indent_width(if o.indent == 0 { 2 } else { 4 })
        .use_tabs(o.indent == 2)
        // A formatter should not add or drop braces.
        .use_braces(UseBraces::Maintain)
        .build();
    let name = format!("snippet.{ext}");
    dprint_plugin_typescript::format_text(dprint_plugin_typescript::FormatTextOptions {
        path: Path::new(&name),
        extension: Some(ext),
        text: src.to_string(),
        config: &config,
        external_formatter: None,
    })
    .map(|formatted| formatted.unwrap_or_else(|| src.to_string()).trim_end().to_string())
    .map_err(|e| tidy_error(&e.to_string()))
}

fn script_compact(src: &str, media_type: MediaType) -> Result<String, String> {
    let parsed = deno_ast::parse_program(ParseParams {
        specifier: ModuleSpecifier::parse("file:///snippet").map_err(|e| e.to_string())?,
        text: src.into(),
        media_type,
        capture_tokens: false,
        scope_analysis: false,
        maybe_syntax: None,
    })
    .map_err(|e| tidy_error(&e.to_string()))?;
    let cm = Lrc::new(SourceMap::default());
    let mut buf = Vec::new();
    {
        let wr = JsWriter::new(cm.clone(), "\n", &mut buf, None);
        let mut emitter = Emitter { cfg: Config::default().with_minify(true), cm, comments: None, wr };
        emitter.emit_program(&parsed.program()).map_err(|e| e.to_string())?;
    }
    String::from_utf8(buf).map_err(|e| e.to_string())
}

/// JavaScript (JSX allowed) or TypeScript. TypeScript is first read as plain
/// `.ts`, where `<T>x` is a cast, and as `.tsx` only if that fails.
fn script(src: &str, ts: bool, o: Opts) -> Result<String, String> {
    let run = |tsx: bool| match (o.minify, ts, tsx) {
        (false, false, _) => script_pretty(src, "jsx", o),
        (false, true, false) => script_pretty(src, "ts", o),
        (false, true, true) => script_pretty(src, "tsx", o),
        (true, false, _) => script_compact(src, MediaType::Jsx),
        (true, true, false) => script_compact(src, MediaType::TypeScript),
        (true, true, true) => script_compact(src, MediaType::Tsx),
    };
    match run(false) {
        Err(first) if ts => run(true).map_err(|_| first),
        other => other,
    }
}

// ---------------------------------------------------------------- CSS

fn css_pretty(src: &str, o: Opts) -> Result<String, String> {
    let options = malva::config::FormatOptions {
        layout: malva::config::LayoutOptions { print_width: WIDTH, use_tabs: o.indent == 2, indent_width: if o.indent == 0 { 2 } else { 4 }, ..Default::default() },
        ..Default::default()
    };
    malva::format_text(src, malva::Syntax::Css, &options).map(|s| s.trim_end().to_string()).map_err(|e| {
        let e = e.to_string();
        let mut c = e.chars();
        c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or(e)
    })
}

/// Drop comments and the whitespace CSS does not need. Deliberately
/// conservative: spaces that can change meaning (around `+` in `calc()`,
/// before `:` or `(` in selectors and media queries) are kept.
fn css_compact(src: &str) -> String {
    let chars: Vec<char> = src.chars().collect();
    let mut out = String::with_capacity(src.len());
    let mut gap = false;
    let mut depth = 0usize;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            gap = true;
            i += 1;
            continue;
        }
        if c == '/' && chars.get(i + 1) == Some(&'*') {
            let end = (i + 2..chars.len().saturating_sub(1)).find(|j| chars[*j] == '*' && chars[j + 1] == '/').map(|j| j + 2).unwrap_or(chars.len());
            // `/*! … */` marks a licence, which has to stay.
            if chars.get(i + 2) == Some(&'!') {
                out.extend(&chars[i..end]);
            } else {
                gap = true;
            }
            i = end;
            continue;
        }
        if gap {
            let last = out.chars().next_back();
            // `a :hover {` needs its space, `color : red;` does not.
            let property_colon = c == ':' && depth > 0 && ends_declaration(&chars, i);
            let keep = last.is_some_and(|l| !"{};,(:".contains(l)) && !"{};,)!".contains(c) && !out.ends_with("*/") && !property_colon;
            if keep {
                out.push(' ');
            }
            gap = false;
        }
        match c {
            '"' | '\'' => {
                let mut j = i + 1;
                while j < chars.len() && chars[j] != c {
                    j += if chars[j] == '\\' { 2 } else { 1 };
                }
                let end = (j + 1).min(chars.len());
                out.extend(&chars[i..end]);
                i = end;
                continue;
            }
            '}' => {
                // The last declaration needs no semicolon.
                if out.ends_with(';') {
                    out.pop();
                }
                depth = depth.saturating_sub(1);
                out.push('}');
            }
            '{' => {
                depth += 1;
                out.push('{');
            }
            other => out.push(other),
        }
        i += 1;
    }
    out
}

/// Whether the text from `from` reaches a `;` or `}` before any `{`: that is a
/// declaration, where a selector would run into its block first.
fn ends_declaration(chars: &[char], from: usize) -> bool {
    let mut i = from;
    while i < chars.len() {
        match chars[i] {
            ';' | '}' => return true,
            '{' => return false,
            q @ ('"' | '\'') => {
                i += 1;
                while i < chars.len() && chars[i] != q {
                    i += if chars[i] == '\\' { 2 } else { 1 };
                }
            }
            _ => {}
        }
        i += 1;
    }
    true
}

fn css(src: &str, o: Opts) -> Result<String, String> {
    // Parsing first means broken CSS is reported in either mode.
    let pretty = css_pretty(src, o)?;
    Ok(if o.minify { css_compact(src) } else { pretty })
}

// ---------------------------------------------------------------- HTML

const VOID: &[&str] = &["area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source", "track", "wbr"];

/// Elements that sit in a line of text; whitespace around them can show.
const INLINE: &[&str] = &[
    "a", "abbr", "b", "bdi", "bdo", "br", "button", "cite", "code", "data", "del", "dfn", "em", "i", "img", "input", "ins", "kbd",
    "label", "mark", "q", "s", "samp", "select", "small", "span", "strong", "sub", "sup", "textarea", "time", "u", "var", "wbr",
];

/// Elements whose content is kept exactly as written.
const RAW: &[&str] = &["script", "style", "textarea", "pre"];

/// Start tags that end an open `<p>`.
const ENDS_P: &[&str] = &[
    "address", "article", "aside", "blockquote", "details", "div", "dl", "fieldset", "figcaption", "figure", "footer", "form", "h1",
    "h2", "h3", "h4", "h5", "h6", "header", "hr", "main", "menu", "nav", "ol", "p", "pre", "section", "table", "ul",
];

enum Tok<'a> {
    /// `<!doctype …>`, `<?xml …?>`.
    Decl(&'a str),
    Comment(&'a str),
    Open { name: String, tag: String, closed: bool },
    Close { name: String, raw: &'a str },
    Text(&'a str),
    Raw { name: String, tag: String, content: &'a str, end: &'a str },
}

/// Where the tag starting at `start` ends, skipping `>` inside quotes.
fn tag_end(s: &str, start: usize) -> Option<usize> {
    let mut quote = None;
    for (i, c) in s[start..].char_indices() {
        match (quote, c) {
            (None, '"' | '\'') => quote = Some(c),
            (Some(q), _) if q == c => quote = None,
            (None, '>') => return Some(start + i + 1),
            _ => {}
        }
    }
    None
}

/// A start tag with the whitespace between its attributes squeezed to one space.
fn tidy_tag(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut quote = None;
    let mut gap = false;
    for c in raw.chars() {
        match (quote, c) {
            (None, c) if c.is_whitespace() => gap = true,
            _ => {
                if gap && c != '>' {
                    out.push(' ');
                }
                gap = false;
                match (quote, c) {
                    (None, '"' | '\'') => quote = Some(c),
                    (Some(q), _) if q == c => quote = None,
                    _ => {}
                }
                out.push(c);
            }
        }
    }
    out
}

fn tokens<'a>(s: &'a str) -> Vec<Tok<'a>> {
    let mut out = Vec::new();
    let mut pos = 0;
    let mut text_from = 0;
    let lower = s.to_ascii_lowercase();
    while let Some(found) = s[pos..].find('<') {
        let at = pos + found;
        let rest = &s[at..];
        let next = rest[1..].chars().next();
        let flush = |out: &mut Vec<Tok<'a>>, upto: usize| {
            if upto > text_from {
                out.push(Tok::Text(&s[text_from..upto]));
            }
        };
        if rest.starts_with("<!--") {
            let end = s[at + 4..].find("-->").map(|i| at + 4 + i + 3).unwrap_or(s.len());
            flush(&mut out, at);
            out.push(Tok::Comment(&s[at..end]));
            pos = end;
        } else if rest.starts_with("<![CDATA[") {
            let end = s[at..].find("]]>").map(|i| at + i + 3).unwrap_or(s.len());
            flush(&mut out, at);
            out.push(Tok::Comment(&s[at..end]));
            pos = end;
        } else if matches!(next, Some('!' | '?')) {
            let Some(end) = tag_end(s, at) else { break };
            flush(&mut out, at);
            out.push(Tok::Decl(&s[at..end]));
            pos = end;
        } else if next == Some('/') && rest[2..].starts_with(|c: char| c.is_ascii_alphabetic()) {
            let Some(end) = tag_end(s, at) else { break };
            flush(&mut out, at);
            let name: String = lower[at + 2..end - 1].chars().take_while(|c| !c.is_whitespace() && *c != '>').collect();
            out.push(Tok::Close { name, raw: &s[at..end] });
            pos = end;
        } else if next.is_some_and(|c| c.is_ascii_alphabetic()) {
            let Some(end) = tag_end(s, at) else { break };
            flush(&mut out, at);
            let name: String = lower[at + 1..end - 1].chars().take_while(|c| !c.is_whitespace() && *c != '/' && *c != '>').collect();
            let closed = s[..end - 1].ends_with('/');
            let tag = tidy_tag(&s[at..end]);
            if RAW.contains(&name.as_str()) && !closed {
                // Everything up to the matching end tag belongs to the element as is.
                let close = format!("</{name}");
                let (content_end, tail) = match lower[end..].find(&close) {
                    Some(i) => (end + i, tag_end(s, end + i).unwrap_or(s.len())),
                    None => (s.len(), s.len()),
                };
                out.push(Tok::Raw { name, tag, content: &s[end..content_end], end: &s[content_end..tail] });
                pos = tail;
            } else {
                out.push(Tok::Open { name, tag, closed });
                pos = end;
            }
        } else {
            // A lone `<` is just text.
            pos = at + 1;
            continue;
        }
        text_from = pos;
    }
    if text_from < s.len() {
        out.push(Tok::Text(&s[text_from..]));
    }
    out
}

enum Node<'a> {
    Decl(&'a str),
    Comment(&'a str),
    Text(&'a str),
    Raw { name: String, tag: String, content: &'a str, end: &'a str },
    Element { name: String, tag: String, children: Vec<Node<'a>>, end: Option<&'a str> },
}

struct Frame<'a> {
    name: String,
    tag: String,
    children: Vec<Node<'a>>,
}

/// Pop frames down to (and including) index `to`, nesting each in its parent.
fn close_to<'a>(stack: &mut Vec<Frame<'a>>, to: usize, end: Option<&'a str>) {
    while stack.len() > to {
        let f = stack.pop().unwrap();
        // Only the frame the end tag names gets it; the rest ended implicitly.
        let end = if stack.len() == to { end } else { None };
        stack.last_mut().unwrap().children.push(Node::Element { name: f.name, tag: f.tag, children: f.children, end });
    }
}

/// The open element a new `name` start tag implicitly ends, if any.
fn implied_end(stack: &[Frame<'_>], name: &str) -> Option<usize> {
    let (targets, walls): (&[&str], &[&str]) = match name {
        "li" => (&["li"], &["ul", "ol", "menu"]),
        "dt" | "dd" => (&["dt", "dd"], &["dl"]),
        "tr" => (&["tr"], &["table", "thead", "tbody", "tfoot"]),
        "td" | "th" => (&["td", "th"], &["tr", "table"]),
        "thead" | "tbody" | "tfoot" => (&["thead", "tbody", "tfoot"], &["table"]),
        "option" => (&["option"], &["select", "datalist", "optgroup"]),
        "optgroup" => (&["optgroup"], &["select"]),
        _ if ENDS_P.contains(&name) => {
            // A paragraph ends only if nothing but inline elements is open inside it.
            let i = stack.iter().rposition(|f| !INLINE.contains(&f.name.as_str()))?;
            return (i > 0 && stack[i].name == "p").then_some(i);
        }
        _ => return None,
    };
    for (i, f) in stack.iter().enumerate().skip(1).rev() {
        if targets.contains(&f.name.as_str()) {
            return Some(i);
        }
        if walls.contains(&f.name.as_str()) {
            return None;
        }
    }
    None
}

fn tree(s: &str) -> Vec<Node<'_>> {
    // The bottom frame is the document itself.
    let mut stack = vec![Frame { name: String::new(), tag: String::new(), children: Vec::new() }];
    for tok in tokens(s) {
        match tok {
            Tok::Decl(d) => stack.last_mut().unwrap().children.push(Node::Decl(d)),
            Tok::Comment(c) => stack.last_mut().unwrap().children.push(Node::Comment(c)),
            Tok::Text(t) => stack.last_mut().unwrap().children.push(Node::Text(t)),
            Tok::Raw { name, tag, content, end } => {
                if let Some(i) = implied_end(&stack, &name) {
                    close_to(&mut stack, i, None);
                }
                stack.last_mut().unwrap().children.push(Node::Raw { name, tag, content, end });
            }
            Tok::Open { name, tag, closed } => {
                if let Some(i) = implied_end(&stack, &name) {
                    close_to(&mut stack, i, None);
                }
                if closed || VOID.contains(&name.as_str()) {
                    stack.last_mut().unwrap().children.push(Node::Element { name, tag, children: Vec::new(), end: None });
                } else {
                    stack.push(Frame { name, tag, children: Vec::new() });
                }
            }
            Tok::Close { name, raw } => match stack.iter().rposition(|f| f.name == name).filter(|i| *i > 0) {
                Some(i) => close_to(&mut stack, i, Some(raw)),
                // An end tag with nothing to end: keep it rather than lose it.
                None => stack.last_mut().unwrap().children.push(Node::Text(raw)),
            },
        }
    }
    close_to(&mut stack, 1, None);
    stack.pop().unwrap().children
}

fn inline(n: &Node) -> bool {
    match n {
        Node::Text(_) | Node::Comment(_) => true,
        Node::Decl(_) => false,
        Node::Raw { name, .. } => name == "textarea",
        Node::Element { name, children, .. } => INLINE.contains(&name.as_str()) && children.iter().all(inline),
    }
}

struct Html {
    opts: Opts,
    lines: Vec<String>,
    notes: Vec<String>,
}

/// An inline run as chunks; a line may break between chunks and nowhere else.
/// `glue` says whether the next piece continues the last chunk.
fn chunks(nodes: &[Node], out: &mut Vec<String>, glue: &mut bool) {
    let atom = |text: &str, out: &mut Vec<String>, glue: &mut bool| {
        if *glue && !out.is_empty() {
            out.last_mut().unwrap().push_str(text);
        } else {
            out.push(text.to_string());
        }
        *glue = true;
    };
    for n in nodes {
        match n {
            Node::Text(t) => {
                if t.starts_with(char::is_whitespace) {
                    *glue = false;
                }
                let mut words = t.split_whitespace().peekable();
                while let Some(w) = words.next() {
                    atom(w, out, glue);
                    if words.peek().is_some() {
                        *glue = false;
                    }
                }
                if t.ends_with(char::is_whitespace) {
                    *glue = false;
                }
            }
            Node::Comment(c) | Node::Decl(c) => atom(c, out, glue),
            Node::Raw { tag, content, end, .. } => atom(&format!("{tag}{content}{end}"), out, glue),
            Node::Element { tag, children, end, .. } => {
                atom(tag, out, glue);
                chunks(children, out, glue);
                if let Some(e) = end {
                    atom(e, out, glue);
                }
            }
        }
    }
}

impl Html {
    fn pad(&self, depth: usize) -> String {
        self.opts.unit().repeat(depth)
    }

    /// Lay chunks out at `depth`, breaking lines only where the source had whitespace.
    fn flow(&mut self, parts: &[String], depth: usize) {
        let pad = self.pad(depth);
        let room = WIDTH.saturating_sub(pad.chars().count()).max(40);
        let mut line = String::new();
        for p in parts {
            if !line.is_empty() && line.chars().count() + 1 + p.chars().count() > room {
                self.lines.push(format!("{pad}{line}"));
                line.clear();
            }
            if !line.is_empty() {
                line.push(' ');
            }
            line.push_str(p);
        }
        if !line.is_empty() {
            self.lines.push(format!("{pad}{line}"));
        }
    }

    fn children(&mut self, nodes: &[Node], depth: usize) {
        let mut i = 0;
        while i < nodes.len() {
            if inline(&nodes[i]) {
                let end = (i..nodes.len()).find(|j| !inline(&nodes[*j])).unwrap_or(nodes.len());
                let (mut parts, mut glue) = (Vec::new(), false);
                chunks(&nodes[i..end], &mut parts, &mut glue);
                self.flow(&parts, depth);
                i = end;
            } else {
                self.block(&nodes[i], depth);
                i += 1;
            }
        }
    }

    fn block(&mut self, n: &Node, depth: usize) {
        let pad = self.pad(depth);
        match n {
            Node::Decl(d) => self.lines.push(format!("{pad}{d}")),
            Node::Raw { name, tag, content, end } if name == "pre" => self.lines.push(format!("{pad}{tag}{content}{end}")),
            Node::Raw { name, tag, content, end } => {
                let body = self.embedded(name, tag, content);
                if body.trim().is_empty() {
                    self.lines.push(format!("{pad}{tag}{end}"));
                    return;
                }
                self.lines.push(format!("{pad}{tag}"));
                // Indenting a template literal would change the string it holds.
                let inner = if name == "script" && body.contains('`') { String::new() } else { self.pad(depth + 1) };
                self.lines.extend(body.lines().map(|l| if l.trim().is_empty() { String::new() } else { format!("{inner}{l}") }));
                if !end.is_empty() {
                    self.lines.push(format!("{pad}{end}"));
                }
            }
            Node::Element { tag, children, end, .. } => {
                let end = end.unwrap_or("");
                if children.iter().all(inline) {
                    let (mut parts, mut glue) = (Vec::new(), false);
                    chunks(children, &mut parts, &mut glue);
                    let flat = parts.join(" ");
                    let one_line = format!("{pad}{tag}{flat}{end}");
                    if one_line.chars().count() <= WIDTH && !flat.contains('\n') {
                        self.lines.push(one_line);
                        return;
                    }
                    self.lines.push(format!("{pad}{tag}"));
                    self.flow(&parts, depth + 1);
                } else {
                    self.lines.push(format!("{pad}{tag}"));
                    self.children(children, depth + 1);
                }
                if !end.is_empty() {
                    self.lines.push(format!("{pad}{end}"));
                }
            }
            Node::Text(_) | Node::Comment(_) => {}
        }
    }

    /// The content of a `<script>` or `<style>`, formatted when it is a
    /// language we know and parses; otherwise as written.
    fn embedded(&mut self, name: &str, tag: &str, content: &str) -> String {
        let kind = tag.to_ascii_lowercase();
        let as_written = || content.trim_matches(['\n', '\r']).trim_end().to_string();
        let result = if name == "style" {
            css(content, self.opts)
        } else if kind.contains("json") || kind.contains("importmap") {
            serde_json::from_str::<serde_json::Value>(content)
                .map(|v| super::to_json(&v, &if self.opts.minify { super::Indent::Minified } else { super::Indent::Spaces(self.opts.unit().len().max(2)) }))
                .map_err(|e| e.to_string())
        } else if !kind.contains("type=") || kind.contains("javascript") || kind.contains("module") || kind.contains("ecmascript") {
            // Template literals keep their own layout, so such scripts are left alone when beautifying.
            if !self.opts.minify && content.contains('`') { Ok(as_written()) } else { script(content, false, self.opts) }
        } else if kind.contains("typescript") {
            script(content, true, self.opts)
        } else {
            // A template or data block of some other type.
            return as_written();
        };
        result.unwrap_or_else(|_| {
            self.notes.push(format!("A <{name}> block could not be parsed and was left as written"));
            as_written()
        })
    }
}

fn html_pretty(src: &str, o: Opts) -> (String, Vec<String>) {
    let nodes = tree(src);
    let mut h = Html { opts: o, lines: Vec::new(), notes: Vec::new() };
    h.children(&nodes, 0);
    (h.lines.join("\n"), h.notes)
}

/// Whether whitespace next to this tag is invisible in the rendered page.
fn breaks_line(name: &str) -> bool {
    !INLINE.contains(&name)
}

fn html_compact(src: &str, o: Opts) -> (String, Vec<String>) {
    let mut h = Html { opts: o, lines: Vec::new(), notes: Vec::new() };
    let mut out = String::with_capacity(src.len());
    // Whitespace was seen and has not been written yet.
    let mut gap = false;
    // Nothing inline has been written since the last line-breaking tag.
    let mut fresh = true;
    // Write `text` as inline content, with the pending space if it can show.
    let inline = |out: &mut String, gap: &mut bool, fresh: &mut bool, text: &str| {
        if *gap && !*fresh {
            out.push(' ');
        }
        (*gap, *fresh) = (false, false);
        out.push_str(text);
    };
    for t in tokens(src) {
        match t {
            // Comments go, except the conditional ones old Internet Explorer reads.
            Tok::Comment(c) => {
                if c.starts_with("<!--[if") || c.starts_with("<![CDATA[") {
                    inline(&mut out, &mut gap, &mut fresh, c);
                }
            }
            Tok::Decl(d) => {
                out.push_str(d);
                (gap, fresh) = (false, true);
            }
            Tok::Open { name, tag, .. } if breaks_line(&name) => {
                out.push_str(&tag);
                (gap, fresh) = (false, true);
            }
            Tok::Close { name, raw } if breaks_line(&name) => {
                out.push_str(raw);
                (gap, fresh) = (false, true);
            }
            Tok::Open { tag, .. } => inline(&mut out, &mut gap, &mut fresh, &tag),
            Tok::Close { raw, .. } => inline(&mut out, &mut gap, &mut fresh, raw),
            Tok::Raw { name, tag, content, end } => match name.as_str() {
                "pre" => {
                    out.push_str(&format!("{tag}{content}{end}"));
                    (gap, fresh) = (false, true);
                }
                "textarea" => inline(&mut out, &mut gap, &mut fresh, &format!("{tag}{content}{end}")),
                // <script> and <style> draw nothing, so the spacing around them carries over.
                _ => out.push_str(&format!("{tag}{}{end}", h.embedded(&name, &tag, content).trim())),
            },
            Tok::Text(text) => {
                gap |= text.starts_with(char::is_whitespace);
                let mut words = text.split_whitespace();
                if let Some(first) = words.next() {
                    inline(&mut out, &mut gap, &mut fresh, first);
                    for w in words {
                        out.push(' ');
                        out.push_str(w);
                    }
                    gap = text.ends_with(char::is_whitespace);
                }
            }
        }
    }
    (out, h.notes)
}

/// Format `src` as `LANGS[lang]`. Returns the result and notes about anything
/// that was left as written.
pub fn format(src: &str, lang: usize, opts: Opts) -> Result<(String, Vec<String>), String> {
    match lang {
        0 => Ok(if opts.minify { html_compact(src, opts) } else { html_pretty(src, opts) }),
        1 => css(src, opts).map(|s| (s, Vec::new())),
        2 => script(src, false, opts).map(|s| (s, Vec::new())),
        _ => script(src, true, opts).map(|s| (s, Vec::new())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PRETTY: Opts = Opts { minify: false, indent: 0 };
    const MIN: Opts = Opts { minify: true, indent: 0 };

    fn fmt(src: &str, lang: usize, o: Opts) -> String {
        format(src, lang, o).unwrap().0
    }

    #[test]
    fn javascript() {
        let src = "// keep me\nimport {a,b} from \"x\"; // and me\nfunction f(x /* arg */, y){ if (x) return y\n  const o={a:1,b:[1,2,3]};\n  return x?.y ?? o.a /* inside */ + 1;\n}\nconst App = () => <div className=\"a\">{items.map(i => <Item key={i.id} />)}</div>;\n";
        assert_eq!(
            fmt(src, 2, PRETTY),
            "// keep me\nimport { a, b } from \"x\"; // and me\nfunction f(x, /* arg */ y) {\n  if (x) return y;\n  const o = { a: 1, b: [1, 2, 3] };\n  return x?.y ?? o.a /* inside */ + 1;\n}\nconst App = () => <div className=\"a\">{items.map(i => <Item key={i.id} />)}</div>;"
        );
        let four = fmt("function f(){return 1}", 2, Opts { minify: false, indent: 1 });
        assert_eq!(four, "function f() {\n    return 1;\n}");
        assert_eq!(fmt("function f(){return 1}", 2, Opts { minify: false, indent: 2 }), "function f() {\n\treturn 1;\n}");
        assert_eq!(fmt(src, 2, MIN), "import{a,b}from\"x\";function f(x,y){if(x)return y;const o={a:1,b:[1,2,3]};return x?.y??o.a+1;}const App=()=><div className=\"a\">{items.map(i=><Item key={i.id}/>)}</div>;");
        assert_eq!(format("const a = ;\n", 2, PRETTY).unwrap_err(), "Expression expected (line 1, column 11)");
        assert!(format("const a = ;\n", 2, MIN).unwrap_err().starts_with("Expression expected"));
    }

    #[test]
    fn typescript() {
        let src = "interface User<T = string> { id: number; name?: T }\nexport function pick<T, K extends keyof T>(o: T, ...k: K[]): Pick<T, K> { return o as any }\nlet c = <string>z;\nenum Color { Red = \"RED\" }\n";
        assert_eq!(
            fmt(src, 3, PRETTY),
            "interface User<T = string> {\n  id: number;\n  name?: T;\n}\nexport function pick<T, K extends keyof T>(o: T, ...k: K[]): Pick<T, K> {\n  return o as any;\n}\nlet c = <string> z;\nenum Color {\n  Red = \"RED\",\n}"
        );
        let min = fmt(src, 3, MIN);
        assert!(min.starts_with("interface User<T=string>{id:number;name?:T;}export function pick<T,K extends keyof T>(o:T,...k:K[]):Pick<T,K>{return o as any;}"), "{min}");
        // TSX is tried once plain TypeScript fails.
        assert!(fmt("const a = <T,>(v: T) => <b>{v}</b>;", 3, PRETTY).contains("<b>{v}</b>"));
        assert!(format("let x: = 1", 3, PRETTY).is_err());
        // Type annotations are not JavaScript.
        assert!(format("let x: number = 1", 2, PRETTY).is_err());
    }

    #[test]
    fn stylesheets() {
        let src = "@import url(\"a.css\");/* note */a:hover,a :focus>b{color:red;margin:0 auto!important}@media (min-width:100px) and (max-width:calc(100% - 10px)){.a{width:calc(1px + 2px)}}";
        assert_eq!(
            fmt(src, 1, PRETTY),
            "@import url(\"a.css\"); /* note */\na:hover, a :focus > b {\n  color: red;\n  margin: 0 auto !important;\n}\n@media (min-width: 100px) and (max-width: calc(100% - 10px)) {\n  .a {\n    width: calc(1px + 2px);\n  }\n}"
        );
        let spaced = "/*! licence */\n@media ( min-width : 100px ) and ( max-width : 5em ) {\n  a :hover , b > c {\n    color : red ;\n    content : \" a  ;  b \" ;\n    width : calc( 1px + 2px ) ;\n    margin : 0 auto !important ;\n  }\n}\n/* gone */\n";
        assert_eq!(
            fmt(spaced, 1, MIN),
            "/*! licence */@media (min-width :100px) and (max-width :5em){a :hover,b > c{color:red;content:\" a  ;  b \";width:calc(1px + 2px);margin:0 auto!important}}"
        );
        assert_eq!(format("a{color:red", 1, PRETTY).unwrap_err(), "Syntax error at line 1, col 11: expect token `}`, but found `<eof>`");
        assert!(format("a{color:red", 1, MIN).is_err());
    }

    #[test]
    fn html_layout() {
        let src = "<!doctype html><html><head><title>T</title><meta charset=utf-8></head><body><div class=\"a\"   id=x><p>Hello <b>world</b>, this is <a href=\"/x\">a link</a>.</p><ul><li>one<li>two <i>2</i></ul><pre>  keep\n   this </pre><img src=a.png><br><!-- note --></div></body></html>";
        assert_eq!(
            fmt(src, 0, PRETTY),
            "<!doctype html>
<html>
  <head>
    <title>T</title>
    <meta charset=utf-8>
  </head>
  <body>
    <div class=\"a\" id=x>
      <p>Hello <b>world</b>, this is <a href=\"/x\">a link</a>.</p>
      <ul>
        <li>one
        <li>two <i>2</i>
      </ul>
      <pre>  keep
   this </pre>
      <img src=a.png><br><!-- note -->
    </div>
  </body>
</html>"
        );
    }

    #[test]
    fn html_is_lenient_and_keeps_inline_spacing() {
        // Unclosed and stray tags are laid out, not rejected, and nothing is dropped.
        assert_eq!(fmt("<div><p>one<p>two</span></div><section>", 0, PRETTY), "<div>\n  <p>one\n  <p>two</span>\n</div>\n<section>");
        // No whitespace appears where there was none, and none is lost where there was some.
        assert_eq!(fmt("<p>a<b>b</b>c <i>d</i> e<br>f</p>", 0, PRETTY), "<p>a<b>b</b>c <i>d</i> e<br>f</p>");
        assert_eq!(fmt("a < b and <3", 0, PRETTY), "a < b and <3");
        let long = format!("<p>{}</p>", vec!["word"; 40].join(" "));
        let out = fmt(&long, 0, PRETTY);
        assert!(out.starts_with("<p>\n  word word") && out.ends_with("word\n</p>"));
        assert!(out.lines().all(|l| l.chars().count() <= WIDTH));
        assert_eq!(out.split_whitespace().filter(|w| *w == "word").count(), 40);
        let table = fmt("<table><tr><td>1<td>2<tr><td>3</table>", 0, PRETTY);
        assert_eq!(table, "<table>\n  <tr>\n    <td>1\n    <td>2\n  <tr>\n    <td>3\n</table>");
    }

    #[test]
    fn html_embedded_code() {
        let src = "<style>body{margin:0}a{color:red}</style><script>function f(){return 1}</script><script type=\"application/json\">{\"a\":[1,2]}</script><script type=\"text/x-template\"><b>{{x}}</b></script><script src=a.js></script><script>const t = `a\n  b`;</script><script>oops(;</script>";
        let (out, notes) = format(src, 0, PRETTY).unwrap();
        assert_eq!(
            out,
            "<style>
  body {
    margin: 0;
  }
  a {
    color: red;
  }
</style>
<script>
  function f() {
    return 1;
  }
</script>
<script type=\"application/json\">
  {
    \"a\": [
      1,
      2
    ]
  }
</script>
<script type=\"text/x-template\">
  <b>{{x}}</b>
</script>
<script src=a.js></script>
<script>
const t = `a
  b`;
</script>
<script>
  oops(;
</script>"
        );
        assert_eq!(notes, ["A <script> block could not be parsed and was left as written"]);
    }

    #[test]
    fn html_minify() {
        let src = "<!doctype html>\n<html>\n  <body>\n    <!-- gone -->\n    <div   class=\"a\"\n         id=\"x\">\n      <p> Hello   <b>big</b>\n        world </p>\n      <pre>  keep\n  this </pre>\n      <style> a { color : red ; } </style>\n      <script>  function f ( ) { return 1 }  </script>\n      <span>a</span> <span>b</span>\n    </div>\n  </body>\n</html>\n";
        assert_eq!(
            fmt(src, 0, MIN),
            "<!doctype html><html><body><div class=\"a\" id=\"x\"><p>Hello <b>big</b> world</p><pre>  keep\n  this </pre><style>a{color:red}</style><script>function f(){return 1;}</script><span>a</span> <span>b</span></div></body></html>"
        );
        assert_eq!(fmt("<!--[if IE]><p>old</p><![endif]--><p>x</p>", 0, MIN), "<!--[if IE]><p>old</p><![endif]--><p>x</p>");
    }
}
