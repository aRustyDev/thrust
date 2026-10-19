//! ProseMirror document -> HTML string, byte-compatible with `generateHTML(json, extensions)` from `@tiptap/html`
//! (ProseMirror `DOMSerializer` on the `zeed-dom` document).
//!
//! The per-node/mark shapes are transcribed from the schema's `toDOM` (see `tests/corpus/todom.json`), and the
//! zeed-dom serialisation quirks are reproduced on purpose, because that is what the product stores today:
//!   * `style` never reaches the output (ProseMirror assigns `dom.style.cssText`, which zeed-dom ignores), so
//!     text alignment, hex colours and table-cell colour styles are not in server-generated HTML;
//!   * `' " & < >`, U+00A0 and U+00AD are escaped in text AND attributes (`&apos;`, `&nbsp;`, `&shy;`);
//!   * boolean `true` attributes are bare (`data-checked`), `false`/`null` are omitted, arrays join with `,`.

use crate::pm::{Mark, Node};
use crate::schema::Schema;
use serde_json::Value;
use std::sync::OnceLock;

pub fn escape_html(text: &str, out: &mut String) {
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\'' => out.push_str("&apos;"),
            '"' => out.push_str("&quot;"),
            '\u{a0}' => out.push_str("&nbsp;"),
            '\u{ad}' => out.push_str("&shy;"),
            c => out.push(c),
        }
    }
}

/// JS `Number.prototype.toString` for the values that occur in attributes.
pub fn js_number_string(n: &serde_json::Number) -> String {
    if let Some(i) = n.as_i64() {
        return i.to_string();
    }
    let f = n.as_f64().unwrap_or(0.0);
    if f.fract() == 0.0 && f.abs() < 1e21 {
        return format!("{}", f as i128);
    }
    if f.abs() >= 1e21 || (f != 0.0 && f.abs() < 1e-6) {
        // exponent form like JS: 1e+21, 1.5e-7
        let s = format!("{:e}", f);
        return match s.split_once('e') {
            Some((m, e)) if !e.starts_with('-') => format!("{m}e+{e}"),
            _ => s,
        };
    }
    format!("{}", f)
}

/// An attribute as it would be handed to `setAttribute` (None = skipped by ProseMirror: null/undefined).
#[derive(Clone, Debug)]
pub enum AV {
    Str(String),
    True,
    False,
    Val(Value),
}

fn av_from_value(v: &Value) -> Option<AV> {
    match v {
        Value::Null => None,
        Value::Bool(true) => Some(AV::True),
        Value::Bool(false) => Some(AV::False),
        Value::String(s) => Some(AV::Str(s.clone())),
        other => Some(AV::Val(other.clone())),
    }
}

fn to_js_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => js_number_string(n),
        Value::Bool(b) => b.to_string(),
        Value::Null => String::new(),
        Value::Array(a) => a.iter().map(to_js_string).collect::<Vec<_>>().join(","),
        Value::Object(_) => "[object Object]".into(),
    }
}

fn write_attr(out: &mut String, name: &str, v: &AV) {
    match v {
        AV::True => {
            out.push(' ');
            out.push_str(name);
        }
        AV::False => {}
        AV::Str(s) => {
            out.push(' ');
            out.push_str(name);
            out.push_str("=\"");
            escape_html(s, out);
            out.push('"');
        }
        AV::Val(val) => {
            out.push(' ');
            out.push_str(name);
            out.push_str("=\"");
            escape_html(&to_js_string(val), out);
            out.push('"');
        }
    }
}

/// Ordered attribute list with Tiptap's `mergeAttributes` semantics.
#[derive(Default, Clone)]
pub struct AttrList(Vec<(String, Option<AV>)>);

