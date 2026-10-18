//! HTML -> ProseMirror document: a port of `prosemirror-model`'s `DOMParser` (`from_dom.ts`) over the zeed-dom
//! tree from `vdom.rs`, with the parse rules of the Plane/Tiptap extensions transcribed from the sources
//! (`tests/corpus/schema.json` records the tag and style rules the real schema exposes).
//!
//! Tiptap's `injectExtensionAttributesToParseRule` behaviour is reproduced: every extension attribute is read with
//! `fromString(getAttribute(name))` unless the extension supplies its own `parseHTML`.

use crate::pm::{join_text, mark_set_from, Attrs, Mark, Node};
use crate::schema::{Schema, TypeId};
use crate::vdom::{AttrV, Dom, Kind};
use serde_json::Value;

const OPT_PRESERVE_WS: u8 = 1;
const OPT_PRESERVE_WS_FULL: u8 = 2;
const OPT_OPEN_LEFT: u8 = 4;

const BLOCK_TAGS: &[&str] = &[
    "address",
    "article",
    "aside",
    "blockquote",
    "body",
    "canvas",
    "dd",
    "div",
    "dl",
    "fieldset",
    "figcaption",
    "figure",
    "footer",
    "form",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "header",
    "hgroup",
    "hr",
    "li",
    "noscript",
    "ol",
    "output",
    "p",
    "pre",
    "section",
    "table",
    "tfoot",
    "ul",
];
const IGNORE_TAGS: &[&str] = &["head", "noscript", "object", "script", "style", "title"];

fn is_ws(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\r' | '\n' | '\u{c}')
}

