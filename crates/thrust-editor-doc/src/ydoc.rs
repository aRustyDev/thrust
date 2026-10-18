//! The y-prosemirror XmlFragment encoding, read and written with `yrs`.
//!
//! Write side mirrors `prosemirrorToYXmlFragment` / `createTypeFromElementNode` / `createTypeFromTextNodes`:
//! each node is a `Y.XmlElement` named after its node type, non-null attributes are stored as-is (any JSON value),
//! consecutive text nodes of a block become ONE `Y.XmlText` whose delta attributes are `{ markName: markAttrs }`.
//!
//! Read side mirrors `createNodeFromYElement` / `createTextNodesFromYText` / `yXmlFragmentToProseMirrorRootNode`:
//! an element that cannot be built (unknown type, invalid content) or a text with an unknown mark is skipped
//! (y-prosemirror deletes it in the in-memory Yjs doc; here we only skip it).

use crate::pm::{join_text, norm_num, Attrs, Mark, Node, PmError};
use crate::schema::Schema;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use yrs::types::text::YChange;
use yrs::types::xml::{XmlFragment, XmlOut};
use yrs::types::{Attrs as YAttrs, Delta};
use yrs::updates::decoder::Decode;
use yrs::{Any, Doc, In, OffsetKind, Options, Out, ReadTxn, Text, Transact, Update, Xml, XmlElementPrelim, XmlFragmentRef, XmlTextRef};

fn new_doc(client_id: Option<u64>) -> Doc {
    let mut o = Options::default();
    if let Some(c) = client_id {
        o.client_id = yrs::ClientID::new(c & ((1u64 << 53) - 1));
    }
    // Yjs counts string offsets in UTF-16 code units; yrs defaults to bytes.
    o.offset_kind = OffsetKind::Utf16;
    Doc::with_options(o)
}

pub fn value_to_any(v: &Value) -> Any {
    match v {
        Value::Null => Any::Null,
        Value::Bool(b) => Any::Bool(*b),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Any::Number(yrs::any::Number::Int(i))
            } else {
                Any::Number(yrs::any::Number::Float(n.as_f64().unwrap_or(0.0)))
            }
        }
        Value::String(s) => Any::String(Arc::from(s.as_str())),
        Value::Array(a) => Any::Array(a.iter().map(value_to_any).collect::<Vec<_>>().into()),
        Value::Object(o) => {
            let m: HashMap<String, Any> = o.iter().map(|(k, v)| (k.clone(), value_to_any(v))).collect();
            Any::Map(Arc::new(m))
        }
    }
}

/// `None` for `undefined` (the attribute is treated as absent).
pub fn any_to_value(a: &Any) -> Option<Value> {
    Some(match a {
        Any::Undefined => return None,
        Any::Null => Value::Null,
        Any::Bool(b) => Value::Bool(*b),
        Any::Number(yrs::any::Number::Int(i)) => Value::Number((*i).into()),
        Any::Number(yrs::any::Number::Float(f)) => norm_num(serde_json::Number::from_f64(*f).map(Value::Number).unwrap_or(Value::Null)),
        Any::String(s) => Value::String(s.to_string()),
        Any::Buffer(_) => Value::Null,
        Any::Array(a) => Value::Array(a.iter().map(|x| any_to_value(x).unwrap_or(Value::Null)).collect()),
        Any::Map(m) => Value::Object(m.iter().filter_map(|(k, v)| any_to_value(v).map(|x| (k.clone(), x))).collect()),
    })
}

// ---------------------------------------------------------------- write

fn text_delta(schema: &Schema, run: &[&Node]) -> Vec<Delta<In>> {
    run.iter()
        .map(|n| {
            let mut attrs: YAttrs = YAttrs::new();
            for m in &n.marks {
                // `isOverlapping` would add a `--hash` suffix; no mark in this schema is overlapping
                // (every mark excludes itself or everything), so the plain name is used.
                let obj: HashMap<String, Any> = m.attrs.iter().map(|(k, v)| (k.clone(), value_to_any(v))).collect();
                attrs.insert(Arc::from(schema.marks[m.ty].name), Any::Map(Arc::new(obj)));
            }
            Delta::Inserted(In::Any(Any::String(Arc::from(n.text.as_deref().unwrap_or("")))), Some(Box::new(attrs)))
        })
        .collect()
}