impl AttrList {
    pub fn new() -> Self {
        AttrList(vec![])
    }
    /// plain insert (later keys replace earlier ones in place, new keys append) with no class/style merging
    pub fn set(&mut self, k: &str, v: Option<AV>) {
        if let Some(e) = self.0.iter_mut().find(|(n, _)| n == k) {
            e.1 = v;
        } else {
            self.0.push((k.to_string(), v));
        }
    }
    pub fn s(mut self, k: &str, v: &str) -> Self {
        self.set(k, Some(AV::Str(v.to_string())));
        self
    }
    pub fn val(mut self, k: &str, v: Option<&Value>) -> Self {
        self.set(k, v.and_then(av_from_value));
        self
    }
    /// Tiptap `mergeAttributes(base, extra)`.
    pub fn merge(mut self, extra: &AttrList) -> Self {
        for (k, v) in &extra.0 {
            let existing_truthy = self
                .0
                .iter()
                .find(|(n, _)| n == k)
                .map(|(_, v)| truthy(v))
                .unwrap_or(false);
            if !existing_truthy {
                self.set(k, v.clone());
                continue;
            }
            if k == "class" {
                let mut classes: Vec<String> = match self
                    .0
                    .iter()
                    .find(|(n, _)| n == k)
                    .and_then(|(_, v)| v.as_ref())
                {
                    Some(AV::Str(s)) => s.split(' ').map(|x| x.to_string()).collect(),
                    _ => vec![],
                };
                if let Some(AV::Str(s)) = v {
                    for c in s.split(' ') {
                        if !classes.iter().any(|x| x == c) {
                            classes.push(c.to_string());
                        }
                    }
                }
                self.set("class", Some(AV::Str(classes.join(" "))));
            } else if k == "style" {
                // dropped on output anyway; keep the structure faithful
                let a = self
                    .0
                    .iter()
                    .find(|(n, _)| n == k)
                    .and_then(|(_, v)| v.as_ref())
                    .map(|v| match v {
                        AV::Str(s) => s.clone(),
                        _ => String::new(),
                    });
                let b = match v {
                    Some(AV::Str(s)) => s.clone(),
                    _ => String::new(),
                };
                self.set(
                    "style",
                    Some(AV::Str(format!("{}; {}", a.unwrap_or_default(), b))),
                );
            } else {
                self.set(k, v.clone());
            }
        }
        self
    }
    pub fn write(&self, out: &mut String) {
        for (k, v) in &self.0 {
            // `style` is routed through dom.style.cssText by ProseMirror and lost by zeed-dom
            if k == "style" {
                continue;
            }
            if let Some(v) = v {
                write_attr(out, k, v);
            }
        }
    }
}

fn truthy(v: &Option<AV>) -> bool {
    match v {
        None | Some(AV::False) => false,
        Some(AV::Str(s)) => !s.is_empty(),
        Some(AV::Val(Value::Number(n))) => n.as_f64().map(|f| f != 0.0).unwrap_or(true),
        Some(AV::Val(Value::Null)) => false,
        _ => true,
    }
}

// ---------------------------------------------------------------------------------------------
// emoji
// ---------------------------------------------------------------------------------------------

struct Emoji {
    name: String,
    glyph: String,
    shortcodes: Vec<String>,
}
static EMOJI: OnceLock<Vec<Emoji>> = OnceLock::new();
static EMOJI_JSON: &str = include_str!("../data/emoji.json");

fn emojis() -> &'static Vec<Emoji> {
    EMOJI.get_or_init(|| {
        let rows: Vec<(String, String, Vec<String>)> =
            serde_json::from_str(EMOJI_JSON).expect("emoji.json");
        rows.into_iter()
            .map(|(name, glyph, shortcodes)| Emoji {
                name,
                glyph,
                shortcodes,
            })
            .collect()
    })
}
/// `shortcodeToEmoji`: first item whose name equals the shortcode or lists it.
fn emoji_lookup(name: &str) -> Option<&'static Emoji> {
    emojis()
        .iter()
        .find(|e| e.name == name || e.shortcodes.iter().any(|s| s == name))
}

// ---------------------------------------------------------------------------------------------

