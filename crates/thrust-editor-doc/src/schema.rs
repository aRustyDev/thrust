//! The Plane editor schema, transcribed by hand from `@plane/editor` (packages/editor/src/extensions) and
//! checked against a machine dump of the real schema (`tools/spike-0006/oracle/schema-dump.ts`) by
//! `Schema::diff_against_dump` so that drift is detected, not assumed away.
//!
//! Also ports ProseMirror's content-expression machinery (`prosemirror-model/src/content.ts`): the NFA/DFA
//! construction, `matchType`, `findWrapping`, `fillBefore`, `defaultType` that the DOM parser needs.

use serde_json::Value;
use std::collections::HashMap;

pub type TypeId = usize;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Variant {
    /// `CoreEditorExtensionsWithoutProps` (work item descriptions, comments).
    Rich,
    /// Core + `DocumentEditorExtensionsWithoutProps` (pages): adds `issue-embed-component`.
    Document,
}

#[derive(Clone, Debug, PartialEq)]
pub enum AttrDefault {
    Val(Value),
    /// JS `undefined`: the attribute key is absent from JSON output.
    Undefined,
}

#[derive(Clone, Debug)]
pub struct AttrSpec {
    pub name: &'static str,
    pub default: AttrDefault,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MarksSpec {
    /// spec `marks` absent: textblocks allow every mark, everything else none.
    Default,
    /// spec `marks: ""`.
    None,
}

#[derive(Clone, Debug)]
pub struct NodeType {
    pub name: &'static str,
    pub id: TypeId,
    pub content: &'static str,
    pub groups: &'static [&'static str],
    pub attrs: Vec<AttrSpec>,
    pub is_text: bool,
    pub inline: bool,
    /// spec `atom` (leaf nodes are atoms too).
    pub atom: bool,
    pub marks: MarksSpec,
    pub code: bool,
    pub defining: bool,
    pub isolating: bool,
    /// start state of this type's content DFA (None for leaves).
    pub content_match: Option<usize>,
}

impl NodeType {
    pub fn is_leaf(&self) -> bool {
        self.content_match.is_none()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Excludes {
    /// default: a mark excludes marks of its own type.
    Itself,
    /// `excludes: "_"`.
    All,
}

#[derive(Clone, Debug)]
pub struct MarkType {
    pub name: &'static str,
    pub rank: usize,
    pub attrs: Vec<AttrSpec>,
    pub excludes: Excludes,
    pub inclusive: Option<bool>,
}

/// One state of a content DFA (ProseMirror `ContentMatch`).
#[derive(Clone, Debug, Default)]
pub struct MatchState {
    pub valid_end: bool,
    pub next: Vec<(TypeId, usize)>,
}

#[derive(Clone, Debug)]
pub struct Schema {
    pub variant: Variant,
    pub nodes: Vec<NodeType>,
    pub marks: Vec<MarkType>,
    pub states: Vec<MatchState>,
    by_name: HashMap<&'static str, TypeId>,
    mark_by_name: HashMap<&'static str, usize>,
    pub top: TypeId,
}

// ---------------------------------------------------------------------------------------------
// raw definitions (order matters: it is the order of `schema.nodes` in the real schema)
// ---------------------------------------------------------------------------------------------

fn v(s: &str) -> AttrDefault {
    AttrDefault::Val(serde_json::from_str(s).expect("static default json"))
}
fn a(name: &'static str, d: AttrDefault) -> AttrSpec {
    AttrSpec { name, default: d }
}

struct Raw {
    name: &'static str,
    content: &'static str,
    groups: &'static [&'static str],
    attrs: Vec<AttrSpec>,
    inline: bool,
    atom: bool,
    marks: MarksSpec,
    code: bool,
    defining: bool,
    isolating: bool,
}

#[allow(clippy::too_many_arguments)]
fn raw(
    name: &'static str,
    content: &'static str,
    groups: &'static [&'static str],
    attrs: Vec<AttrSpec>,
    inline: bool,
    atom: bool,
    marks: MarksSpec,
    code: bool,
    defining: bool,
    isolating: bool,
) -> Raw {
    Raw {
        name,
        content,
        groups,
        attrs,
        inline,
        atom,
        marks,
        code,
        defining,
        isolating,
    }
}