// ------------------------------------------------------------------------------------------------
// rules
// ------------------------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
enum Sel {
    Tag(&'static str),
    TagAttrEq(&'static str, &'static str, &'static str),
    TagAttrExists(&'static str, &'static str),
    ImgSrcNotData,
}

impl Sel {
    fn matches(&self, dom: &Dom, el: usize) -> bool {
        let tag = &dom.nodes[el].tag;
        match *self {
            Sel::Tag(t) => tag.eq_ignore_ascii_case(t),
            Sel::TagAttrEq(t, a, v) => {
                tag.eq_ignore_ascii_case(t) && dom.get_attribute(el, a).as_deref() == Some(v)
            }
            Sel::TagAttrExists(t, a) => tag.eq_ignore_ascii_case(t) && dom.has_attribute(el, a),
            Sel::ImgSrcNotData => {
                tag.eq_ignore_ascii_case("img")
                    && dom.has_attribute(el, "src")
                    && !dom
                        .get_attribute(el, "src")
                        .map(|s| s.starts_with("data:"))
                        .unwrap_or(false)
            }
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum Target {
    Node(TypeId),
    Mark(usize),
}

/// rule-level `getAttrs` (None = returns `false`)
type RuleFn = fn(&Dom, usize) -> Option<Attrs>;

#[derive(Clone)]
struct TagRule {
    sel: Sel,
    target: Target,
    get_attrs: Option<RuleFn>,
    static_attrs: Option<Attrs>,
    preserve_ws: Option<bool>,
    preserve_full: bool,
    priority: i32,
}

#[derive(Clone)]
struct StyleRule {
    style: &'static str,
    mark: usize,
    clear_mark: bool,
    consuming: bool,
    get_attrs: Option<fn(&str) -> Option<Attrs>>,
}

fn dangerous(dom: &Dom, el: usize) -> bool {
    crate::html::is_dangerous_href(&dom.get_attribute(el, "href").unwrap_or_default())
}

fn ga_link(dom: &Dom, el: usize) -> Option<Attrs> {
    if dangerous(dom, el) {
        None
    } else {
        Some(Attrs::new())
    }
}
fn ga_textstyle(dom: &Dom, el: usize) -> Option<Attrs> {
    if dom.has_attribute(el, "style") {
        Some(Attrs::new())
    } else {
        None
    }
}
fn ga_bold_b(dom: &Dom, el: usize) -> Option<Attrs> {
    // style.fontWeight !== 'normal' && null
    if dom.style(el).get("fontWeight") != Some("normal") {
        Some(Attrs::new())
    } else {
        None
    }
}
fn ga_italic_i(dom: &Dom, el: usize) -> Option<Attrs> {
    if dom.style(el).get("fontStyle") != Some("normal") {
        Some(Attrs::new())
    } else {
        None
    }
}
fn ga_always(_: &Dom, _: usize) -> Option<Attrs> {
    // `x && null` forms that can never return false
    Some(Attrs::new())
}
fn sga_bold_weight(v: &str) -> Option<Attrs> {
    // /^(bold(er)?|[5-9]\d{2,})$/
    let ok = v == "bold"
        || v == "bolder"
        || (v.len() >= 3
            && v.as_bytes()[0] >= b'5'
            && v.as_bytes()[0] <= b'9'
            && v.bytes().all(|b| b.is_ascii_digit()));
    ok.then(Attrs::new)
}
fn sga_strike(v: &str) -> Option<Attrs> {
    v.contains("line-through").then(Attrs::new)
}
fn sga_underline(v: &str) -> Option<Attrs> {
    v.contains("underline").then(Attrs::new)
}

fn build_rules(schema: &Schema) -> (Vec<TagRule>, Vec<StyleRule>, Vec<&'static str>) {
    let n = |name: &str| schema.node_type(name).unwrap();
    let m = |name: &str| schema.mark_type(name).unwrap();
    let mut tags: Vec<TagRule> = vec![];
    let tr = |sel: Sel, target: Target, ga: Option<RuleFn>| TagRule {
        sel,
        target,
        get_attrs: ga,
        static_attrs: None,
        preserve_ws: None,
        preserve_full: false,
        priority: 50,
    };

    // marks first (schema.marks order), then nodes (schema.nodes order) — then stable-sorted by priority
    tags.push(tr(
        Sel::TagAttrExists("a", "href"),
        Target::Mark(m("link")),
        Some(ga_link),
    ));
    tags.push(tr(
        Sel::Tag("span"),
        Target::Mark(m("textStyle")),
        Some(ga_textstyle),
    ));
    tags.push(tr(Sel::Tag("strong"), Target::Mark(m("bold")), None));
    tags.push(tr(Sel::Tag("b"), Target::Mark(m("bold")), Some(ga_bold_b)));
    tags.push(tr(Sel::Tag("em"), Target::Mark(m("italic")), None));
    tags.push(tr(
        Sel::Tag("i"),
        Target::Mark(m("italic")),
        Some(ga_italic_i),
    ));
    tags.push(tr(Sel::Tag("s"), Target::Mark(m("strike")), None));
    tags.push(tr(Sel::Tag("del"), Target::Mark(m("strike")), None));
    tags.push(tr(Sel::Tag("strike"), Target::Mark(m("strike")), None));
    tags.push(tr(Sel::Tag("u"), Target::Mark(m("underline")), None));
    tags.push(tr(Sel::Tag("code"), Target::Mark(m("code")), None));
    // customColor: `node.getAttribute("data-text-color") && null` can never be `false`: matches EVERY span
    tags.push(tr(
        Sel::Tag("span"),
        Target::Mark(m("customColor")),
        Some(ga_always),
    ));
    tags.push(tr(
        Sel::Tag("span"),
        Target::Mark(m("customColor")),
        Some(ga_always),
    ));

    tags.push(tr(Sel::Tag("p"), Target::Node(n("paragraph")), None));
    tags.push(tr(
        Sel::Tag("mention-component"),
        Target::Node(n("mention")),
        None,
    ));
    tags.push(tr(Sel::Tag("ul"), Target::Node(n("bulletList")), None));
    tags.push(tr(Sel::Tag("br"), Target::Node(n("hardBreak")), None));
    for (i, h) in ["h1", "h2", "h3", "h4", "h5", "h6"].iter().enumerate() {
        let mut r = tr(Sel::Tag(h), Target::Node(n("heading")), None);
        let mut a = Attrs::new();
        a.insert("level".into(), Value::Number((i as i64 + 1).into()));
        r.static_attrs = Some(a);
        tags.push(r);
    }
    tags.push(tr(Sel::Tag("li"), Target::Node(n("listItem")), None));
    tags.push(tr(Sel::Tag("ol"), Target::Node(n("orderedList")), None));
    tags.push(tr(
        Sel::TagAttrEq("span", "data-type", "emoji"),
        Target::Node(n("emoji")),
        None,
    ));
    tags.push(tr(
        Sel::Tag("blockquote"),
        Target::Node(n("blockquote")),
        None,
    ));
    tags.push(tr(
        Sel::TagAttrEq("div", "data-type", "horizontalRule"),
        Target::Node(n("horizontalRule")),
        None,
    ));
    tags.push(tr(Sel::Tag("hr"), Target::Node(n("horizontalRule")), None));
    tags.push(tr(Sel::ImgSrcNotData, Target::Node(n("image")), None));
    tags.push(tr(
        Sel::Tag("image-component"),
        Target::Node(n("imageComponent")),
        None,
    ));
    let mut pre = tr(Sel::Tag("pre"), Target::Node(n("codeBlock")), None);
    pre.preserve_ws = Some(true);
    pre.preserve_full = true;
    tags.push(pre);
    tags.push(tr(Sel::Tag("table"), Target::Node(n("table")), None));
    tags.push(tr(Sel::Tag("th"), Target::Node(n("tableHeader")), None));
    tags.push(tr(Sel::Tag("td"), Target::Node(n("tableCell")), None));
    tags.push(tr(Sel::Tag("tr"), Target::Node(n("tableRow")), None));
    tags.push(tr(
        Sel::TagAttrEq("div", "data-block-type", "callout-component"),
        Target::Node(n("calloutComponent")),
        None,
    ));
    if let Some(e) = schema.node_type("issue-embed-component") {
        tags.push(tr(Sel::Tag("issue-embed-component"), Target::Node(e), None));
    }
    // priority-51 rules (task list/item) sort to the front; `taskList` is inserted before `taskItem`
    let mut p51 = vec![];
    let mut rest = vec![];
    // task rules are declared on the nodes: they are inserted at their schema position with priority 51
    let mut tl = tr(
        Sel::TagAttrEq("ul", "data-type", "taskList"),
        Target::Node(n("taskList")),
        None,
    );
    tl.priority = 51;
    let mut ti = tr(
        Sel::TagAttrEq("li", "data-type", "taskItem"),
        Target::Node(n("taskItem")),
        None,
    );
    ti.priority = 51;
    p51.push(tl);
    p51.push(ti);
    rest.extend(tags);
    p51.extend(rest);
    let tags = p51;

    let styles = vec![
        StyleRule {
            style: "font-weight=400",
            mark: m("bold"),
            clear_mark: true,
            consuming: true,
            get_attrs: None,
        },
        StyleRule {
            style: "font-weight",
            mark: m("bold"),
            clear_mark: false,
            consuming: true,
            get_attrs: Some(sga_bold_weight),
        },
        StyleRule {
            style: "font-style=normal",
            mark: m("italic"),
            clear_mark: true,
            consuming: true,
            get_attrs: None,
        },
        StyleRule {
            style: "font-style=italic",
            mark: m("italic"),
            clear_mark: false,
            consuming: true,
            get_attrs: None,
        },
        StyleRule {
            style: "text-decoration",
            mark: m("strike"),
            clear_mark: false,
            consuming: false,
            get_attrs: Some(sga_strike),
        },
        StyleRule {
            style: "text-decoration",
            mark: m("underline"),
            clear_mark: false,
            consuming: false,
            get_attrs: Some(sga_underline),
        },
    ];
    let mut matched: Vec<&'static str> = vec![];
    for s in &styles {
        let prop = s.style.split('=').next().unwrap();
        if !matched.contains(&prop) {
            matched.push(prop);
        }
    }
    (tags, styles, matched)
}

// ------------------------------------------------------------------------------------------------
// extension attribute parsing (Tiptap `injectExtensionAttributesToParseRule`)
// ------------------------------------------------------------------------------------------------

/// Tiptap `fromString`
fn from_string(s: &str) -> Value {
    let b = s.as_bytes();
    // /^[+-]?(?:\d*\.)?\d+$/
    let mut i = 0;
    if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
        i += 1;
    }
    let start = i;
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
    }
    let numeric = if i < b.len() && b[i] == b'.' {
        let j = i + 1;
        let mut k = j;
        while k < b.len() && b[k].is_ascii_digit() {
            k += 1;
        }
        k == b.len() && k > j
    } else {
        i == b.len() && i > start
    };
    if numeric {
        if let Ok(f) = s.parse::<f64>() {
            return crate::pm::norm_num(
                serde_json::Number::from_f64(f)
                    .map(Value::Number)
                    .unwrap_or(Value::Null),
            );
        }
    }
    match s {
        "true" => Value::Bool(true),
        "false" => Value::Bool(false),
        _ => Value::String(s.to_string()),
    }
}

#[derive(Clone, Copy)]
enum Ext {
    /// `fromString(getAttribute(name))`
    Default(&'static str),
    TextAlign,
    OlStart,
    OlType,
    TaskChecked,
    EmojiName,
    CodeLanguage,
    ColWidth,
    ColorText,
    ColorBg,
}

fn js_parse_int(s: &str) -> Option<i64> {
    let t = s.trim_start_matches(|c: char| c.is_whitespace() || c == '\u{feff}');
    let (neg, rest) = match t.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, t.strip_prefix('+').unwrap_or(t)),
    };
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return None;
    }
    let v: i64 = digits.parse().ok()?;
    Some(if neg { -v } else { v })
}

fn run_ext(dom: &Dom, el: usize, e: Ext) -> Option<(String, Value)> {
    match e {
        Ext::Default(name) => dom
            .get_attribute(el, name)
            .map(|s| (name.to_string(), from_string(&s))),
        Ext::TextAlign => {
            let st = dom.style(el);
            let a = st.get("textAlign").unwrap_or("");
            if ["left", "center", "right"].contains(&a) {
                Some(("textAlign".into(), Value::String(a.to_string())))
            } else {
                None // defaultAlignment is null
            }
        }
        Ext::OlStart => {
            let v = if dom.has_attribute(el, "start") {
                match js_parse_int(&dom.get_attribute(el, "start").unwrap_or_default()) {
                    Some(i) => Value::Number(i.into()),
                    None => Value::Null, // NaN
                }
            } else {
                Value::Number(1.into())
            };
            Some(("start".into(), v))
        }
        Ext::OlType => dom
            .get_attribute(el, "type")
            .map(|s| ("type".to_string(), Value::String(s))),
        Ext::TaskChecked => {
            let d = dom.get_attribute(el, "data-checked");
            Some((
                "checked".into(),
                Value::Bool(d.as_deref() == Some("") || d.as_deref() == Some("true")),
            ))
        }
        Ext::EmojiName => match dom.dataset(el, "name") {
            Some(AttrV::Str(s)) => Some(("name".into(), Value::String(s))),
            Some(AttrV::True) => Some(("name".into(), Value::Bool(true))),
            None => None,
        },
        // `element.firstElementChild?.classList` + spread: zeed-dom has no `firstElementChild`, so the real server
        // code always yields no language (measured: tests/corpus f-pre-code -> language null).
        Ext::CodeLanguage => None,
        Ext::ColWidth => {
            let cw = dom.get_attribute(el, "colwidth");
            match cw {
                Some(s) if !s.is_empty() => Some((
                    "colwidth".into(),
                    match js_parse_int(&s) {
                        Some(i) => Value::Array(vec![Value::Number(i.into())]),
                        None => Value::Array(vec![Value::Null]),
                    },
                )),
                _ => None,
            }
        }
        Ext::ColorText => dom
            .get_attribute(el, "data-text-color")
            .map(|s| ("color".to_string(), Value::String(s))),
        Ext::ColorBg => dom
            .get_attribute(el, "data-background-color")
            .map(|s| ("backgroundColor".to_string(), Value::String(s))),
    }
}

fn ext_attrs(schema: &Schema, t: Target) -> Vec<Ext> {
    use Ext::*;
    match t {
        Target::Mark(m) => match schema.marks[m].name {
            "link" => vec![
                Default("href"),
                Default("target"),
                Default("rel"),
                Default("class"),
            ],
            "customColor" => vec![ColorText, ColorBg],
            _ => vec![],
        },
        Target::Node(n) => match schema.nodes[n].name {
            "paragraph" => vec![TextAlign],
            "heading" => vec![TextAlign, Default("level")],
            "orderedList" => vec![OlStart, OlType],
            "taskItem" => vec![TaskChecked],
            "image" => vec![
                Default("src"),
                Default("alt"),
                Default("title"),
                Default("width"),
                Default("height"),
                Default("aspectRatio"),
                Default("alignment"),
            ],
            "imageComponent" => vec![
                Default("src"),
                Default("alt"),
                Default("title"),
                Default("id"),
                Default("width"),
                Default("height"),
                Default("aspectRatio"),
                Default("alignment"),
                Default("status"),
            ],
            "mention" => vec![
                Default("id"),
                Default("entity_identifier"),
                Default("entity_name"),
            ],
            "emoji" => vec![EmojiName],
            "codeBlock" => vec![CodeLanguage],
            "tableRow" => vec![Default("background"), Default("textColor")],
            "tableHeader" => vec![
                Default("colspan"),
                Default("rowspan"),
                ColWidth,
                Default("background"),
            ],
            "tableCell" => vec![
                Default("colspan"),
                Default("rowspan"),
                ColWidth,
                Default("background"),
                Default("textColor"),
            ],
            "calloutComponent" => vec![
                Default("id"),
                Default("data-icon-color"),
                Default("data-icon-name"),
                Default("data-emoji-unicode"),
                Default("data-emoji-url"),
                Default("data-logo-in-use"),
                Default("data-background"),
                Default("data-block-type"),
            ],
            "issue-embed-component" => vec![
                Default("entity_identifier"),
                Default("project_identifier"),
                Default("workspace_identifier"),
                Default("id"),
                Default("entity_name"),
            ],
            _ => vec![],
        },
    }
}

// ------------------------------------------------------------------------------------------------
// the parse context (ProseMirror ParseContext / NodeContext)
// ------------------------------------------------------------------------------------------------

struct NodeContext {
    id: u32,
    ty: Option<TypeId>,
    attrs: Attrs,
    marks: Vec<Mark>,
    solid: bool,
    /// content-match state; None = JS `null`
    matcher: Option<usize>,
    content: Vec<Node>,
    options: u8,
}

struct Parser<'a> {
    schema: &'a Schema,
    dom: &'a mut Dom,
    tags: Vec<TagRule>,
    styles: Vec<StyleRule>,
    matched_styles: Vec<&'static str>,
    normalize_lists: bool,
    nodes: Vec<NodeContext>,
    open: usize,
    needs_block: bool,
    local_preserve_ws: bool,
    next_id: u32,
}

