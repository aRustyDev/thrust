//! A small ProseMirror document model: just enough of `prosemirror-model` (Node, Mark, Fragment normalisation,
//! `computeAttrs`, `createChecked`, `createAndFill`, `toJSON`/`fromJSON`, `Mark.addToSet`) for the conversions.

use crate::schema::{AttrDefault, Excludes, Schema, TypeId};
use serde_json::{Map, Value};
use std::collections::BTreeMap;

pub type Attrs = BTreeMap<String, Value>;

#[derive(Clone, Debug, PartialEq)]
pub struct Mark {
    pub ty: usize,
    pub attrs: Attrs,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    pub ty: TypeId,
    pub attrs: Attrs,
    pub content: Vec<Node>,
    pub marks: Vec<Mark>,
    pub text: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PmError(pub String);
impl std::fmt::Display for PmError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}
impl std::error::Error for PmError {}

/// Numbers compare by value (JS has a single number type): normalise integral floats to integers.
pub fn norm_num(v: Value) -> Value {
    match v {
        Value::Number(n) => {
            if n.is_i64() || n.is_u64() {
                Value::Number(n)
            } else if let Some(f) = n.as_f64() {
                if f.fract() == 0.0 && f.abs() < 9.007_199_254_740_992e15 {
                    Value::Number((f as i64).into())
                } else {
                    Value::Number(n)
                }
            } else {
                Value::Number(n)
            }
        }
        Value::Array(a) => Value::Array(a.into_iter().map(norm_num).collect()),
        Value::Object(o) => Value::Object(o.into_iter().map(|(k, v)| (k, norm_num(v))).collect()),
        o => o,
    }
}

/// ProseMirror `computeAttrs`: keep only spec'd attributes, `undefined`/missing fall back to the default.
pub fn compute_attrs(spec: &[crate::schema::AttrSpec], given: &Attrs) -> Attrs {
    let mut out = Attrs::new();
    for a in spec {
        match given.get(a.name) {
            Some(v) => {
                out.insert(a.name.to_string(), norm_num(v.clone()));
            }
            None => {
                if let AttrDefault::Val(d) = &a.default {
                    out.insert(a.name.to_string(), d.clone());
                } // Undefined default: key stays absent
            }
        }
    }
    out
}

impl Mark {
    pub fn create(schema: &Schema, ty: usize, given: &Attrs) -> Mark {
        Mark { ty, attrs: compute_attrs(&schema.marks[ty].attrs, given) }
    }

    /// `Mark.addToSet`: respects exclusion and keeps the set sorted by rank.
    pub fn add_to_set(&self, schema: &Schema, set: &[Mark]) -> Vec<Mark> {
        let mut copy: Option<Vec<Mark>> = None;
        let mut placed = false;
        for (i, other) in set.iter().enumerate() {
            if self == other {
                return set.to_vec();
            }
            if excludes(schema, self.ty, other.ty) {
                if copy.is_none() {
                    copy = Some(set[..i].to_vec());
                }
            } else if excludes(schema, other.ty, self.ty) {
                return set.to_vec();
            } else {
                if !placed && schema.marks[other.ty].rank > schema.marks[self.ty].rank {
                    let c = copy.get_or_insert_with(|| set[..i].to_vec());
                    c.push(self.clone());
                    placed = true;
                }
                if let Some(c) = copy.as_mut() {
                    c.push(other.clone());
                }
            }
        }
        let mut copy = copy.unwrap_or_else(|| set.to_vec());
        if !placed {
            copy.push(self.clone());
        }
        copy
    }
}

/// `MarkType.excludes(other)`.
pub fn excludes(schema: &Schema, this: usize, other: usize) -> bool {
    match schema.marks[this].excludes {
        Excludes::All => true,
        Excludes::Itself => this == other,
    }
}

/// `Mark.setFrom`: sort by rank.
pub fn mark_set_from(schema: &Schema, mut marks: Vec<Mark>) -> Vec<Mark> {
    marks.sort_by_key(|m| schema.marks[m.ty].rank);
    marks
}

