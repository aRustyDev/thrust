//! A port of the `zeed-dom` HTML parser that `@tiptap/html`'s `generateJSON` runs on the server.
//!
//! Faithful including its quirks (this is what produces today's stored data): a regex tokenizer, no implied end
//! tags, and an end tag that pops the element stack *whatever its name* (a stray end tag at the top level empties
//! the stack and everything after it is dropped). It is deliberately NOT an HTML5 parser.

use regex::Regex;
use std::sync::OnceLock;

#[derive(Clone, Debug, PartialEq)]
pub enum AttrV {
    Str(String),
    /// attribute without a value (`<input checked>`), JS `true`
    True,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Fragment,
    Element,
    Text,
}

#[derive(Clone, Debug)]
pub struct DNode {
    pub kind: Kind,
    pub parent: Option<usize>,
    pub children: Vec<usize>,
    /// element name as written (zeed keeps the original case for serialisation)
    pub name: String,
    /// upper-cased (`tagName`)
    pub tag: String,
    pub attrs: Vec<(String, AttrV)>,
    pub text: String,
}

#[derive(Debug, Default)]
pub struct Dom {
    pub nodes: Vec<DNode>,
}

const SELF_CLOSING: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "keygen", "link", "meta", "param",
    "source", "track", "wbr", "command",
];

impl Dom {
    pub fn new() -> Dom {
        let mut d = Dom::default();
        d.nodes.push(DNode {
            kind: Kind::Fragment,
            parent: None,
            children: vec![],
            name: String::new(),
            tag: String::new(),
            attrs: vec![],
            text: String::new(),
        });
        d
    }
    fn push(&mut self, n: DNode) -> usize {
        self.nodes.push(n);
        self.nodes.len() - 1
    }
    pub fn append_child(&mut self, parent: usize, child: usize) {
        if let Some(old) = self.nodes[child].parent {
            self.nodes[old].children.retain(|c| *c != child);
        }
        self.nodes[child].parent = Some(parent);
        self.nodes[parent].children.push(child);
    }
    pub fn first_child(&self, n: usize) -> Option<usize> {
        self.nodes[n].children.first().copied()
    }
    pub fn next_sibling(&self, n: usize) -> Option<usize> {
        let p = self.nodes[n].parent?;
        let sib = &self.nodes[p].children;
        let i = sib.iter().position(|c| *c == n)?;
        sib.get(i + 1).copied()
    }
    pub fn previous_sibling(&self, n: usize) -> Option<usize> {
        let p = self.nodes[n].parent?;
        let sib = &self.nodes[p].children;
        let i = sib.iter().position(|c| *c == n)?;
        if i == 0 {
            None
        } else {
            sib.get(i - 1).copied()
        }
    }
    pub fn is_element(&self, n: usize) -> bool {
        self.nodes[n].kind == Kind::Element
    }

    // ---- element API as zeed-dom exposes it -----------------------------------------------------
    fn find_attr(&self, n: usize, name: &str) -> Option<usize> {
        let lower = name.to_lowercase();
        self.nodes[n]
            .attrs
            .iter()
            .position(|(k, _)| k.to_lowercase() == lower)
    }
    /// `getAttribute`: string value, `""` for a valueless attribute, None when absent.
    pub fn get_attribute(&self, n: usize, name: &str) -> Option<String> {
        let i = self.find_attr(n, name)?;
        Some(match &self.nodes[n].attrs[i].1 {
            AttrV::Str(s) => s.clone(),
            AttrV::True => String::new(),
        })
    }
    pub fn has_attribute(&self, n: usize, name: &str) -> bool {
        self.find_attr(n, name).is_some()
    }
    /// `dom.style`: parsed inline declarations (name lower-cased by the regex callers), plus zeed's per-tag DEFAULTS.
    pub fn style(&self, n: usize) -> Style {
        let mut decls: Vec<(String, String)> = vec![];
        if let Some(s) = self.get_attribute(n, "style") {
            if !s.is_empty() {
                for c in style_re().captures_iter(&s) {
                    decls.push((c[1].to_string(), c[2].trim().to_string()));
                }
            }
        }
        let count = decls.len();
        let tag = self.nodes[n].tag.to_lowercase();
        let mut props: Vec<(String, String)> = vec![];
        match tag.as_str() {
            "b" | "strong" => props.push(("fontWeight".into(), "bold".into())),
            "em" | "i" => props.push(("fontStyle".into(), "italic".into())),
            "mark" | "ins" => props.push(("backgroundColor".into(), "rgb(255, 250, 165)".into())),
            "u" | "a" => props.push(("textDecorations".into(), "underline".into())),
            "s" | "del" | "strike" => props.push(("textDecorations".into(), "line-through".into())),
            _ => {}
        }
        for (k, v) in &decls {
            props.push((k.clone(), v.clone()));
            props.push((to_camel_case(k), v.clone()));
        }
        Style {
            count,
            props,
            decls,
        }
    }
    /// `dom.dataset[name]` (camelCase or the raw suffix).
    pub fn dataset(&self, n: usize, key: &str) -> Option<AttrV> {
        for (k, v) in &self.nodes[n].attrs {
            if let Some(rest) = k.strip_prefix("data-") {
                if rest == key || to_camel_case(rest) == key {
                    return Some(v.clone());
                }
            }
        }
        None
    }
    pub fn first_element_child(&self, n: usize) -> Option<usize> {
        self.nodes[n]
            .children
            .iter()
            .copied()
            .find(|c| self.is_element(*c))
    }
    pub fn class_list_contains(&self, n: usize, cls: &str) -> bool {
        self.get_attribute(n, "class")
            .map(|c| c.trim().split_whitespace().any(|x| x == cls))
            .unwrap_or(false)
    }
}