const CALLOUT_URL: &str =
    "https://cdn.jsdelivr.net/npm/emoji-datasource-apple/img/apple/64/1f4a1.png";

fn raw_nodes(variant: Variant) -> Vec<Raw> {
    use MarksSpec::{Default as MD, None as MN};
    let mut n = vec![
        raw(
            "paragraph",
            "inline*",
            &["block"],
            vec![a("textAlign", v("null"))],
            false,
            false,
            MD,
            false,
            false,
            false,
        ),
        raw(
            "mention",
            "",
            &["inline"],
            vec![
                a("id", v("null")),
                a("entity_identifier", v("null")),
                a("entity_name", v("null")),
            ],
            true,
            true,
            MD,
            false,
            false,
            false,
        ),
        raw(
            "bulletList",
            "listItem+",
            &["block", "list"],
            vec![],
            false,
            false,
            MD,
            false,
            false,
            false,
        ),
        raw(
            "doc",
            "block+",
            &[],
            vec![],
            false,
            false,
            MD,
            false,
            false,
            false,
        ),
        raw(
            "hardBreak",
            "",
            &["inline"],
            vec![],
            true,
            true,
            MD,
            false,
            false,
            false,
        ),
        raw(
            "heading",
            "inline*",
            &["block"],
            vec![a("textAlign", v("null")), a("level", v("1"))],
            false,
            false,
            MD,
            false,
            true,
            false,
        ),
        raw(
            "listItem",
            "paragraph block*",
            &[],
            vec![],
            false,
            false,
            MD,
            false,
            true,
            false,
        ),
        raw(
            "orderedList",
            "listItem+",
            &["block", "list"],
            vec![a("start", v("1")), a("type", v("null"))],
            false,
            false,
            MD,
            false,
            false,
            false,
        ),
        raw(
            "text",
            "",
            &["inline"],
            vec![],
            true,
            true,
            MD,
            false,
            false,
            false,
        ),
        raw(
            "emoji",
            "",
            &["inline"],
            vec![a("name", v("null"))],
            true,
            true,
            MD,
            false,
            false,
            false,
        ),
        raw(
            "blockquote",
            "block+",
            &["block"],
            vec![],
            false,
            false,
            MD,
            false,
            true,
            false,
        ),
        raw(
            "horizontalRule",
            "",
            &["block"],
            vec![],
            false,
            true,
            MD,
            false,
            false,
            false,
        ),
        raw(
            "image",
            "",
            &["block"],
            vec![
                a("src", v("null")),
                a("alt", v("null")),
                a("title", v("null")),
                a("width", v("\"35%\"")),
                a("height", v("null")),
                a("aspectRatio", v("null")),
                a("alignment", v("\"left\"")),
            ],
            false,
            true,
            MD,
            false,
            false,
            false,
        ),
        raw(
            "imageComponent",
            "",
            &["block"],
            vec![
                a("src", v("null")),
                a("alt", v("null")),
                a("title", v("null")),
                a("id", v("null")),
                a("width", v("\"35%\"")),
                a("height", v("\"auto\"")),
                a("aspectRatio", v("null")),
                a("alignment", v("\"left\"")),
                a("status", v("\"pending\"")),
            ],
            false,
            true,
            MD,
            false,
            false,
            false,
        ),
        raw(
            "taskList",
            "taskItem+",
            &["block", "list"],
            vec![],
            false,
            false,
            MD,
            false,
            false,
            false,
        ),
        raw(
            "taskItem",
            "paragraph block*",
            &[],
            vec![a("checked", v("false"))],
            false,
            false,
            MD,
            false,
            true,
            false,
        ),
        raw(
            "codeBlock",
            "text*",
            &["block"],
            vec![a("language", v("null"))],
            false,
            false,
            MN,
            true,
            true,
            false,
        ),
        raw(
            "table",
            "tableRow+",
            &["block"],
            vec![],
            false,
            false,
            MD,
            false,
            false,
            true,
        ),
        raw(
            "tableHeader",
            "block+",
            &[],
            vec![
                a("colspan", v("1")),
                a("rowspan", v("1")),
                a("colwidth", v("[150]")),
                a("background", v("\"none\"")),
            ],
            false,
            false,
            MD,
            false,
            false,
            true,
        ),
        raw(
            "tableCell",
            "block+",
            &[],
            vec![
                a("colspan", v("1")),
                a("rowspan", v("1")),
                a("colwidth", v("[150]")),
                a("background", v("null")),
                a("textColor", v("null")),
            ],
            false,
            false,
            MD,
            false,
            false,
            true,
        ),
        raw(
            "tableRow",
            "(tableCell | tableHeader)*",
            &[],
            vec![a("background", v("null")), a("textColor", v("null"))],
            false,
            false,
            MD,
            false,
            false,
            false,
        ),
        raw(
            "calloutComponent",
            "block+",
            &["block"],
            vec![
                a("id", v("null")),
                a("data-icon-color", AttrDefault::Undefined),
                a("data-icon-name", AttrDefault::Undefined),
                a("data-emoji-unicode", v("\"128161\"")),
                a(
                    "data-emoji-url",
                    AttrDefault::Val(Value::String(CALLOUT_URL.into())),
                ),
                a("data-logo-in-use", v("\"emoji\"")),
                a("data-background", AttrDefault::Undefined),
                a("data-block-type", v("\"callout-component\"")),
            ],
            false,
            false,
            MD,
            false,
            false,
            false,
        ),
    ];
    if variant == Variant::Document {
        n.push(raw(
            "issue-embed-component",
            "",
            &["block"],
            vec![
                a("entity_identifier", AttrDefault::Undefined),
                a("project_identifier", AttrDefault::Undefined),
                a("workspace_identifier", AttrDefault::Undefined),
                a("id", AttrDefault::Undefined),
                a("entity_name", AttrDefault::Undefined),
            ],
            false,
            true,
            MD,
            false,
            false,
            false,
        ));
    }
    n
}