fn ws_options_for(
    schema: &Schema,
    ty: Option<TypeId>,
    preserve: Option<(bool, bool)>,
    base: u8,
) -> u8 {
    if let Some((p, full)) = preserve {
        return (if p { OPT_PRESERVE_WS } else { 0 })
            | (if full { OPT_PRESERVE_WS_FULL } else { 0 });
    }
    let _ = (schema, ty);
    // no node type in this schema has `whitespace: "pre"`
    base & !OPT_OPEN_LEFT
}

impl<'a> Parser<'a> {
    fn new_ctx(
        &mut self,
        ty: Option<TypeId>,
        attrs: Attrs,
        marks: Vec<Mark>,
        solid: bool,
        matcher: Option<usize>,
        options: u8,
    ) -> NodeContext {
        self.next_id += 1;
        let m = match (matcher, ty) {
            (Some(m), _) => Some(m),
            (None, Some(t)) if options & OPT_OPEN_LEFT == 0 => self.schema.nodes[t].content_match,
            _ => None,
        };
        NodeContext {
            id: self.next_id,
            ty,
            attrs,
            marks,
            solid,
            matcher: m,
            content: vec![],
            options,
        }
    }

    fn top(&self) -> &NodeContext {
        &self.nodes[self.open]
    }

    // NodeContext.findWrapping
    fn ctx_find_wrapping(&mut self, depth: usize, node_ty: TypeId) -> Option<Vec<TypeId>> {
        let schema = self.schema;
        if self.nodes[depth].matcher.is_none() {
            let Some(t) = self.nodes[depth].ty else {
                return Some(vec![]);
            };
            let start = schema.nodes[t].content_match.unwrap();
            if let Some(fill) = schema.fill_before(start, &[node_ty], false) {
                self.nodes[depth].matcher = schema.match_types(start, &fill);
            } else if let Some(wrap) = schema.find_wrapping(start, node_ty) {
                self.nodes[depth].matcher = Some(start);
                return Some(wrap);
            } else {
                return None;
            }
        }
        schema.find_wrapping(self.nodes[depth].matcher.unwrap(), node_ty)
    }