pub struct Style {
    pub count: usize,
    props: Vec<(String, String)>,
    decls: Vec<(String, String)>,
}
impl Style {
    /// property by camelCase or hyphenated name; later declarations win (JS object spread)
    pub fn get(&self, name: &str) -> Option<&str> {
        self.props
            .iter()
            .rev()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }
    /// `getPropertyValue(name)`: only declared (inline) properties.
    pub fn get_property_value(&self, name: &str) -> Option<&str> {
        self.decls
            .iter()
            .rev()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }
}

fn to_camel_case(s: &str) -> String {
    // s.toLowerCase().replace(/[^a-z0-9]+(.)/gi, (_m, chr) => chr.toUpperCase())
    let lower = s.to_lowercase();
    let mut out = String::new();
    let chars: Vec<char> = lower.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_ascii_alphanumeric() {
            out.push(chars[i]);
            i += 1;
        } else {
            let mut j = i;
            while j < chars.len() && !chars[j].is_ascii_alphanumeric() {
                j += 1;
            }
            // `[^a-z0-9]+(.)`: the run, then ONE more char (any) is upper-cased
            if j < chars.len() {
                out.extend(chars[j].to_uppercase());
                i = j + 1;
            } else {
                // trailing run with no following char: the regex does not match, text is kept
                for c in &chars[i..] {
                    out.push(*c);
                }
                i = chars.len();
            }
        }
    }
    out
}

fn style_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"(?i)\s*([\w-]+)\s*:\s*((url\(.*?\)[^;]*|[^;]+))").unwrap())
}
fn start_tag_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        Regex::new(
            r#"(?m)^<([^>\s/]+)((\s+[^=>\s]+(\s*=\s*("[^"]*"|'[^']*'|[^>\s]+))?)*)\s*(?:/\s*)?>"#,
        )
        .unwrap()
    })
}
fn end_tag_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"(?m)^</([^>\s]+)[^>]*>").unwrap())
}
fn self_close_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"(?m)\s*/\s*>\s*$").unwrap())
}
fn attr_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r#"([^=\s]+)(\s*=\s*(("([^"]*)")|('([^']*)')|[^>\s]+))?"#).unwrap())
}

/// The 106 HTML5 "legacy" entity names that decode without a trailing `;`.
const LEGACY: &[&str] = &[
    "AElig", "AMP", "Aacute", "Acirc", "Agrave", "Aring", "Atilde", "Auml", "COPY", "Ccedil",
    "ETH", "Eacute", "Ecirc", "Egrave", "Euml", "GT", "Iacute", "Icirc", "Igrave", "Iuml", "LT",
    "Ntilde", "Oacute", "Ocirc", "Ograve", "Oslash", "Otilde", "Ouml", "QUOT", "REG", "THORN",
    "Uacute", "Ucirc", "Ugrave", "Uuml", "Yacute", "aacute", "acirc", "acute", "aelig", "agrave",
    "amp", "aring", "atilde", "auml", "brvbar", "ccedil", "cedil", "cent", "copy", "curren", "deg",
    "divide", "eacute", "ecirc", "egrave", "eth", "euml", "frac12", "frac14", "frac34", "gt",
    "iacute", "icirc", "iexcl", "igrave", "iquest", "iuml", "laquo", "lt", "macr", "micro",
    "middot", "nbsp", "not", "ntilde", "oacute", "ocirc", "ograve", "ordf", "ordm", "oslash",
    "otilde", "ouml", "para", "plusmn", "pound", "quot", "raquo", "reg", "sect", "shy", "sup1",
    "sup2", "sup3", "szlig", "thorn", "times", "uacute", "ucirc", "ugrave", "uml", "uuml",
    "yacute", "yen", "yuml",
];