const LINK_CLASS: &str = "text-accent-secondary underline underline-offset-[3px] hover:text-accent-primary transition-colors cursor-pointer";
const CODE_CLASS: &str = "rounded-sm bg-layer-3 px-[6px] py-[1.5px] font-code font-medium text-(--extended-color-orange-600) border-[0.5px] border-subtle";

fn js_ws(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\u{b}' | '\u{c}' | '\r' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200a}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202f}'
                | '\u{205f}'
                | '\u{3000}'
                | '\u{feff}'
    )
}

/// `isDangerousHref` from custom-link/extension.tsx.
pub fn is_dangerous_href(raw: &str) -> bool {
    let no_tab: String = raw
        .chars()
        .filter(|c| !matches!(c, '\t' | '\n' | '\r'))
        .collect();
    let trimmed = no_tab.trim_start_matches(|c: char| {
        js_ws(c) || ('\u{0}'..='\u{8}').contains(&c) || ('\u{e}'..='\u{1f}').contains(&c)
    });
    let lower = trimmed.to_lowercase();
    ["javascript:", "data:", "vbscript:"]
        .iter()
        .any(|p| lower.starts_with(p))
}

struct Open {
    tag: &'static str,
    attrs: AttrList,
}

impl Open {
    fn t(tag: &'static str, attrs: AttrList) -> Open {
        Open { tag, attrs }
    }
    fn write(&self, out: &mut String) {
        out.push('<');
        out.push_str(self.tag);
        self.attrs.write(out);
        out.push('>');
    }
}

fn a(node: &Node, k: &str) -> Option<Value> {
    node.attrs.get(k).cloned()
}

fn mark_open(schema: &Schema, m: &Mark) -> Open {
    let name = schema.marks[m.ty].name;
    match name {
        "link" => {
            let opts = AttrList::new()
                .s("target", "_blank")
                .s("rel", "noopener noreferrer nofollow")
                .s("class", LINK_CLASS);
            let mut h = AttrList::new()
                .val("href", m.attrs.get("href"))
                .val("target", m.attrs.get("target"))
                .val("rel", m.attrs.get("rel"))
                .val("class", m.attrs.get("class"));
            let href = m.attrs.get("href").and_then(|v| v.as_str()).unwrap_or("");
            if is_dangerous_href(href) {
                h.set("href", Some(AV::Str(String::new())));
            }
            Open::t("a", opts.merge(&h))
        }
        "textStyle" => Open::t("span", AttrList::new()),
        "bold" => Open::t("strong", AttrList::new()),
        "italic" => Open::t("em", AttrList::new()),
        "strike" => Open::t("s", AttrList::new()),
        "underline" => Open::t("u", AttrList::new()),
        "code" => Open::t(
            "code",
            AttrList::new()
                .s("class", CODE_CLASS)
                .s("spellcheck", "false"),
        ),
        "customColor" => {
            let mut h = AttrList::new();
            if let Some(c) = m
                .attrs
                .get("color")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
            {
                h.set("data-text-color", Some(AV::Str(c.to_string())));
                h.set("style", Some(AV::Str(format!("color: {c}"))));
            }
            if let Some(c) = m
                .attrs
                .get("backgroundColor")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
            {
                h.set("data-background-color", Some(AV::Str(c.to_string())));
            }
            Open::t("span", h)
        }
        other => panic!("unknown mark {other}"),
    }
}