    fn ctx_finish(&mut self, ctx: NodeContext, open_end: bool) -> Node {
        let schema = self.schema;
        let mut content = ctx.content;
        if ctx.options & OPT_PRESERVE_WS == 0 {
            if let Some(last) = content.last_mut() {
                if let Some(t) = last.text.as_mut() {
                    let trimmed_len = t.trim_end_matches(is_ws).len();
                    if trimmed_len == 0 {
                        content.pop();
                    } else if trimmed_len != t.len() {
                        t.truncate(trimmed_len);
                    }
                }
            }
        }
        let mut content = join_text(content);
        if !open_end {
            if let Some(m) = ctx.matcher {
                if let Some(fill) = schema.fill_before(m, &[], true) {
                    for t in fill {
                        if let Some(n) = Node::create_and_fill(schema, t) {
                            content.push(n);
                        }
                    }
                }
            }
        }
        match ctx.ty {
            Some(t) => {
                let mut n = Node::create(schema, t, &ctx.attrs, content);
                n.marks = ctx.marks;
                n
            }
            None => Node::create(schema, schema.top, &Attrs::new(), content), // unreachable for closed parses
        }
    }

    fn inline_context(&self, ctx_depth: usize, dom_node: usize) -> bool {
        let ctx = &self.nodes[ctx_depth];
        if let Some(t) = ctx.ty {
            return self.schema.inline_content(t);
        }
        if let Some(first) = ctx.content.first() {
            return self.schema.nodes[first.ty].inline;
        }
        match self.dom.nodes[dom_node].parent {
            Some(p) => !BLOCK_TAGS.contains(&self.dom.nodes[p].tag.to_lowercase().as_str()),
            None => false,
        }
    }