fn named_entity(name_with_semicolon: &str) -> Option<String> {
    let o = htmlentity::entity::decode(name_with_semicolon.as_bytes());
    let st: htmlentity::types::StringResult = (&o).into();
    match st {
        Ok(s) if s != name_with_semicolon => Some(s),
        _ => None,
    }
}

/// `entities` `replaceCodePoint`
fn replace_code_point(cp: u32) -> char {
    if (0xd800..=0xdfff).contains(&cp) || cp > 0x10ffff {
        return '\u{fffd}';
    }
    let mapped = match cp {
        0 => 0xfffd,
        0x80 => 0x20ac,
        0x82 => 0x201a,
        0x83 => 0x0192,
        0x84 => 0x201e,
        0x85 => 0x2026,
        0x86 => 0x2020,
        0x87 => 0x2021,
        0x88 => 0x02c6,
        0x89 => 0x2030,
        0x8a => 0x0160,
        0x8b => 0x2039,
        0x8c => 0x0152,
        0x8e => 0x017d,
        0x91 => 0x2018,
        0x92 => 0x2019,
        0x93 => 0x201c,
        0x94 => 0x201d,
        0x95 => 0x2022,
        0x96 => 0x2013,
        0x97 => 0x2014,
        0x98 => 0x02dc,
        0x99 => 0x2122,
        0x9a => 0x0161,
        0x9b => 0x203a,
        0x9c => 0x0153,
        0x9e => 0x017e,
        0x9f => 0x0178,
        c => c,
    };
    char::from_u32(mapped).unwrap_or('\u{fffd}')
}

/// `entities.decodeHTML` in legacy mode: numeric references with or without `;`, every HTML5 named reference
/// with `;`, and the 106 legacy names without it (longest legacy prefix of the alphanumeric run).
pub fn unescape_html(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let b = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] != b'&' {
            let ch = s[i..].chars().next().unwrap();
            out.push(ch);
            i += ch.len_utf8();
            continue;
        }
        let rest = &s[i + 1..];
        if let Some(num) = rest.strip_prefix('#') {
            let (hex, digits_part) = match num.strip_prefix(['x', 'X']) {
                Some(h) => (true, h),
                None => (false, num),
            };
            let n = digits_part
                .bytes()
                .take_while(|c| {
                    if hex {
                        c.is_ascii_hexdigit()
                    } else {
                        c.is_ascii_digit()
                    }
                })
                .count();
            if n > 0 {
                let mut cp: u64 = 0;
                for c in digits_part[..n].chars() {
                    cp = (cp * if hex { 16 } else { 10 }
                        + c.to_digit(if hex { 16 } else { 10 }).unwrap() as u64)
                        .min(0x1fffff);
                }
                out.push(replace_code_point(cp as u32));
                let consumed = 1 + 1 + usize::from(hex) + n;
                i += consumed;
                if s.as_bytes().get(i) == Some(&b';') {
                    i += 1;
                }
                continue;
            }
            out.push('&');
            i += 1;
            continue;
        }
        let run: usize = rest
            .bytes()
            .take_while(|c| c.is_ascii_alphanumeric())
            .count();
        if run > 0 {
            let name = &rest[..run];
            if rest.as_bytes().get(run) == Some(&b';') {
                if let Some(rep) = named_entity(&format!("&{name};")) {
                    out.push_str(&rep);
                    i += 1 + run + 1;
                    continue;
                }
            }
            if let Some(len) = (2..=run).rev().find(|l| LEGACY.contains(&&name[..*l])) {
                if let Some(rep) = named_entity(&format!("&{};", &name[..len])) {
                    out.push_str(&rep);
                    i += 1 + len;
                    continue;
                }
            }
        }
        out.push('&');
        i += 1;
    }
    out
}