fn raw_marks() -> Vec<MarkType> {
    let m = |name: &'static str,
             attrs: Vec<AttrSpec>,
             excludes: Excludes,
             inclusive: Option<bool>| MarkType {
        name,
        rank: 0,
        attrs,
        excludes,
        inclusive,
    };
    let mut marks = vec![
        m(
            "link",
            vec![
                a("href", v("null")),
                a("target", v("\"_blank\"")),
                a("rel", v("\"noopener noreferrer nofollow\"")),
                a("class", v("\"text-accent-secondary underline underline-offset-[3px] hover:text-accent-primary transition-colors cursor-pointer\"")),
            ],
            Excludes::Itself,
            Some(false),
        ),
        m("textStyle", vec![], Excludes::Itself, None),
        m("bold", vec![], Excludes::Itself, None),
        m("italic", vec![], Excludes::Itself, None),
        m("strike", vec![], Excludes::Itself, None),
        m("underline", vec![], Excludes::Itself, None),
        m("code", vec![], Excludes::All, None),
        m("customColor", vec![a("color", v("null")), a("backgroundColor", v("null"))], Excludes::Itself, None),
    ];
    for (i, mk) in marks.iter_mut().enumerate() {
        mk.rank = i;
    }
    marks
}

// ---------------------------------------------------------------------------------------------
// content expressions (port of prosemirror-model content.ts)
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone)]
enum Expr {
    Choice(Vec<Expr>),
    Seq(Vec<Expr>),
    Plus(Box<Expr>),
    Star(Box<Expr>),
    Opt(Box<Expr>),
    Name(TypeId),
}

struct Parser<'a> {
    toks: Vec<String>,
    pos: usize,
    names: &'a HashMap<&'static str, TypeId>,
    groups: &'a HashMap<&'static str, Vec<TypeId>>,
}