fn write_children(schema: &Schema, txn: &mut yrs::TransactionMut, parent: &impl XmlFragment, children: &[Node]) {
    let mut i = 0;
    while i < children.len() {
        let c = &children[i];
        if c.is_text() {
            let mut j = i;
            while j < children.len() && children[j].is_text() {
                j += 1;
            }
            let run: Vec<&Node> = children[i..j].iter().collect();
            let prelim = yrs::types::xml::XmlDeltaPrelim { attributes: HashMap::new(), delta: text_delta(schema, &run) };
            parent.push_back(txn, yrs::types::xml::XmlIn::Text(prelim));
            i = j;
        } else {
            let name = schema.node_name(c.ty);
            let el = parent.push_back(txn, XmlElementPrelim::empty(name));
            write_children(schema, txn, &el, &c.content);
            // attributes after children, like Y.XmlElement._integrate
            for spec in &schema.nodes[c.ty].attrs {
                match c.attrs.get(spec.name) {
                    Some(Value::Null) => {} // y-prosemirror skips null attributes
                    Some(v) => {
                        el.insert_attribute(txn, spec.name, value_to_any(v));
                    }
                    // JS `undefined` is written as an explicit undefined (`val !== null`)
                    None => {
                        el.insert_attribute(txn, spec.name, Any::Undefined);
                    }
                }
            }
            i += 1;
        }
    }
}

/// Encode `root` (a `doc` node) as the `default` XmlFragment and, optionally, `title` as the `title` fragment
/// (a second client id), producing one Yjs update v1.
pub fn encode(schema: &Schema, root: &Node, title: Option<&Node>, client_id: Option<u64>) -> Vec<u8> {
    let doc = new_doc(client_id);
    {
        let frag = doc.get_or_insert_xml_fragment("default");
        let mut txn = doc.transact_mut();
        write_children(schema, &mut txn, &frag, &root.content);
    }
    if let Some(t) = title {
        let tdoc = new_doc(client_id.map(|c| c ^ 0x5bd1_e995));
        {
            let frag = tdoc.get_or_insert_xml_fragment("title");
            let mut txn = tdoc.transact_mut();
            write_children(schema, &mut txn, &frag, &t.content);
        }
        let upd = tdoc.transact().encode_state_as_update_v1(&yrs::StateVector::default());
        let mut txn = doc.transact_mut();
        if let Ok(u) = Update::decode_v1(&upd) {
            let _ = txn.apply_update(u);
        }
    }
    let txn = doc.transact();
    txn.encode_state_as_update_v1(&yrs::StateVector::default())
}

// ---------------------------------------------------------------- read

fn element_attrs(txn: &impl ReadTxn, el: &yrs::XmlElementRef) -> Attrs {
    let mut a = Attrs::new();
    for (k, v) in el.attributes(txn) {
        if let Out::Any(any) = v {
            if let Some(val) = any_to_value(&any) {
                a.insert(k.to_string(), val);
            }
        }
    }
    a
}

fn text_nodes(schema: &Schema, txn: &impl ReadTxn, t: &XmlTextRef) -> Option<Vec<Node>> {
    let mut out = vec![];
    for d in t.diff(txn, YChange::identity) {
        let Out::Any(Any::String(s)) = d.insert else { continue };
        let mut marks = vec![];
        if let Some(attrs) = d.attributes {
            // `attributesToMarks`: iteration order of the delta attributes, then sorted by rank in `schema.text`
            for (name, val) in attrs.iter() {
                let base = strip_hash(name);
                let Some(ty) = schema.mark_type(base) else { return None };
                let given: Attrs = match any_to_value(val) {
                    Some(Value::Object(o)) => o.into_iter().collect(),
                    _ => Attrs::new(),
                };
                marks.push(Mark::create(schema, ty, &given));
            }
        }
        out.push(Node::text(schema, &s, marks).ok()?);
    }
    Some(out)
}

/// `yattr2markname`: strip a trailing `--XXXXXXXX` hash.
fn strip_hash(name: &str) -> &str {
    let b = name.as_bytes();
    if b.len() > 10 && &b[b.len() - 10..b.len() - 8] == b"--" {
        let tail = &name[name.len() - 8..];
        if tail.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'+' || c == b'/' || c == b'=') {
            return &name[..name.len() - 10];
        }
    }
    name
}