    // ---- DOM entry points -------------------------------------------------------------------------
    fn add_all(&mut self, parent: usize, marks: &[Mark]) {
        let kids = self.dom.nodes[parent].children.clone();
        for k in kids {
            self.add_dom(k, marks);
        }
    }

    fn add_dom(&mut self, n: usize, marks: &[Mark]) {
        match self.dom.nodes[n].kind {
            Kind::Text => self.add_text_node(n, marks),
            Kind::Element => self.add_element(n, marks, None),
            Kind::Fragment => {}
        }
    }

    fn add_text_node(&mut self, dom_node: usize, marks: &[Mark]) {
        let schema = self.schema;
        let mut value = self.dom.nodes[dom_node].text.clone();
        let top_opts = self.top().options;
        let preserve_ws_full = top_opts & OPT_PRESERVE_WS_FULL != 0;
        let preserve_ws =
            preserve_ws_full || self.local_preserve_ws || top_opts & OPT_PRESERVE_WS != 0;
        if preserve_ws_full
            || self.inline_context(self.open, dom_node)
            || value.chars().any(|c| !is_ws(c))
        {
            if !preserve_ws {
                // value.replace(/[ \t\r\n\u000c]+/g, " ")
                let mut out = String::with_capacity(value.len());
                let mut in_ws = false;
                for c in value.chars() {
                    if is_ws(c) {
                        if !in_ws {
                            out.push(' ');
                        }
                        in_ws = true;
                    } else {
                        in_ws = false;
                        out.push(c);
                    }
                }
                value = out;
                if value.starts_with(is_ws) && self.open == self.nodes.len() - 1 {
                    let node_before = self.top().content.last();
                    let dom_before = self.dom.previous_sibling(dom_node);
                    let before_is_br = dom_before
                        .map(|d| self.dom.nodes[d].tag == "BR")
                        .unwrap_or(false);
                    let strip = match node_before {
                        None => true,
                        Some(nb) => {
                            before_is_br
                                || nb
                                    .text
                                    .as_deref()
                                    .map(|t| t.ends_with(is_ws))
                                    .unwrap_or(false)
                        }
                    };
                    if strip {
                        value = value[1..].to_string();
                    }
                }
            } else if preserve_ws_full {
                value = value.replace("\r\n", "\n").replace('\r', "\n");
            } else if value.contains(['\r', '\n']) && self.linebreak_fits() {
                // schema.linebreakReplacement (hardBreak): newlines become hard breaks
                let hb = schema.node_type("hardBreak").unwrap();
                let norm = value.replace("\r\n", "\n").replace('\r', "\n");
                for (i, line) in norm.split('\n').enumerate() {
                    if i > 0 {
                        let b = Node::create(schema, hb, &Attrs::new(), vec![]);
                        self.insert_node(b, marks, true);
                    }
                    if !line.is_empty() {
                        if let Ok(t) = Node::text(schema, line, vec![]) {
                            self.insert_node(t, marks, !line.chars().any(|c| !c.is_whitespace()));
                        }
                    }
                }
                value = String::new();
            } else {
                value = value.replace("\r\n", " ").replace(['\r', '\n'], " ");
            }
            if !value.is_empty() {
                let has_non_ws = value.chars().any(|c| !c.is_whitespace());
                if let Ok(node) = Node::text(schema, &value, vec![]) {
                    self.insert_node(node, marks, !has_non_ws);
                }
            }
        }
    }