fn tokenize(s: &str) -> Vec<String> {
    let mut out = vec![];
    let mut cur = String::new();
    for ch in s.chars() {
        if ch.is_alphanumeric() || ch == '_' || ch == '-' {
            cur.push(ch);
        } else {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            if !ch.is_whitespace() {
                out.push(ch.to_string());
            }
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

impl Parser<'_> {
    fn eat(&mut self, t: &str) -> bool {
        if self.toks.get(self.pos).map(|s| s.as_str()) == Some(t) {
            self.pos += 1;
            true
        } else {
            false
        }
    }
    fn expr(&mut self) -> Expr {
        let mut exprs = vec![self.seq()];
        while self.eat("|") {
            exprs.push(self.seq());
        }
        if exprs.len() == 1 {
            exprs.pop().unwrap()
        } else {
            Expr::Choice(exprs)
        }
    }
    fn seq(&mut self) -> Expr {
        let mut exprs = vec![];
        loop {
            exprs.push(self.subscript());
            match self.toks.get(self.pos).map(|s| s.as_str()) {
                Some(")") | Some("|") | None => break,
                _ => {}
            }
        }
        if exprs.len() == 1 {
            exprs.pop().unwrap()
        } else {
            Expr::Seq(exprs)
        }
    }
    fn subscript(&mut self) -> Expr {
        let mut e = self.atom();
        loop {
            if self.eat("+") {
                e = Expr::Plus(Box::new(e));
            } else if self.eat("*") {
                e = Expr::Star(Box::new(e));
            } else if self.eat("?") {
                e = Expr::Opt(Box::new(e));
            } else {
                break;
            }
        }
        e
    }
    fn atom(&mut self) -> Expr {
        if self.eat("(") {
            let e = self.expr();
            assert!(self.eat(")"), "missing ) in content expression");
            return e;
        }
        let name = self.toks[self.pos].clone();
        self.pos += 1;
        if let Some(id) = self.names.get(name.as_str()) {
            return Expr::Name(*id);
        }
        if let Some(members) = self.groups.get(name.as_str()) {
            return Expr::Choice(members.iter().map(|id| Expr::Name(*id)).collect());
        }
        panic!("no node type or group `{name}` in content expression");
    }
}

#[derive(Clone)]
struct Edge {
    term: Option<TypeId>,
    to: usize,
}

struct Nfa {
    nodes: Vec<Vec<Edge>>,
}

impl Nfa {
    fn node(&mut self) -> usize {
        self.nodes.push(vec![]);
        self.nodes.len() - 1
    }
    /// returns handles (from, index) of dangling edges
    fn edge(&mut self, from: usize, to: usize, term: Option<TypeId>) -> (usize, usize) {
        self.nodes[from].push(Edge { term, to });
        (from, self.nodes[from].len() - 1)
    }
    fn connect(&mut self, edges: &[(usize, usize)], to: usize) {
        for (f, i) in edges {
            self.nodes[*f][*i].to = to;
        }
    }
    fn compile(&mut self, expr: &Expr, from: usize) -> Vec<(usize, usize)> {
        match expr {
            Expr::Choice(exprs) => {
                let mut out = vec![];
                for e in exprs {
                    out.extend(self.compile(e, from));
                }
                out
            }
            Expr::Seq(exprs) => {
                let mut from = from;
                for (i, e) in exprs.iter().enumerate() {
                    let next = self.compile(e, from);
                    if i == exprs.len() - 1 {
                        return next;
                    }
                    from = self.node();
                    self.connect(&next, from);
                }
                unreachable!()
            }
            Expr::Star(e) => {
                let lp = self.node();
                self.edge(from, lp, None);
                let inner = self.compile(e, lp);
                self.connect(&inner, lp);
                vec![self.edge(lp, usize::MAX, None)]
            }
            Expr::Plus(e) => {
                let lp = self.node();
                let first = self.compile(e, from);
                self.connect(&first, lp);
                let second = self.compile(e, lp);
                self.connect(&second, lp);
                vec![self.edge(lp, usize::MAX, None)]
            }
            Expr::Opt(e) => {
                let mut out = vec![self.edge(from, usize::MAX, None)];
                out.extend(self.compile(e, from));
                out
            }
            Expr::Name(id) => vec![self.edge(from, usize::MAX, Some(*id))],
        }
    }
    fn null_from(&self, node: usize) -> Vec<usize> {
        let mut result = vec![];
        self.scan(node, &mut result);
        result.sort_unstable();
        result
    }
    fn scan(&self, node: usize, result: &mut Vec<usize>) {
        let edges = &self.nodes[node];
        if edges.len() == 1 && edges[0].term.is_none() {
            return self.scan(edges[0].to, result);
        }
        result.push(node);
        for e in edges {
            if e.term.is_none() && !result.contains(&e.to) {
                self.scan(e.to, result);
            }
        }
    }
}

fn build_dfa(
    nfa: &Nfa,
    states: &mut Vec<MatchState>,
    labeled: &mut HashMap<Vec<usize>, usize>,
    set: Vec<usize>,
) -> usize {
    let mut out: Vec<(TypeId, Vec<usize>)> = vec![];
    for node in &set {
        for e in &nfa.nodes[*node] {
            let Some(term) = e.term else { continue };
            let idx = match out.iter().position(|(t, _)| *t == term) {
                Some(i) => i,
                None => {
                    out.push((term, vec![]));
                    out.len() - 1
                }
            };
            for n in nfa.null_from(e.to) {
                if !out[idx].1.contains(&n) {
                    out[idx].1.push(n);
                }
            }
        }
    }
    let id = states.len();
    states.push(MatchState {
        valid_end: set.contains(&(nfa.nodes.len() - 1)),
        next: vec![],
    });
    labeled.insert(set, id);
    for (term, mut s) in out {
        s.sort_unstable();
        let next = match labeled.get(&s) {
            Some(i) => *i,
            None => build_dfa(nfa, states, labeled, s),
        };
        states[id].next.push((term, next));
    }
    id
}

// ---------------------------------------------------------------------------------------------

impl Schema {
    pub fn new(variant: Variant) -> Schema {
        let raws = raw_nodes(variant);
        let mut by_name = HashMap::new();
        let mut groups: HashMap<&'static str, Vec<TypeId>> = HashMap::new();
        for (i, r) in raws.iter().enumerate() {
            by_name.insert(r.name, i);
            for g in r.groups {
                groups.entry(g).or_default().push(i);
            }
        }
        let mut states: Vec<MatchState> = vec![];
        let mut nodes = vec![];
        for (i, r) in raws.iter().enumerate() {
            let content_match = if r.content.is_empty() {
                None
            } else {
                let mut p = Parser {
                    toks: tokenize(r.content),
                    pos: 0,
                    names: &by_name,
                    groups: &groups,
                };
                let expr = p.expr();
                let mut nfa = Nfa {
                    nodes: vec![vec![]],
                };
                let dangling = nfa.compile(&expr, 0);
                let end = nfa.node();
                nfa.connect(&dangling, end);
                let mut labeled = HashMap::new();
                let start = nfa.null_from(0);
                Some(build_dfa(&nfa, &mut states, &mut labeled, start))
            };
            let is_text = r.name == "text";
            let inline = r.inline || is_text;
            nodes.push(NodeType {
                name: r.name,
                id: i,
                content: r.content,
                groups: r.groups,
                attrs: r.attrs.clone(),
                is_text,
                inline,
                atom: r.atom,
                marks: r.marks,
                code: r.code,
                defining: r.defining,
                isolating: r.isolating,
                content_match,
            });
        }
        let marks = raw_marks();
        let mark_by_name = marks.iter().map(|m| (m.name, m.rank)).collect();
        let top = by_name["doc"];
        Schema {
            variant,
            nodes,
            marks,
            states,
            by_name,
            mark_by_name,
            top,
        }
    }

    pub fn node_type(&self, name: &str) -> Option<TypeId> {
        self.by_name.get(name).copied()
    }
    pub fn mark_type(&self, name: &str) -> Option<usize> {
        self.mark_by_name.get(name).copied()
    }
    pub fn node_name(&self, id: TypeId) -> &'static str {
        self.nodes[id].name
    }
    pub fn text_type(&self) -> TypeId {
        self.by_name["text"]
    }

    pub fn is_block(&self, id: TypeId) -> bool {
        !self.nodes[id].inline
    }
    /// ProseMirror `NodeType.inlineContent`: the first content edge is an inline type.
    pub fn inline_content(&self, id: TypeId) -> bool {
        match self.nodes[id].content_match {
            Some(s) => self.states[s]
                .next
                .first()
                .map(|(t, _)| self.nodes[*t].inline)
                .unwrap_or(false),
            None => false,
        }
    }
    pub fn is_textblock(&self, id: TypeId) -> bool {
        self.is_block(id) && self.inline_content(id)
    }
    pub fn in_group(&self, id: TypeId, group: &str) -> bool {
        self.nodes[id].groups.contains(&group)
    }

    /// `NodeType.allowsMarkType`
    pub fn allows_mark_type(&self, node: TypeId, _mark: usize) -> bool {
        match self.nodes[node].marks {
            MarksSpec::None => false,
            MarksSpec::Default => self.inline_content(node),
        }
    }

    // ----- ContentMatch API -----
    pub fn match_type(&self, state: usize, ty: TypeId) -> Option<usize> {
        self.states[state]
            .next
            .iter()
            .find(|(t, _)| *t == ty)
            .map(|(_, n)| *n)
    }
    pub fn match_types(&self, mut state: usize, types: &[TypeId]) -> Option<usize> {
        for t in types {
            state = self.match_type(state, *t)?;
        }
        Some(state)
    }
    pub fn valid_end(&self, state: usize) -> bool {
        self.states[state].valid_end
    }
    pub fn default_type(&self, state: usize) -> Option<TypeId> {
        self.states[state]
            .next
            .iter()
            .map(|(t, _)| *t)
            .find(|t| !self.nodes[*t].is_text)
    }

    /// `ContentMatch.findWrapping`: the chain of node types to open to make `target` fit at `state`.
    pub fn find_wrapping(&self, state: usize, target: TypeId) -> Option<Vec<TypeId>> {
        struct Active {
            m: usize,
            ty: Option<TypeId>,
            via: Option<usize>,
        }
        let mut seen: Vec<TypeId> = vec![];
        let mut active = vec![Active {
            m: state,
            ty: None,
            via: None,
        }];
        let mut head = 0;
        while head < active.len() {
            let (m, cur_ty) = (active[head].m, active[head].ty);
            if self.match_type(m, target).is_some() {
                let mut result = vec![];
                let mut i = head;
                while let Some(t) = active[i].ty {
                    result.push(t);
                    i = active[i].via.unwrap();
                }
                result.reverse();
                return Some(result);
            }
            for (t, next) in self.states[m].next.clone() {
                let nt = &self.nodes[t];
                if !nt.is_leaf()
                    && !seen.contains(&t)
                    && (cur_ty.is_none() || self.states[next].valid_end)
                {
                    active.push(Active {
                        m: nt.content_match.unwrap(),
                        ty: Some(t),
                        via: Some(head),
                    });
                    seen.push(t);
                }
            }
            head += 1;
        }
        None
    }

    /// `ContentMatch.fillBefore(after, toEnd)`: a list of node types to insert so `after` can follow `state`.
    /// Returns the types to create (each is then `createAndFill`ed).
    pub fn fill_before(&self, state: usize, after: &[TypeId], to_end: bool) -> Option<Vec<TypeId>> {
        let mut seen = vec![state];
        self.fill_search(state, after, to_end, &mut seen, vec![])
    }
    fn fill_search(
        &self,
        m: usize,
        after: &[TypeId],
        to_end: bool,
        seen: &mut Vec<usize>,
        types: Vec<TypeId>,
    ) -> Option<Vec<TypeId>> {
        if let Some(fin) = self.match_types(m, after) {
            if !to_end || self.states[fin].valid_end {
                return Some(types);
            }
        }
        for (t, next) in self.states[m].next.clone() {
            if !self.nodes[t].is_text && !seen.contains(&next) {
                seen.push(next);
                let mut tt = types.clone();
                tt.push(t);
                if let Some(found) = self.fill_search(next, after, to_end, seen, tt) {
                    return Some(found);
                }
            }
        }
        None
    }

    /// Drift detector: compare this transcription with the machine dump of the real schema.
    /// Returns human-readable mismatches (empty = identical for everything the dump records).
    pub fn diff_against_dump(&self, dump: &Value) -> Vec<String> {
        let mut out = vec![];
        let dn = dump["nodes"].as_object().expect("dump.nodes");
        let dm = dump["marks"].as_object().expect("dump.marks");
        let my_nodes: Vec<&str> = self.nodes.iter().map(|n| n.name).collect();
        let their_nodes: Vec<&str> = dn.keys().map(|s| s.as_str()).collect();
        if my_nodes != their_nodes {
            out.push(format!(
                "node list/order differs: rust={my_nodes:?} oracle={their_nodes:?}"
            ));
        }
        for n in &self.nodes {
            let Some(d) = dn.get(n.name) else { continue };
            let want_content = d["content"].as_str().unwrap_or("");
            if want_content != n.content {
                out.push(format!(
                    "node {}: content `{}` != oracle `{}`",
                    n.name, n.content, want_content
                ));
            }
            let want_group = d["group"].as_str().unwrap_or("");
            let have_group = n.groups.join(" ");
            if want_group != have_group {
                out.push(format!(
                    "node {}: group `{}` != oracle `{}`",
                    n.name, have_group, want_group
                ));
            }
            if d["inline"].as_bool() != Some(n.inline) {
                out.push(format!("node {}: inline differs", n.name));
            }
            let attrs = d["attrs"].as_object().cloned().unwrap_or_default();
            let have: Vec<&str> = n.attrs.iter().map(|x| x.name).collect();
            let want: Vec<&str> = attrs.keys().map(|s| s.as_str()).collect();
            if have != want {
                out.push(format!(
                    "node {}: attr list/order {have:?} != oracle {want:?}",
                    n.name
                ));
            }
            for at in &n.attrs {
                if let Some(wd) = attrs.get(at.name) {
                    let ok = match (&at.default, wd.get("default")) {
                        (AttrDefault::Undefined, Some(Value::String(s))) => s == "__undefined__",
                        (AttrDefault::Val(x), Some(y)) => x == y,
                        _ => false,
                    };
                    if !ok {
                        out.push(format!(
                            "node {} attr {}: default {:?} != oracle {}",
                            n.name, at.name, at.default, wd
                        ));
                    }
                }
            }
        }
        let my_marks: Vec<&str> = self.marks.iter().map(|m| m.name).collect();
        let their_marks: Vec<&str> = dm.keys().map(|s| s.as_str()).collect();
        if my_marks != their_marks {
            out.push(format!(
                "mark list/order differs: rust={my_marks:?} oracle={their_marks:?}"
            ));
        }
        for m in &self.marks {
            let Some(d) = dm.get(m.name) else { continue };
            let attrs = d["attrs"].as_object().cloned().unwrap_or_default();
            for at in &m.attrs {
                if let Some(wd) = attrs.get(at.name) {
                    let ok = matches!((&at.default, wd.get("default")), (AttrDefault::Val(x), Some(y)) if x == y);
                    if !ok {
                        out.push(format!(
                            "mark {} attr {}: default differs from oracle {}",
                            m.name, at.name, wd
                        ));
                    }
                }
            }
            let want_excl = d["excludes"].as_str();
            let have_excl = if m.excludes == Excludes::All {
                Some("_")
            } else {
                None
            };
            if want_excl != have_excl {
                out.push(format!("mark {}: excludes differs", m.name));
            }
        }
        out
    }
}