impl Node {
    pub fn text(schema: &Schema, text: &str, marks: Vec<Mark>) -> Result<Node, PmError> {
        if text.is_empty() {
            return Err(PmError("Empty text nodes are not allowed".into()));
        }
        Ok(Node { ty: schema.text_type(), attrs: Attrs::new(), content: vec![], marks: mark_set_from(schema, marks), text: Some(text.to_string()) })
    }

    pub fn is_text(&self) -> bool {
        self.text.is_some()
    }

    /// `NodeType.create(attrs, content, marks)`: no content check, text nodes joined (`Fragment.from`).
    pub fn create(schema: &Schema, ty: TypeId, given: &Attrs, content: Vec<Node>) -> Node {
        Node { ty, attrs: compute_attrs(&schema.nodes[ty].attrs, given), content: join_text(content), marks: vec![], text: None }
    }

    /// `schema.node(...)` = `NodeType.createChecked`: also validates content (and marks on children).
    pub fn create_checked(schema: &Schema, ty: TypeId, given: &Attrs, content: Vec<Node>) -> Result<Node, PmError> {
        let n = Node::create(schema, ty, given, content);
        n.check_shallow(schema)?;
        Ok(n)
    }

    /// `NodeType.validContent` for this node's direct children.
    pub fn check_shallow(&self, schema: &Schema) -> Result<(), PmError> {
        let nt = &schema.nodes[self.ty];
        match nt.content_match {
            None => {
                if !self.content.is_empty() {
                    return Err(PmError(format!("Invalid content for node {}", nt.name)));
                }
            }
            Some(start) => {
                let types: Vec<TypeId> = self.content.iter().map(|c| c.ty).collect();
                match schema.match_types(start, &types) {
                    Some(end) if schema.valid_end(end) => {}
                    _ => return Err(PmError(format!("Invalid content for node {}", nt.name))),
                }
                for c in &self.content {
                    for m in &c.marks {
                        if !schema.allows_mark_type(self.ty, m.ty) {
                            return Err(PmError(format!("Invalid content for node {}", nt.name)));
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// Recursive `Node.check()`.
    pub fn check(&self, schema: &Schema) -> Result<(), PmError> {
        if self.is_text() {
            return Ok(());
        }
        self.check_shallow(schema)?;
        for c in &self.content {
            c.check(schema)?;
        }
        Ok(())
    }

    /// `NodeType.createAndFill`: default attrs, content filled to a valid state.
    pub fn create_and_fill(schema: &Schema, ty: TypeId) -> Option<Node> {
        let nt = &schema.nodes[ty];
        if nt.is_text {
            return None;
        }
        let mut content = vec![];
        if let Some(start) = nt.content_match {
            let fill = schema.fill_before(start, &[], true)?;
            let mut m = start;
            for t in fill {
                content.push(Node::create_and_fill(schema, t)?);
                m = schema.match_type(m, t)?;
            }
            let _ = m;
        }
        Some(Node::create(schema, ty, &Attrs::new(), content))
    }

    pub fn with_marks(mut self, marks: Vec<Mark>) -> Node {
        self.marks = marks;
        self
    }

    /// `Node.toJSON`.
    pub fn to_json(&self, schema: &Schema) -> Value {
        let nt = &schema.nodes[self.ty];
        let mut o = Map::new();
        o.insert("type".into(), Value::String(nt.name.into()));
        if !nt.attrs.is_empty() {
            o.insert("attrs".into(), Value::Object(self.attrs.iter().map(|(k, v)| (k.clone(), v.clone())).collect()));
        }
        if !self.content.is_empty() {
            o.insert("content".into(), Value::Array(self.content.iter().map(|c| c.to_json(schema)).collect()));
        }
        if !self.marks.is_empty() {
            o.insert("marks".into(), Value::Array(self.marks.iter().map(|m| m.to_json(schema)).collect()));
        }
        if let Some(t) = &self.text {
            o.insert("text".into(), Value::String(t.clone()));
        }
        Value::Object(o)
    }

    /// `Node.fromJSON` (no content validation, like ProseMirror).
    pub fn from_json(schema: &Schema, v: &Value) -> Result<Node, PmError> {
        let o = v.as_object().ok_or_else(|| PmError("Invalid input for Node.fromJSON".into()))?;
        let marks = match o.get("marks") {
            Some(Value::Array(ms)) => {
                let mut out = vec![];
                for m in ms {
                    out.push(Mark::from_json(schema, m)?);
                }
                out
            }
            Some(Value::Null) | None => vec![],
            Some(_) => return Err(PmError("Invalid mark data".into())),
        };
        let tname = o.get("type").and_then(|t| t.as_str()).ok_or_else(|| PmError("missing type".into()))?;
        if tname == "text" {
            let t = o.get("text").and_then(|t| t.as_str()).ok_or_else(|| PmError("Invalid text node in JSON".into()))?;
            return Node::text(schema, t, marks);
        }
        let ty = schema.node_type(tname).ok_or_else(|| PmError(format!("There is no node type {tname} in this schema")))?;
        let content = match o.get("content") {
            Some(Value::Array(cs)) => {
                let mut out = vec![];
                for c in cs {
                    out.push(Node::from_json(schema, c)?);
                }
                out
            }
            _ => vec![],
        };
        let given: Attrs = match o.get("attrs") {
            Some(Value::Object(a)) => a.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
            _ => Attrs::new(),
        };
        let mut n = Node::create(schema, ty, &given, content);
        n.marks = mark_set_from(schema, marks);
        Ok(n)
    }

    pub fn text_content(&self) -> String {
        let mut s = String::new();
        self.collect_text(&mut s);
        s
    }
    fn collect_text(&self, out: &mut String) {
        if let Some(t) = &self.text {
            out.push_str(t);
        }
        for c in &self.content {
            c.collect_text(out);
        }
    }
}

impl Mark {
    pub fn to_json(&self, schema: &Schema) -> Value {
        let mt = &schema.marks[self.ty];
        let mut o = Map::new();
        o.insert("type".into(), Value::String(mt.name.into()));
        if !mt.attrs.is_empty() {
            o.insert("attrs".into(), Value::Object(self.attrs.iter().map(|(k, v)| (k.clone(), v.clone())).collect()));
        }
        Value::Object(o)
    }
    pub fn from_json(schema: &Schema, v: &Value) -> Result<Mark, PmError> {
        let o = v.as_object().ok_or_else(|| PmError("Invalid input for Mark.fromJSON".into()))?;
        let name = o.get("type").and_then(|t| t.as_str()).ok_or_else(|| PmError("missing mark type".into()))?;
        let ty = schema.mark_type(name).ok_or_else(|| PmError(format!("There is no mark type {name} in this schema")))?;
        let given: Attrs = match o.get("attrs") {
            Some(Value::Object(a)) => a.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
            _ => Attrs::new(),
        };
        Ok(Mark::create(schema, ty, &given))
    }
}

/// `Fragment.fromArray`: join adjacent text nodes that have identical marks.
pub fn join_text(nodes: Vec<Node>) -> Vec<Node> {
    let mut out: Vec<Node> = Vec::with_capacity(nodes.len());
    for n in nodes {
        if let (Some(last), true) = (out.last_mut(), n.is_text()) {
            if last.is_text() && last.marks == n.marks {
                last.text.as_mut().unwrap().push_str(n.text.as_deref().unwrap());
                continue;
            }
        }
        out.push(n);
    }
    out
}

/// Document root: `type: doc` node with the given blocks (no check).
pub fn doc(schema: &Schema, content: Vec<Node>) -> Node {
    Node::create(schema, schema.top, &Attrs::new(), content)
}