    /// `this.top.findWrapping(schema.linebreakReplacement.create())` is non-null
    fn linebreak_fits(&mut self) -> bool {
        let hb = self.schema.node_type("hardBreak").unwrap();
        let d = self.open;
        self.ctx_find_wrapping(d, hb).is_some()
    }

    fn match_tag(&self, el: usize, after: Option<usize>) -> Option<(usize, Attrs)> {
        let start = after.map(|a| a + 1).unwrap_or(0);
        for i in start..self.tags.len() {
            let rule = &self.tags[i];
            if !rule.sel.matches(self.dom, el) {
                continue;
            }
            let mut attrs = rule.static_attrs.clone().unwrap_or_default();
            if let Some(f) = rule.get_attrs {
                match f(self.dom, el) {
                    None => continue,
                    Some(a) => attrs = a,
                }
            }
            for e in ext_attrs(self.schema, rule.target) {
                if let Some((k, v)) = run_ext(self.dom, el, e) {
                    if !v.is_null() || matches!(e, Ext::OlStart) {
                        attrs.insert(k, v);
                    }
                }
            }
            return Some((i, attrs));
        }
        None
    }

    fn add_element(&mut self, el: usize, marks: &[Mark], match_after: Option<usize>) {
        let schema = self.schema;
        let outer_ws = self.local_preserve_ws;
        let top_depth = self.open;
        let top_id = self.nodes[top_depth].id;
        let tag_upper = self.dom.nodes[el].tag.clone();
        if tag_upper == "PRE"
            || self
                .dom
                .style(el)
                .get("whiteSpace")
                .map(|w| w.contains("pre"))
                .unwrap_or(false)
        {
            self.local_preserve_ws = true;
        }
        let name = tag_upper.to_lowercase();
        if (name == "ol" || name == "ul") && self.normalize_lists {
            self.normalize_list(el);
        }
        let matched = self.match_tag(el, match_after);
        if matched.is_none() && IGNORE_TAGS.contains(&name.as_str()) {
            // rule ? rule.ignore : ignoreTags.hasOwnProperty(name)
            self.ignore_fallback(el, marks);
        } else if matched.is_none() {
            let old_needs_block = self.needs_block;
            let mut sync = false;
            let mut top_id_eff = top_id;
            if BLOCK_TAGS.contains(&name.as_str()) {
                let top = &self.nodes[top_depth];
                if !top.content.is_empty()
                    && schema.nodes[top.content[0].ty].inline
                    && self.open > 0
                {
                    self.open -= 1;
                    top_id_eff = self.nodes[self.open].id;
                }
                sync = true;
                if self.nodes[self.open_for_id(top_id_eff)].ty.is_none() {
                    self.needs_block = true;
                }
            } else if self.dom.first_child(el).is_none() {
                self.leaf_fallback(el, marks);
                self.local_preserve_ws = outer_ws;
                return;
            }
            if let Some(inner) = self.read_styles(el, marks) {
                self.add_all(el, &inner);
            }
            if sync {
                self.sync_id(top_id_eff);
            }
            self.needs_block = old_needs_block;
        } else {
            let (ri, attrs) = matched.unwrap();
            if let Some(inner) = self.read_styles(el, marks) {
                let rule = self.tags[ri].clone();
                self.add_element_by_rule(el, &rule, attrs, inner);
            }
        }
        self.local_preserve_ws = outer_ws;
    }

    fn open_for_id(&self, id: u32) -> usize {
        self.nodes
            .iter()
            .position(|n| n.id == id)
            .unwrap_or(self.open)
    }

    fn leaf_fallback(&mut self, el: usize, marks: &[Mark]) {
        if self.dom.nodes[el].tag == "BR" {
            if let Some(t) = self.top().ty {
                if self.schema.inline_content(t) {
                    // addTextNode(createTextNode("\n"))
                    let id = self.dom.nodes.len();
                    self.dom.nodes.push(crate::vdom::DNode {
                        kind: Kind::Text,
                        parent: None,
                        children: vec![],
                        name: String::new(),
                        tag: "#text".into(),
                        attrs: vec![],
                        text: "\n".into(),
                    });
                    self.add_text_node(id, marks);
                }
            }
        }
    }

    fn ignore_fallback(&mut self, el: usize, marks: &[Mark]) {
        if self.dom.nodes[el].tag == "BR"
            && self
                .top()
                .ty
                .map(|t| !self.schema.inline_content(t))
                .unwrap_or(true)
        {
            if let Ok(t) = Node::text(self.schema, "-", vec![]) {
                self.find_place(&t, marks, true);
            }
        }
    }