/// `parseHTML(html)` from zeed-dom.
pub fn parse_html(input: &str) -> Dom {
    let mut dom = Dom::new();
    let mut stack: Vec<usize> = vec![0];
    let mut html: &str = input;
    let mut treat_as_chars = false;
    while !html.is_empty() {
        if html.starts_with("<!--") {
            if let Some(idx) = html.find("-->") {
                html = &html[idx + 3..];
                treat_as_chars = false;
            } else {
                treat_as_chars = true;
            }
        } else if html.starts_with("</") {
            if let Some(m) = end_tag_re().find(html) {
                html = &html[m.end()..];
                treat_as_chars = false;
                stack.pop(); // endElement pops whatever is on top
            } else {
                treat_as_chars = true;
            }
        } else if html.starts_with('<') {
            if let Some(caps) = start_tag_re().captures(html) {
                let whole = caps.get(0).unwrap();
                let rest = &html[whole.end()..];
                treat_as_chars = false;
                let tag_name = caps[1].to_string();
                let is_self_close = self_close_re().is_match(whole.as_str());
                let mut attr_input = caps.get(2).map(|m| m.as_str()).unwrap_or("").to_string();
                if is_self_close {
                    attr_input = trim_trailing_slash(&attr_input);
                }
                // parseAttributes
                let mut attrs: Vec<(String, AttrV)> = vec![];
                for c in attr_re().captures_iter(&attr_input) {
                    let name = c[1].to_string();
                    let v = if let Some(sq) = c.get(7) {
                        AttrV::Str(sq.as_str().to_string())
                    } else if let Some(dq) = c.get(5) {
                        AttrV::Str(dq.as_str().to_string())
                    } else if let Some(raw) = c.get(3) {
                        AttrV::Str(raw.as_str().to_string())
                    } else {
                        AttrV::True
                    };
                    if let Some(e) = attrs.iter_mut().find(|(k, _)| *k == name) {
                        e.1 = v;
                    } else {
                        attrs.push((name, v));
                    }
                }
                // scanner.startElement
                let lower = tag_name.to_lowercase();
                if lower != "!doctype" {
                    for (_, v) in attrs.iter_mut() {
                        if let AttrV::Str(s) = v {
                            *s = unescape_html(s);
                        }
                    }
                    if let Some(&top) = stack.last() {
                        let el = dom.push(DNode {
                            kind: Kind::Element,
                            parent: None,
                            children: vec![],
                            name: tag_name.clone(),
                            tag: tag_name.to_uppercase(),
                            attrs,
                            text: String::new(),
                        });
                        dom.append_child(top, el);
                        if !(SELF_CLOSING.contains(&lower.as_str()) || is_self_close) {
                            stack.push(el);
                        }
                    }
                }
                html = rest;
                if lower == "script" || lower == "style" {
                    let needle = format!("</{lower}");
                    if let Some(idx) = html.to_ascii_lowercase().find(&needle) {
                        characters(&mut dom, &stack, &html[..idx]);
                        html = &html[idx..];
                        treat_as_chars = false;
                    } else {
                        treat_as_chars = true;
                    }
                }
            } else {
                treat_as_chars = true;
            }
        }
        if treat_as_chars {
            let first = html.find('<');
            let mut offset = first;
            let mut index = first;
            if index == Some(0) {
                index = html[1..].find('<'); // may be None
                offset = Some(1);
            }
            let (chars, rest) = match index {
                None => (html, ""),
                Some(_) => {
                    let off = offset.unwrap();
                    (&html[..off], &html[off..])
                }
            };
            characters(&mut dom, &stack, chars);
            html = rest;
        }
        treat_as_chars = true;
    }
    dom
}

fn trim_trailing_slash(s: &str) -> String {
    // attrInput.replace(/\s*\/\s*$/, "")
    let t = s.trim_end();
    if let Some(stripped) = t.strip_suffix('/') {
        stripped.trim_end().to_string()
    } else {
        s.to_string()
    }
}

fn characters(dom: &mut Dom, stack: &[usize], text: &str) {
    let text = unescape_html(text);
    let Some(&top) = stack.last() else { return };
    if let Some(&last) = dom.nodes[top].children.last() {
        if dom.nodes[last].kind == Kind::Text {
            dom.nodes[last].text.push_str(&text);
            return;
        }
    }
    let t = dom.push(DNode {
        kind: Kind::Text,
        parent: None,
        children: vec![],
        name: String::new(),
        tag: "#text".into(),
        attrs: vec![],
        text,
    });
    dom.append_child(top, t);
}