fn render_node(schema: &Schema, n: &Node, out: &mut String) {
    if let Some(t) = &n.text {
        escape_html(t, out);
        return;
    }
    let name = schema.node_name(n.ty);
    let open =
        |tag: &'static str, attrs: AttrList, out: &mut String| Open::t(tag, attrs).write(out);
    let close = |tag: &str, out: &mut String| {
        out.push_str("</");
        out.push_str(tag);
        out.push('>');
    };
    let children = |out: &mut String| render_fragment(schema, &n.content, out);
    match name {
        "paragraph" => {
            open(
                "p",
                AttrList::new().s("class", "editor-paragraph-block"),
                out,
            );
            children(out);
            close("p", out);
        }
        "heading" => {
            let level = a(n, "level")
                .and_then(|v| v.as_i64())
                .filter(|l| (1..=6).contains(l))
                .unwrap_or(1);
            let tag = ["h1", "h2", "h3", "h4", "h5", "h6"][(level - 1) as usize];
            open(tag, AttrList::new().s("class", "editor-heading-block"), out);
            children(out);
            close(tag, out);
        }
        "bulletList" => {
            open(
                "ul",
                AttrList::new().s("class", "list-disc pl-7 space-y-(--list-spacing-y)"),
                out,
            );
            children(out);
            close("ul", out);
        }
        "orderedList" => {
            let mut at = AttrList::new().s("class", "list-decimal pl-7 space-y-(--list-spacing-y)");
            let start = a(n, "start");
            if start.as_ref().and_then(|v| v.as_i64()) != Some(1) {
                at.set("start", start.as_ref().and_then(av_from_value));
            }
            at.set("type", a(n, "type").as_ref().and_then(av_from_value));
            open("ol", at, out);
            children(out);
            close("ol", out);
        }
        "listItem" => {
            open("li", AttrList::new().s("class", "not-prose space-y-2"), out);
            children(out);
            close("li", out);
        }
        "taskList" => {
            open(
                "ul",
                AttrList::new()
                    .s("class", "not-prose pl-2 space-y-2")
                    .s("data-type", "taskList"),
                out,
            );
            children(out);
            close("ul", out);
        }
        "taskItem" => {
            let checked = a(n, "checked").and_then(|v| v.as_bool()).unwrap_or(false);
            let mut at = AttrList::new().s("class", "flex");
            at.set(
                "data-checked",
                Some(if checked { AV::True } else { AV::False }),
            );
            at.set("data-type", Some(AV::Str("taskItem".into())));
            open("li", at, out);
            out.push_str("<label><input type=\"checkbox\"");
            if checked {
                out.push_str(" checked=\"checked\"");
            }
            out.push_str("><span></span></label><div>");
            children(out);
            out.push_str("</div></li>");
        }
        "blockquote" => {
            open("blockquote", AttrList::new(), out);
            children(out);
            close("blockquote", out);
        }
        "codeBlock" => {
            out.push_str("<pre><code");
            if let Some(l) = a(n, "language")
                .and_then(|v| v.as_str().map(|s| s.to_string()))
                .filter(|s| !s.is_empty())
            {
                out.push_str(" class=\"language-");
                escape_html(&l, out);
                out.push('"');
            }
            out.push('>');
            children(out);
            out.push_str("</code></pre>");
        }
        "horizontalRule" => {
            open(
                "div",
                AttrList::new()
                    .s("class", "py-4 border-strong-1")
                    .s("data-type", "horizontalRule"),
                out,
            );
            out.push_str("<div></div></div>");
        }
        "hardBreak" => out.push_str("<br>"),
        "image" => {
            let at = AttrList::new()
                .val("src", n.attrs.get("src"))
                .val("alt", n.attrs.get("alt"))
                .val("title", n.attrs.get("title"))
                .val("width", n.attrs.get("width"))
                .val("height", n.attrs.get("height"))
                .val("aspectRatio", n.attrs.get("aspectRatio"))
                .val("alignment", n.attrs.get("alignment"));
            open("img", at, out);
        }
        "imageComponent" => {
            let at = AttrList::new()
                .val("src", n.attrs.get("src"))
                .val("alt", n.attrs.get("alt"))
                .val("title", n.attrs.get("title"))
                .val("id", n.attrs.get("id"))
                .val("width", n.attrs.get("width"))
                .val("height", n.attrs.get("height"))
                .val("aspectRatio", n.attrs.get("aspectRatio"))
                .val("alignment", n.attrs.get("alignment"))
                .val("status", n.attrs.get("status"));
            open("image-component", at, out);
            close("image-component", out);
        }
        "mention" => {
            let at = AttrList::new()
                .val("id", n.attrs.get("id"))
                .val("entity_identifier", n.attrs.get("entity_identifier"))
                .val("entity_name", n.attrs.get("entity_name"));
            open("mention-component", at, out);
            close("mention-component", out);
        }
        "emoji" => {
            let nm = n.attrs.get("name").and_then(|v| v.as_str());
            let mut at = AttrList::new();
            at.set("data-name", n.attrs.get("name").and_then(av_from_value));
            at.set("data-type", Some(AV::Str("emoji".into())));
            open("span", at, out);
            let inner = match nm.and_then(emoji_lookup) {
                Some(e) => e.glyph.clone(),
                None => format!(":{}:", nm.unwrap_or("null")),
            };
            escape_html(&inner, out);
            close("span", out);
        }
        "table" => {
            open("table", AttrList::new(), out);
            out.push_str("<tbody>");
            children(out);
            out.push_str("</tbody></table>");
        }
        "tableRow" => {
            let at = AttrList::new()
                .val("background", n.attrs.get("background"))
                .val("textColor", n.attrs.get("textColor"));
            open("tr", at, out);
            children(out);
            close("tr", out);
        }
        "tableHeader" => {
            let at = AttrList::new()
                .val("colspan", n.attrs.get("colspan"))
                .val("rowspan", n.attrs.get("rowspan"))
                .val("colwidth", n.attrs.get("colwidth"))
                .val("background", n.attrs.get("background"));
            open("th", at, out);
            children(out);
            close("th", out);
        }
        "tableCell" => {
            let at = AttrList::new()
                .val("colspan", n.attrs.get("colspan"))
                .val("rowspan", n.attrs.get("rowspan"))
                .val("colwidth", n.attrs.get("colwidth"))
                .val("background", n.attrs.get("background"))
                .val("textColor", n.attrs.get("textColor"));
            open("td", at, out);
            children(out);
            close("td", out);
        }
        "calloutComponent" => {
            let mut at = AttrList::new();
            for k in [
                "id",
                "data-icon-color",
                "data-icon-name",
                "data-emoji-unicode",
                "data-emoji-url",
                "data-logo-in-use",
                "data-background",
                "data-block-type",
            ] {
                at = at.val(k, n.attrs.get(k));
            }
            open("div", at, out);
            children(out);
            close("div", out);
        }
        "issue-embed-component" => {
            let mut at = AttrList::new();
            for k in [
                "entity_identifier",
                "project_identifier",
                "workspace_identifier",
                "id",
                "entity_name",
            ] {
                at = at.val(k, n.attrs.get(k));
            }
            open("issue-embed-component", at, out);
            close("issue-embed-component", out);
        }
        other => panic!("no HTML renderer for node {other}"),
    }
}

/// `DOMSerializer.serializeFragment`: adjacent nodes share the mark wrappers they have in common.
pub fn render_fragment(schema: &Schema, nodes: &[Node], out: &mut String) {
    let mut active: Vec<(&Mark, &'static str)> = vec![];
    for node in nodes {
        let mut keep = 0;
        let mut rendered = 0;
        while keep < active.len() && rendered < node.marks.len() {
            let next = &node.marks[rendered];
            if next != active[keep].0 {
                break;
            }
            keep += 1;
            rendered += 1;
        }
        while active.len() > keep {
            let (_, tag) = active.pop().unwrap();
            out.push_str("</");
            out.push_str(tag);
            out.push('>');
        }
        while rendered < node.marks.len() {
            let m = &node.marks[rendered];
            rendered += 1;
            let o = mark_open(schema, m);
            o.write(out);
            active.push((m, o.tag));
        }
        render_node(schema, node, out);
    }
    while let Some((_, tag)) = active.pop() {
        out.push_str("</");
        out.push_str(tag);
        out.push('>');
    }
}

/// `generateHTML(doc, extensions)`.
pub fn to_html(schema: &Schema, doc: &Node) -> String {
    let mut s = String::new();
    render_fragment(schema, &doc.content, &mut s);
    s
}