fn node_from_element(schema: &Schema, txn: &impl ReadTxn, el: &yrs::XmlElementRef) -> Option<Node> {
    let mut children = vec![];
    for c in el.children(txn) {
        match c {
            XmlOut::Element(e) => {
                if let Some(n) = node_from_element(schema, txn, &e) {
                    children.push(n);
                }
            }
            XmlOut::Text(t) => {
                if let Some(ns) = text_nodes(schema, txn, &t) {
                    children.extend(ns);
                }
            }
            XmlOut::Fragment(_) => {}
        }
    }
    let ty = schema.node_type(el.tag())?;
    let attrs = element_attrs(txn, el);
    Node::create_checked(schema, ty, &attrs, children).ok()
}

/// `yXmlFragmentToProseMirrorRootNode(fragment, schema)`.
pub fn fragment_to_doc(schema: &Schema, txn: &impl ReadTxn, frag: &XmlFragmentRef) -> Node {
    let mut children = vec![];
    for c in frag.children(txn) {
        match c {
            XmlOut::Element(e) => {
                if let Some(n) = node_from_element(schema, txn, &e) {
                    children.push(n);
                }
            }
            // y-prosemirror's yXmlFragmentToProseMirrorFragment assumes elements; text at the root is not produced
            _ => {}
        }
    }
    Node::create(schema, schema.top, &Attrs::new(), join_text(children))
}

pub fn decode(schema: &Schema, update: &[u8], fragment: &str) -> Result<Node, PmError> {
    let doc = new_doc(None);
    let u = Update::decode_v1(update).map_err(|e| PmError(format!("invalid Yjs update: {e}")))?;
    {
        let mut txn = doc.transact_mut();
        txn.apply_update(u).map_err(|e| PmError(format!("cannot apply Yjs update: {e}")))?;
    }
    let frag = doc.get_or_insert_xml_fragment(fragment);
    let txn = doc.transact();
    Ok(fragment_to_doc(schema, &txn, &frag))
}

/// Raw structure dump (same shape as `ystruct()` in the oracle) for structural comparison.
pub fn struct_dump(update: &[u8], fragment: &str) -> Result<Value, PmError> {
    let doc = new_doc(None);
    let u = Update::decode_v1(update).map_err(|e| PmError(format!("invalid Yjs update: {e}")))?;
    {
        let mut txn = doc.transact_mut();
        txn.apply_update(u).map_err(|e| PmError(format!("cannot apply Yjs update: {e}")))?;
    }
    let frag = doc.get_or_insert_xml_fragment(fragment);
    let txn = doc.transact();
    Ok(dump_children(&txn, &frag, true))
}

fn dump_children(txn: &impl ReadTxn, node: &impl XmlFragment, is_fragment: bool) -> Value {
    let mut kids = vec![];
    for c in node.children(txn) {
        match c {
            XmlOut::Element(e) => {
                let mut attrs = serde_json::Map::new();
                let mut keys: Vec<(String, Out)> = e.attributes(txn).map(|(k, v)| (k.to_string(), v)).collect();
                keys.sort_by(|a, b| a.0.cmp(&b.0));
                for (k, v) in keys {
                    if let Out::Any(a) = v {
                        attrs.insert(k, any_to_value(&a).unwrap_or(Value::String("__undefined__".into())));
                    }
                }
                let mut o = serde_json::Map::new();
                o.insert("el".into(), Value::String(e.tag().to_string()));
                o.insert("attrs".into(), Value::Object(attrs));
                o.insert("children".into(), dump_children(txn, &e, false)["children"].clone());
                kids.push(Value::Object(o));
            }
            XmlOut::Text(t) => {
                let mut delta = vec![];
                for d in t.diff(txn, YChange::identity) {
                    let mut o = serde_json::Map::new();
                    if let Out::Any(Any::String(s)) = &d.insert {
                        o.insert("insert".into(), Value::String(s.to_string()));
                    }
                    if let Some(attrs) = d.attributes {
                        let m: serde_json::Map<String, Value> = attrs.iter().map(|(k, v)| (k.to_string(), any_to_value(v).unwrap_or(Value::Null))).collect();
                        if !m.is_empty() {
                            o.insert("attributes".into(), Value::Object(m));
                        }
                    }
                    delta.push(Value::Object(o));
                }
                let mut o = serde_json::Map::new();
                o.insert("text".into(), Value::Array(delta));
                kids.push(Value::Object(o));
            }
            XmlOut::Fragment(_) => {}
        }
    }
    let mut o = serde_json::Map::new();
    if is_fragment {
        o.insert("fragment".into(), Value::Bool(true));
    }
    o.insert("children".into(), Value::Array(kids));
    Value::Object(o)
}