    fn read_styles(&mut self, el: usize, marks: &[Mark]) -> Option<Vec<Mark>> {
        let styles = self.dom.style(el);
        let mut marks: Vec<Mark> = marks.to_vec();
        if styles.count > 0 {
            for name in self.matched_styles.clone() {
                let Some(value) = styles.get_property_value(name).map(|s| s.to_string()) else {
                    continue;
                };
                if value.is_empty() {
                    continue;
                }
                let mut after: Option<usize> = None;
                loop {
                    // matchStyle(prop, value, ctx, after)
                    let mut found: Option<(usize, Option<Attrs>)> = None;
                    let start = after.map(|a| a + 1).unwrap_or(0);
                    for i in start..self.styles.len() {
                        let r = &self.styles[i];
                        if !r.style.starts_with(name) {
                            continue;
                        }
                        if r.style.len() > name.len()
                            && (r.style.as_bytes()[name.len()] != b'='
                                || &r.style[name.len() + 1..] != value)
                        {
                            continue;
                        }
                        let mut attrs = None;
                        if let Some(f) = r.get_attrs {
                            match f(&value) {
                                None => continue,
                                Some(a) => attrs = Some(a),
                            }
                        }
                        found = Some((i, attrs));
                        break;
                    }
                    let Some((ri, attrs)) = found else { break };
                    let rule = self.styles[ri].clone();
                    if rule.clear_mark {
                        marks.retain(|m| m.ty != rule.mark);
                    } else {
                        marks.push(Mark::create(
                            self.schema,
                            rule.mark,
                            &attrs.unwrap_or_default(),
                        ));
                    }
                    if !rule.consuming {
                        after = Some(ri);
                    } else {
                        break;
                    }
                }
            }
        }
        Some(marks)
    }

    fn add_element_by_rule(
        &mut self,
        el: usize,
        rule: &TagRule,
        rule_attrs: Attrs,
        mut marks: Vec<Mark>,
    ) {
        let schema = self.schema;
        let mut sync = false;
        let mut leaf = false;
        match rule.target {
            Target::Node(nt) => {
                if !schema.nodes[nt].is_leaf() {
                    let pw = rule.preserve_ws.map(|p| (p, rule.preserve_full));
                    if let Some(inner) = self.enter(nt, &rule_attrs, &marks, pw) {
                        sync = true;
                        marks = inner;
                    }
                } else {
                    leaf = true;
                    let node = Node::create(schema, nt, &rule_attrs, vec![]);
                    let is_br = self.dom.nodes[el].tag == "BR";
                    if !self.insert_node(node, &marks, is_br) {
                        self.leaf_fallback(el, &marks);
                    }
                }
            }
            Target::Mark(mt) => {
                marks.push(Mark::create(schema, mt, &rule_attrs));
            }
        }
        let start_in = self.nodes[self.open].id;
        if !leaf {
            self.add_all(el, &marks);
        }
        if sync && self.sync_id(start_in) {
            self.open -= 1;
        }
    }

    // ---- placing nodes ----------------------------------------------------------------------------
    fn find_place(&mut self, node: &Node, marks: &[Mark], cautious: bool) -> Option<Vec<Mark>> {
        let mut route: Option<Vec<TypeId>> = None;
        let mut sync_depth: Option<usize> = None;
        let mut penalty = 0usize;
        let mut depth = self.open as isize;
        while depth >= 0 {
            let d = depth as usize;
            let found = self.ctx_find_wrapping(d, node.ty);
            if let Some(f) = found {
                if route
                    .as_ref()
                    .map(|r| r.len() > f.len() + penalty)
                    .unwrap_or(true)
                {
                    let done = f.is_empty();
                    route = Some(f);
                    sync_depth = Some(d);
                    if done {
                        break;
                    }
                }
            }
            if self.nodes[d].solid {
                if cautious {
                    break;
                }
                penalty += 2;
            }
            depth -= 1;
        }
        let route = route?;
        let sd = sync_depth.unwrap();
        let sid = self.nodes[sd].id;
        self.sync_id(sid);
        let mut marks = marks.to_vec();
        for t in route {
            marks = self.enter_inner(t, &Attrs::new(), &marks, false, None);
        }
        Some(marks)
    }

    fn mark_may_apply(&self, mark: usize, node_ty: TypeId) -> bool {
        let schema = self.schema;
        for parent in 0..schema.nodes.len() {
            if !schema.allows_mark_type(parent, mark) {
                continue;
            }
            let Some(start) = schema.nodes[parent].content_match else {
                continue;
            };
            let mut seen = vec![start];
            let mut stack = vec![start];
            while let Some(s) = stack.pop() {
                for (t, next) in &schema.states[s].next {
                    if *t == node_ty {
                        return true;
                    }
                    if !seen.contains(next) {
                        seen.push(*next);
                        stack.push(*next);
                    }
                }
            }
        }
        false
    }

    fn insert_node(&mut self, node: Node, marks: &[Mark], cautious: bool) -> bool {
        let schema = self.schema;
        let mut marks = marks.to_vec();
        if schema.nodes[node.ty].inline && self.needs_block && self.top().ty.is_none() {
            if let Some(block) = self.textblock_from_context() {
                marks = self.enter_inner(block, &Attrs::new(), &marks, false, None);
            }
        }
        let Some(inner) = self.find_place(&node, &marks, cautious) else {
            return false;
        };
        self.close_extra(false);
        let top_ty = self.top().ty;
        if let Some(m) = self.nodes[self.open].matcher {
            self.nodes[self.open].matcher = schema.match_type(m, node.ty);
        }
        let mut node_marks: Vec<Mark> = vec![];
        let all: Vec<Mark> = inner
            .into_iter()
            .chain(node.marks.iter().cloned())
            .collect();
        for m in all {
            let ok = match top_ty {
                Some(t) => schema.allows_mark_type(t, m.ty),
                None => self.mark_may_apply(m.ty, node.ty),
            };
            if ok {
                node_marks = m.add_to_set(schema, &node_marks);
            }
        }
        let mut node = node;
        node.marks = mark_set_from(schema, node_marks);
        self.nodes[self.open].content.push(node);
        true
    }

    fn textblock_from_context(&self) -> Option<TypeId> {
        (0..self.schema.nodes.len()).find(|t| self.schema.is_textblock(*t))
    }

    fn enter(
        &mut self,
        ty: TypeId,
        attrs: &Attrs,
        marks: &[Mark],
        preserve: Option<(bool, bool)>,
    ) -> Option<Vec<Mark>> {
        let probe = Node::create(self.schema, ty, attrs, vec![]);
        let inner = self.find_place(&probe, marks, false);
        inner.map(|_| self.enter_inner(ty, attrs, marks, true, preserve))
    }

    fn enter_inner(
        &mut self,
        ty: TypeId,
        attrs: &Attrs,
        marks: &[Mark],
        solid: bool,
        preserve: Option<(bool, bool)>,
    ) -> Vec<Mark> {
        let schema = self.schema;
        self.close_extra(false);
        let top_opts = self.top().options;
        if let Some(m) = self.nodes[self.open].matcher {
            self.nodes[self.open].matcher = schema.match_type(m, ty);
        }
        let mut options = ws_options_for(schema, Some(ty), preserve, top_opts);
        if top_opts & OPT_OPEN_LEFT != 0 && self.top().content.is_empty() {
            options |= OPT_OPEN_LEFT;
        }
        let mut apply_marks: Vec<Mark> = vec![];
        let top_ty = self.top().ty;
        let mut remaining = vec![];
        for m in marks {
            let allowed = match top_ty {
                Some(t) => schema.allows_mark_type(t, m.ty),
                None => self.mark_may_apply(m.ty, ty),
            };
            if allowed {
                apply_marks = m.add_to_set(schema, &apply_marks);
            } else {
                remaining.push(m.clone());
            }
        }
        let ctx = self.new_ctx(Some(ty), attrs.clone(), apply_marks, solid, None, options);
        self.nodes.push(ctx);
        self.open += 1;
        remaining
    }

    fn close_extra(&mut self, open_end: bool) {
        let mut i = self.nodes.len() - 1;
        if i > self.open {
            while i > self.open {
                let ctx = self.nodes.pop().unwrap();
                let node = self.ctx_finish(ctx, open_end);
                self.nodes[i - 1].content.push(node);
                i -= 1;
            }
        }
    }

    fn sync_id(&mut self, to: u32) -> bool {
        let mut i = self.open as isize;
        while i >= 0 {
            let idx = i as usize;
            if self.nodes[idx].id == to {
                self.open = idx;
                return true;
            } else if self.local_preserve_ws {
                self.nodes[idx].options |= OPT_PRESERVE_WS;
            }
            i -= 1;
        }
        false
    }

    fn normalize_list(&mut self, list: usize) {
        let kids = self.dom.nodes[list].children.clone();
        let mut prev_item: Option<usize> = None;
        for child in kids {
            if self.dom.nodes[child].kind != Kind::Element {
                continue; // name == null: no change to prev_item (JS: `else if (name)` branches only)
            }
            let name = self.dom.nodes[child].tag.to_lowercase();
            if (name == "ol" || name == "ul") && prev_item.is_some() {
                let pi = prev_item.unwrap();
                self.dom.append_child(pi, child);
                // JS: child = prevItem (then nextSibling of prevItem): continue from the item
            } else if name == "li" {
                prev_item = Some(child);
            } else {
                prev_item = None;
            }
        }
    }

    fn finish(mut self) -> Node {
        self.open = 0;
        self.close_extra(false);
        let ctx = self.nodes.remove(0);
        self.ctx_finish(ctx, false)
    }
}

/// `DOMParser.fromSchema(schema).parse(dom)`.
pub fn parse(schema: &Schema, dom: &mut Dom) -> Node {
    let (mut tags, styles, matched) = build_rules(schema);
    // stable sort by priority, descending (rules were assembled marks first, then nodes)
    let mut idx: Vec<usize> = (0..tags.len()).collect();
    idx.sort_by_key(|i| std::cmp::Reverse(tags[*i].priority));
    tags = idx.iter().map(|i| tags[*i].clone()).collect();
    let mut p = Parser {
        schema,
        dom,
        tags,
        styles,
        matched_styles: matched,
        normalize_lists: true,
        nodes: vec![],
        open: 0,
        needs_block: false,
        local_preserve_ws: false,
        next_id: 0,
    };
    let top = p.new_ctx(
        Some(schema.top),
        Attrs::new(),
        vec![],
        true,
        None,
        ws_options_for(schema, None, None, 0),
    );
    p.nodes.push(top);
    p.add_all(0, &[]);
    p.finish()
}

/// Convenience: zeed-dom parse + ProseMirror parse (`generateJSON`'s tree).
pub fn html_to_doc(schema: &Schema, html: &str) -> Node {
    let mut dom = crate::vdom::parse_html(html);
    parse(schema, &mut dom)
}
