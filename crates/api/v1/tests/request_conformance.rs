//! The request schemas of the document against the parsers that read the requests (issue #58, step 0 of the request-parser series).
//!
//! The document describes a request body in eight hand-written types (`openapi.rs`); the routes read the body with hand parsers
//! (`fields.rs`). The two are written apart, and the only things that compare them are `oasdiff` (the document against the PIN, not
//! against the parser) and a test that a change takes the keys of a creation. A bound can change in the parser and the document go on
//! saying the old one.
//!
//! This test closes that gap from the document's side. It reads each request schema, GENERATES the bodies the schema's own keywords
//! speak of, and sends each to the real router, in a world of its own:
//!
//! * a value the schema allows is accepted: each enum value, a string at `minLength` and at `maxLength` (in four alphabets, because a
//!   length is a count of characters and not of bytes), an integer at `minimum` and at `maximum`, a `null` where the type has one;
//! * a value it does not is refused UNDER THAT KEY AND NO OTHER: one past either bound, a value that is not in the enum (and the
//!   case and spacing variants of one that is), a `null` where the type has none, a key the creation requires, left out, a string that
//!   is no uuid or no date, and a type DRF does not coerce.
//!
//! What it does not say: that the route accepts a value the schema leaves out of its enum (the document's lists are the pin's, and
//! `scripts/check-openapi.sh` compares them with it), and anything the schema cannot express (a row that must exist, `validate()`,
//! the order of the checks). Where a schema key cannot be given a value the world can take (a key that names a row the world does not
//! have), [`UNSAMPLED`] says so and why, and a key that is neither sampled nor listed there fails the test.
//!
//! The document is the input of every probe, so a probe set cannot be written by hand to agree with the parser: the controls at the
//! end break the document in memory, one keyword at a time, and each break must be reported.
#![cfg(feature = "openapi")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "test code: a panic is how a test fails, and clippy.toml exempts only #[test] functions and #[cfg(test)] modules, not the helpers around them"
)]

mod common;

use std::collections::BTreeSet;

use common::*;
use rstest::rstest;
use serde_json::{json, Map, Value};
use thrust_contracts::Uuid;

/// The four alphabets a string at a bound is made of: one byte, two, three, and four (a code point of its own in UTF-8, and two
/// units in UTF-16, which is what JavaScript counts).
const ALPHABETS: [&str; 4] = ["a", "é", "日", "😀"];

/// What the world has for a key that names a row: the ids a probe may use, which every world of an operation has alike.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Fixtures {
    /// A person of the workspace other than the caller.
    member: String,
    state: String,
    label: String,
    /// A second label, and a second work item, to be a parent (nothing may be its own parent).
    other_label: String,
    other_item: Option<String>,
}

/// An operation that takes a body: its method, path, and the component that describes the body.
#[derive(Debug, Clone)]
struct Operation {
    id: String,
    method: &'static str,
    path: String,
    schema: String,
}

impl Operation {
    fn is_creation(&self) -> bool {
        self.method == "POST"
    }
}

fn document() -> Value {
    serde_json::to_value(thrust_api_v1::openapi::openapi()).expect("the document is JSON")
}

/// Every operation of the document that takes a JSON body, with the schema it names.
fn operations_with_a_body(doc: &Value) -> Vec<Operation> {
    let mut out = Vec::new();
    for (path, item) in doc["paths"].as_object().expect("paths") {
        for method in ["post", "patch", "put"] {
            let Some(op) = item.get(method) else { continue };
            let Some(body) = op.get("requestBody") else { continue };
            let reference = body["content"]["application/json"]["schema"]["$ref"]
                .as_str()
                .unwrap_or_else(|| panic!("{method} {path}: a request body that is not a $ref: {body}"));
            out.push(Operation {
                id: op["operationId"].as_str().expect("an operation id").to_string(),
                method: match method {
                    "post" => "POST",
                    "patch" => "PATCH",
                    _ => panic!("{method} {path}: this test has not met a PUT body"),
                },
                path: path.clone(),
                schema: reference.rsplit('/').next().expect("a name").to_string(),
            });
        }
    }
    out
}

fn id_of(n: u128) -> String {
    Uuid::from_u128(n).to_string()
}

/// A world of its own and the URI the operation is sent to in it. A change is sent to a row that is there (a work item is made first,
/// and another beside it to be a parent), a creation to the collection.
fn target(op: &Operation) -> (World, String, Fixtures) {
    let w = real_world();
    let base = format!("/api/v1/workspaces/{SLUG}/projects/{}", id_of(PROJECT));
    let family = if op.path.contains("/work-items/") { "work-items" } else { "issues" };
    let is_item = op.path.contains("/issues/") || op.path.contains("/work-items/");
    let mut items = Vec::new();
    if is_item {
        for name in ["seed-a", "seed-b"] {
            let made =
                send_json(&w.router, "key-admin", "POST", &format!("{base}/{family}/"), &json!({ "name": name }));
            assert_eq!(made.status, 201, "{:?}", made.body);
            items.push(made.body["id"].as_str().expect("an id").to_string());
        }
    }
    let mut uri = op
        .path
        .replace("{slug}", SLUG)
        .replace("{project_id}", &id_of(PROJECT))
        .replace("{state_id}", &id_of(STATE_TODO));
    if uri.contains("{pk}") {
        let pk = if uri.contains("/projects/{pk}/") {
            id_of(PROJECT)
        } else if uri.contains("/labels/{pk}/") {
            id_of(LABEL_BUG)
        } else if is_item {
            items[0].clone()
        } else {
            panic!("{}: a {{pk}} this test cannot fill", op.path)
        };
        uri = uri.replace("{pk}", &pk);
    }
    assert!(!uri.contains('{'), "a placeholder this test cannot fill: {}", op.path);
    let fixtures = Fixtures {
        member: Who::Member.id().0.to_string(),
        state: id_of(STATE_TODO),
        label: id_of(LABEL_BUG),
        other_label: id_of(LABEL_FEATURE),
        other_item: items.get(1).cloned(),
    };
    (w, uri, fixtures)
}

/// The keys of a schema that the world has no row for: no estimate, no estimate point, no work item type. The schema lets a uuid be
/// there and the parser answers "no such object", which is not a disagreement but is not an acceptance either, so a probe cannot show
/// an accepted value. A key that is not here and has no sample is a defect of this test.
const UNSAMPLED: &[(&str, &str)] = &[
    ("PatchedProjectUpdateRequest", "estimate"),
    ("IssueRequest", "estimate_point"),
    ("IssueRequest", "type"),
    ("IssueRequest", "type_id"),
    ("PatchedIssueRequest", "estimate_point"),
    ("PatchedIssueRequest", "type"),
    ("PatchedIssueRequest", "type_id"),
];

/// A property's schema with its `$ref` followed and a single `allOf` folded in: the document writes `priority` the pin's way, as an
/// `allOf` of the enum, so a keyword the property itself carries (`title`) is laid over the enum's.
fn resolved(doc: &Value, schema: &Value) -> Value {
    if let Some(reference) = schema["$ref"].as_str() {
        return resolved(doc, &doc["components"]["schemas"][reference.rsplit('/').next().expect("a name")]);
    }
    if let Some([only]) = schema["allOf"].as_array().map(Vec::as_slice) {
        let mut out = resolved(doc, only);
        for (k, v) in schema.as_object().expect("an object") {
            if k != "allOf" {
                out[k] = v.clone();
            }
        }
        return out;
    }
    schema.clone()
}

/// A property, as the probes need it.
struct Prop {
    /// `string`, `integer`, `number`, `boolean`, `array`, or `None` for a schema that names no type (any value).
    kind: Option<String>,
    nullable: bool,
    format: Option<String>,
    min_length: Option<u64>,
    max_length: Option<u64>,
    minimum: Option<i64>,
    maximum: Option<i64>,
    choices: Option<Vec<String>>,
}

fn prop_of(doc: &Value, schema: &Value) -> Prop {
    let s = resolved(doc, schema);
    let (kind, nullable) = match &s["type"] {
        Value::String(t) => (Some(t.clone()), t == "null"),
        Value::Array(types) => (
            types.iter().filter_map(Value::as_str).find(|t| *t != "null").map(String::from),
            types.iter().any(|t| t == "null"),
        ),
        _ => (None, true),
    };
    Prop {
        kind,
        nullable,
        format: s["format"].as_str().map(String::from),
        min_length: s["minLength"].as_u64(),
        max_length: s["maxLength"].as_u64(),
        minimum: s["minimum"].as_i64(),
        maximum: s["maximum"].as_i64(),
        choices: s["enum"]
            .as_array()
            .map(|a| a.iter().map(|v| v.as_str().expect("a string choice").to_string()).collect()),
    }
}

/// What a key that names a row is given where the world has one.
fn row_for(schema: &str, key: &str, fx: &Fixtures) -> Option<Value> {
    Some(match key {
        "project_lead" | "default_assignee" | "created_by" | "assignees" => json!(fx.member),
        "default_state" | "state" => json!(fx.state),
        "labels" => json!(fx.label),
        "parent" if schema.contains("Label") => json!(fx.other_label),
        "parent" => json!(fx.other_item.as_ref()?),
        _ => return None,
    })
}

/// A probe: one body, the key it is about, and whether the route must accept it or refuse it under that key alone.
#[derive(Debug, Clone)]
struct Probe {
    /// The key the probe changes; empty for the base body as it is.
    key: String,
    what: String,
    /// The value the key is given; `None` leaves it out.
    value: Option<Value>,
    accepted: bool,
}

fn accepted(key: &str, what: impl Into<String>, value: Value) -> Probe {
    Probe { key: key.into(), what: what.into(), value: Some(value), accepted: true }
}

fn refused(key: &str, what: impl Into<String>, value: Value) -> Probe {
    Probe { key: key.into(), what: what.into(), value: Some(value), accepted: false }
}

/// A string of `len` characters, each the alphabet's one.
fn of_len(unit: &str, len: u64) -> String {
    unit.repeat(usize::try_from(len).expect("a length"))
}

/// The values a key of this schema must accept, from its keywords alone; `rows` is what the world has for a key that names a row.
fn allowed(p: &Prop, rows: Option<&Value>, key: &str) -> Vec<(String, Value)> {
    let mut out: Vec<(String, Value)> = Vec::new();
    if p.nullable {
        out.push(("null".into(), Value::Null));
    }
    if let Some(choices) = &p.choices {
        out.extend(choices.iter().map(|c| (format!("the choice {c:?}"), json!(c))));
        return out;
    }
    match p.kind.as_deref() {
        Some("string") => match p.format.as_deref() {
            Some("uuid") => out.extend(rows.map(|r| ("a row that is there".to_string(), r.clone()))),
            Some("date") => out.push(("a date".into(), json!("2026-01-01"))),
            Some("date-time") => out.push(("a date and time".into(), json!("2026-01-01T00:00:00Z"))),
            _ => {
                let low = p.min_length.unwrap_or(0);
                let mut lengths = vec![low];
                lengths.extend(p.max_length.filter(|m| *m != low));
                if p.max_length.is_none() {
                    lengths.push(low.max(1) + 2);
                }
                for unit in ALPHABETS {
                    for len in &lengths {
                        let value = json!(of_len(unit, *len));
                        // the empty string is the same in every alphabet
                        if !out.iter().any(|(_, v)| *v == value) {
                            out.push((format!("{len} of {unit:?}"), value));
                        }
                    }
                }
            }
        },
        Some("integer") => {
            let values: BTreeSet<i64> = match (p.minimum, p.maximum) {
                (Some(lo), Some(hi)) => [lo, hi, lo + (hi - lo) / 2].into(),
                (Some(lo), None) => [lo, lo + 1].into(),
                (None, Some(hi)) => [hi, hi - 1].into(),
                (None, None) => [0, 1].into(),
            };
            out.extend(values.into_iter().map(|v| (format!("the integer {v}"), json!(v))));
        }
        Some("number") => out.extend([0.0, 1.5, -2.25].map(|v| (format!("the number {v}"), json!(v)))),
        Some("boolean") => out.extend([true, false].map(|v| (format!("{v}"), json!(v)))),
        Some("array") => {
            out.push(("an empty list".into(), json!([])));
            if let Some(r) = rows {
                out.push(("a list of a row that is there".into(), json!([r])));
            }
        }
        None => out.extend([
            ("a number".to_string(), json!(1)),
            ("a string".to_string(), json!("x")),
            ("a boolean".to_string(), json!(true)),
            ("an object".to_string(), json!({ "a": [1, { "b": null }] })),
            ("a list".to_string(), json!([1, "x"])),
        ]),
        Some(other) => panic!("{key}: a type this test has not met: {other}"),
    }
    out
}

/// The values a key must refuse, from the same keywords: one past each bound, outside the enum, a null the type does not have, a
/// string that is no uuid or no date, and the types DRF does not coerce into this one.
fn forbidden(p: &Prop) -> Vec<(String, Value)> {
    let mut out: Vec<(String, Value)> = Vec::new();
    if !p.nullable {
        out.push(("null".into(), Value::Null));
    }
    if let Some(choices) = &p.choices {
        // the variants of a choice, which an exact-match choice field refuses: of the first, the middle and the last choice (all of
        // them when there are few)
        let at: BTreeSet<usize> = if choices.len() <= 8 {
            (0..choices.len()).collect()
        } else {
            [0, choices.len() / 2, choices.len() - 1].into()
        };
        for i in at {
            let c = &choices[i];
            let variants = [
                (c.to_uppercase(), "upper-cased"),
                (c.to_lowercase(), "lower-cased"),
                (format!(" {c}"), "with a space before"),
                (format!("{c} "), "with a space after"),
                (format!("{c}x"), "with a letter after"),
            ];
            for (v, how) in variants {
                if !choices.contains(&v) {
                    out.push((format!("the choice {c:?} {how}"), json!(v)));
                }
            }
        }
        out.push(("a string that is no choice".into(), json!("no such choice")));
        out.push(("the empty string".into(), json!("")));
    }
    match p.kind.as_deref() {
        Some("string") if p.choices.is_none() => {
            match p.format.as_deref() {
                Some("uuid") => out.push(("a string that is no uuid".into(), json!("not-a-uuid"))),
                Some("date") => {
                    out.push(("a string that is no date".into(), json!("not-a-date")));
                    out.push(("a day that is not in the month".into(), json!("2026-02-30")));
                }
                Some("date-time") => out.push(("a string that is no date and time".into(), json!("not-a-date-time"))),
                _ => {}
            }
            if let Some(max) = p.max_length {
                for unit in ALPHABETS {
                    out.push((format!("{} of {unit:?}: one past the maximum", max + 1), json!(of_len(unit, max + 1))));
                }
            }
            if let Some(min) = p.min_length.filter(|m| *m >= 1) {
                out.push((format!("{} characters: one short of the minimum", min - 1), json!(of_len("a", min - 1))));
            }
            out.extend([
                ("an object".into(), json!({})),
                ("a list".into(), json!([])),
                ("a boolean".into(), json!(true)),
            ]);
        }
        Some("integer") => {
            if let Some(hi) = p.maximum {
                out.push((format!("{}: one past the maximum", hi + 1), json!(hi + 1)));
            }
            if let Some(lo) = p.minimum {
                out.push((format!("{}: one short of the minimum", lo - 1), json!(lo - 1)));
            }
            out.extend([
                ("a fraction".into(), json!(1.5)),
                ("a word".into(), json!("x")),
                ("an object".into(), json!({})),
                ("a list".into(), json!([])),
            ]);
        }
        Some("number") => {
            out.extend([("a word".into(), json!("x")), ("an object".into(), json!({})), ("a list".into(), json!([]))]);
        }
        Some("boolean") => {
            out.extend([("a word".into(), json!("x")), ("an object".into(), json!({})), ("a list".into(), json!([]))]);
        }
        Some("array") => {
            out.extend([
                ("an object".into(), json!({})),
                ("a string".into(), json!("x")),
                ("a number".into(), json!(1)),
                ("a list with something that is no uuid".into(), json!(["not-a-uuid"])),
                ("a list with a null".into(), json!([null])),
            ]);
        }
        _ => {}
    }
    out
}

/// The body every probe starts from: for a creation, each required key with the first value its schema allows; for a change, nothing.
fn base_body(doc: &Value, op: &Operation, fx: &Fixtures) -> Map<String, Value> {
    let schema = &doc["components"]["schemas"][&op.schema];
    let mut body = Map::new();
    for key in schema["required"].as_array().into_iter().flatten().filter_map(Value::as_str) {
        let p = prop_of(doc, &schema["properties"][key]);
        let rows = row_for(&op.schema, key, fx);
        let first = allowed(&p, rows.as_ref(), key)
            .into_iter()
            .find(|(what, _)| what != "null")
            .unwrap_or_else(|| panic!("{}: the required key {key} has no value to start from", op.schema));
        body.insert(key.to_string(), first.1);
    }
    body
}

/// Every probe of one operation's schema (`only`: of one key). The second value is the keys that have no sample.
fn probes(doc: &Value, op: &Operation, fx: &Fixtures, only: Option<&str>) -> (Vec<Probe>, Vec<String>) {
    let schema = &doc["components"]["schemas"][&op.schema];
    let required: BTreeSet<&str> =
        schema["required"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
    let mut out = Vec::new();
    let mut unsampled = Vec::new();
    if only.is_none() {
        let what = if op.is_creation() { "only the required keys" } else { "no keys at all" };
        out.push(Probe { key: String::new(), what: what.into(), value: None, accepted: true });
    }
    for (key, property) in schema["properties"].as_object().expect("properties") {
        if only.is_some_and(|k| k != key) {
            continue;
        }
        let p = prop_of(doc, property);
        let rows = row_for(&op.schema, key, fx);
        let mut here: Vec<Probe> = Vec::new();
        for (what, v) in allowed(&p, rows.as_ref(), key) {
            here.push(accepted(key, what, v));
        }
        for (what, v) in forbidden(&p) {
            here.push(refused(key, what, v));
        }
        if required.contains(key.as_str()) {
            here.push(Probe {
                key: key.clone(),
                what: "left out, and the schema requires it".into(),
                value: None,
                accepted: false,
            });
        } else if op.is_creation() {
            here.push(Probe {
                key: key.clone(),
                what: "left out, and the schema does not require it".into(),
                value: None,
                accepted: true,
            });
        }
        // a uuid key that names a row the world lacks: only `null` and what the schema forbids can be shown
        let names_a_row = p.format.as_deref() == Some("uuid") || (p.kind.as_deref() == Some("array") && rows.is_none());
        if names_a_row && rows.is_none() {
            unsampled.push(key.clone());
        }
        out.extend(here);
    }
    (out, unsampled)
}

/// A probe that the route answered other than the schema says.
#[derive(Debug)]
struct Disagreement {
    schema: String,
    key: String,
    /// The value sent, as JSON text; `None` for a key left out.
    value: Option<String>,
    /// What the schema says of it: allowed (so the route must accept it) or forbidden.
    allowed: bool,
    status: u16,
    answer: String,
    shown: String,
}

impl std::fmt::Display for Disagreement {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let says = if self.allowed { "allows it; the route refused" } else { "forbids it; the route took" };
        let answer: String = self.answer.chars().take(200).collect();
        write!(f, "{}\n    the schema {says}: {} {answer}", self.shown, self.status)
    }
}

/// What the schema allows and the route refuses on purpose. Each is a refusal that the schema cannot say (it is `validate()`, or the
/// pin raising), and each is pinned where it is decided. An entry names the schema, the key, the value sent as JSON text, and a piece
/// of the answer it must still give: a probe that disagrees any other way is reported, and an entry that no probe met is STALE (it
/// would go on hiding a defect that came back under another answer).
struct Known {
    schema: &'static str,
    key: &'static str,
    value: &'static str,
    answer_has: &'static str,
    reason: &'static str,
}

const STATE_TRIAGE: &str = "`StateSerializer.validate` refuses the group `triage` (it is the intake's), after every field was fine: \
                            tests/states.rs `a_triage_state_is_not_made_by_v1_and_only_when_every_field_was_fine` and `a_change_cannot_make_a_state_a_triage_state`";
const EMPTY_HTML: &str = "the pin's `html.fromstring(\"\")` has no document to read, so `IssueSerializer.validate` answers `Invalid HTML passed` to an \
                          empty description: tests/work_items.rs, the `description_html: \"\"` cases of the refusals";

const KNOWN: &[Known] = &[
    Known { schema: "StateRequest", key: "group", value: "\"triage\"", answer_has: "Cannot create triage state", reason: STATE_TRIAGE },
    Known { schema: "PatchedStateRequest", key: "group", value: "\"triage\"", answer_has: "Cannot create triage state", reason: STATE_TRIAGE },
    Known { schema: "IssueRequest", key: "description_html", value: "\"\"", answer_has: "Invalid HTML passed", reason: EMPTY_HTML },
    Known { schema: "PatchedIssueRequest", key: "description_html", value: "\"\"", answer_has: "Invalid HTML passed", reason: EMPTY_HTML },
    Known {
        schema: "IssueRequest",
        key: "deleted_at",
        value: "\"2026-01-01T00:00:00Z\"",
        answer_has: "Something went wrong please try again later",
        reason: "PQ-404 (research 0007, EX-001): a creation that gives `deleted_at` a value makes the work item soft-deleted in the pin and then \
                 raises, a 500 that is reproduced and not repaired: tests/work_items.rs `a_deleted_at_on_a_creation_is_the_pins_500_and_makes_nothing`",
    },
];

/// The disagreements that no entry of `known` explains, and the entries that explained one.
fn explain(known: &[Known], found: Vec<Disagreement>) -> (Vec<Disagreement>, BTreeSet<usize>) {
    let (mut left, mut used) = (Vec::new(), BTreeSet::new());
    for d in found {
        let entry = known.iter().position(|k| {
            d.allowed
                && k.schema == d.schema
                && k.key == d.key
                && Some(k.value) == d.value.as_deref()
                && d.answer.contains(k.answer_has)
        });
        match entry {
            Some(i) => {
                used.insert(i);
            }
            None => left.push(d),
        }
    }
    (left, used)
}

/// What the route answered to one probe, against what the schema says: `None` when they agree.
fn disagreement(op: &Operation, base: &Map<String, Value>, fx: &Fixtures, probe: &Probe) -> Option<Disagreement> {
    let (w, uri, fresh) = target(op);
    assert_eq!(
        &fresh, fx,
        "{}: the world is not the same twice, and the ids the probes use would not be the world's",
        op.id
    );
    let mut body = base.clone();
    if !probe.key.is_empty() {
        match &probe.value {
            Some(v) => body.insert(probe.key.clone(), v.clone()),
            None => body.remove(&probe.key),
        };
    }
    let r = send_json(&w.router, "key-admin", op.method, &uri, &Value::Object(body.clone()));
    let agrees = if probe.accepted {
        matches!(r.status, 200 | 201)
    } else {
        // refused under this key and no other: a refusal that blames another key is a different refusal
        let blamed: BTreeSet<String> = r.body.as_object().map(|m| m.keys().cloned().collect()).unwrap_or_default();
        r.status == 400 && blamed == BTreeSet::from([probe.key.clone()])
    };
    (!agrees).then(|| {
        let text: String = Value::Object(body).to_string().chars().take(160).collect();
        Disagreement {
            schema: op.schema.clone(),
            key: probe.key.clone(),
            value: probe.value.as_ref().map(Value::to_string),
            allowed: probe.accepted,
            status: r.status,
            answer: r.body.to_string(),
            shown: format!("{} {} {}: {}: {text}", op.id, op.schema, probe.key, probe.what),
        }
    })
}

/// Every disagreement of one schema's probes (`only`: of one key), over every operation that takes the schema.
fn conforms(doc: &Value, schema: &str, only: Option<&str>) -> (usize, Vec<Disagreement>) {
    let ops: Vec<Operation> = operations_with_a_body(doc).into_iter().filter(|o| o.schema == schema).collect();
    assert!(!ops.is_empty(), "no operation of the document takes {schema}");
    let (mut count, mut found) = (0, Vec::new());
    for op in &ops {
        let (_w, _uri, fx) = target(op);
        let base = base_body(doc, op, &fx);
        let (all, unsampled) = probes(doc, op, &fx, only);
        let expected: BTreeSet<&str> = UNSAMPLED.iter().filter(|(s, _)| *s == schema).map(|(_, k)| *k).collect();
        let got: BTreeSet<&str> = unsampled.iter().map(String::as_str).collect();
        if only.is_none() {
            assert_eq!(got, expected, "{}: the keys that name a row the world lacks (UNSAMPLED)", op.id);
        }
        for probe in &all {
            count += 1;
            found.extend(disagreement(op, &base, &fx, probe));
        }
    }
    (count, found)
}

#[rstest]
fn the_route_takes_what_the_schema_allows_and_refuses_what_it_forbids(
    #[values(
        "ProjectCreateRequest",
        "PatchedProjectUpdateRequest",
        "StateRequest",
        "PatchedStateRequest",
        "LabelCreateUpdateRequest",
        "PatchedLabelCreateUpdateRequest",
        "IssueRequest",
        "PatchedIssueRequest"
    )]
    schema: &str,
) {
    let doc = document();
    let (count, found) = conforms(&doc, schema, None);
    assert!(count > 40, "{schema}: only {count} probes: the generator reached almost nothing");
    let (unexplained, used) = explain(KNOWN, found);
    let lines: Vec<String> = unexplained.iter().map(ToString::to_string).collect();
    assert!(unexplained.is_empty(), "{schema}: {} of {count} probes disagree:\n{}", lines.len(), lines.join("\n"));
    let stale: Vec<_> = KNOWN
        .iter()
        .enumerate()
        .filter(|(i, k)| k.schema == schema && !used.contains(i))
        .map(|(_, k)| format!("{}.{} {} (pinned: {})", k.schema, k.key, k.value, k.reason))
        .collect();
    assert!(stale.is_empty(), "{schema}: entries of KNOWN that no probe met (stale): {stale:?}");
}

/// The list above is the document's: a request schema that is added to the document and not to the list is not checked, and this is
/// what says so.
#[test]
fn every_request_schema_of_the_document_is_one_the_test_above_runs() {
    let doc = document();
    let described: BTreeSet<String> = operations_with_a_body(&doc).into_iter().map(|o| o.schema).collect();
    let listed: BTreeSet<String> = [
        "ProjectCreateRequest",
        "PatchedProjectUpdateRequest",
        "StateRequest",
        "PatchedStateRequest",
        "LabelCreateUpdateRequest",
        "PatchedLabelCreateUpdateRequest",
        "IssueRequest",
        "PatchedIssueRequest",
    ]
    .map(String::from)
    .into();
    assert_eq!(described, listed);
    assert_eq!(operations_with_a_body(&doc).len(), 10, "the eight bodies, the two of them also under work-items/");
}

// --- the controls (AGENTS.md rule 8): the check must be able to fail -------------------------------------------------------------

/// The document broken in memory, one keyword at a time, each of them a way for a document and a parser to drift apart. The route is
/// the one the tests above pass, so every break must come back as a disagreement that no entry of `KNOWN` explains. The probes of the
/// broken key alone are sent.
#[test]
fn a_document_that_says_something_the_parser_does_not_keep_is_reported() {
    type Break = fn(&mut Value);
    // relative to what the schema says now: a bound is written once, in a type, and may change on purpose
    fn bump(bound: &mut Value, by: i64) {
        *bound = json!(bound.as_i64().expect("a bound that is a whole number") + by);
    }
    fn require(schema: &mut Value, key: &str, yes: bool) {
        let required = schema["required"].as_array_mut().expect("a creation requires something");
        required.retain(|k| k != key);
        if yes {
            required.push(json!(key));
        }
    }
    let breaks: &[(&str, &str, &str, Break)] = &[
        ("StateRequest", "name", "a maxLength one more", |s| bump(&mut s["properties"]["name"]["maxLength"], 1)),
        ("StateRequest", "name", "a maxLength one less", |s| bump(&mut s["properties"]["name"]["maxLength"], -1)),
        ("StateRequest", "name", "a minLength one less", |s| bump(&mut s["properties"]["name"]["minLength"], -1)),
        ("StateRequest", "name", "a null that the type allows", |s| {
            s["properties"]["name"]["type"] = json!(["string", "null"])
        }),
        ("StateRequest", "external_id", "no null in the type", |s| {
            s["properties"]["external_id"]["type"] = json!("string")
        }),
        ("StateRequest", "group", "a choice more", |s| {
            s["properties"]["group"]["enum"].as_array_mut().expect("an enum").push(json!("paused"));
        }),
        ("StateRequest", "name", "a key that is not required", |s| require(s, "name", false)),
        ("StateRequest", "description", "a key that is required", |s| require(s, "description", true)),
        ("PatchedProjectUpdateRequest", "archive_in", "a maximum one more", |s| {
            bump(&mut s["properties"]["archive_in"]["maximum"], 1)
        }),
        ("PatchedProjectUpdateRequest", "archive_in", "a maximum one less", |s| {
            bump(&mut s["properties"]["archive_in"]["maximum"], -1)
        }),
        ("PatchedProjectUpdateRequest", "close_in", "a minimum one less", |s| {
            bump(&mut s["properties"]["close_in"]["minimum"], -1)
        }),
        ("PatchedProjectUpdateRequest", "close_in", "a minimum one more", |s| {
            bump(&mut s["properties"]["close_in"]["minimum"], 1)
        }),
        ("PatchedProjectUpdateRequest", "timezone", "a zone more", |s| {
            s["properties"]["timezone"]["enum"].as_array_mut().expect("an enum").push(json!("Mars/Olympus_Mons"));
        }),
        ("IssueRequest", "start_date", "no format (any string)", |s| {
            s["properties"]["start_date"].as_object_mut().expect("an object").remove("format");
        }),
        ("IssueRequest", "point", "a string, not an integer", |s| {
            s["properties"]["point"]["type"] = json!(["string", "null"])
        }),
        ("IssueRequest", "priority", "a null that the type allows", |s| {
            s["properties"]["priority"] =
                json!({ "type": ["string", "null"], "enum": ["urgent", "high", "medium", "low", "none"] });
        }),
        ("LabelCreateUpdateRequest", "parent", "no null in the type", |s| {
            s["properties"]["parent"]["type"] = json!("string")
        }),
    ];
    let doc = document();
    for (schema, key, what, f) in breaks {
        // the unbroken key is quiet: the break, and only the break, is what is reported
        let (probes, quiet) = conforms(&doc, schema, Some(key));
        assert!(probes > 0, "{schema}.{key}: no probes");
        assert!(explain(KNOWN, quiet).0.is_empty(), "{schema}.{key}: the unbroken key already disagrees");
        let mut broken = doc.clone();
        f(&mut broken["components"]["schemas"][schema]);
        let (_, found) = conforms(&broken, schema, Some(key));
        let (reported, _) = explain(KNOWN, found);
        assert!(!reported.is_empty(), "{schema}.{key}, {what}: the break was not reported");
    }
}

/// The table of known refusals absorbs a disagreement only when it is the one named, with the answer named, and says which entries it
/// used, so that a stale one is seen.
#[test]
fn a_known_refusal_explains_only_the_disagreement_it_names() {
    let found = |schema: &str, key: &str, value: Option<&str>, allowed: bool, answer: &str| Disagreement {
        schema: schema.into(),
        key: key.into(),
        value: value.map(String::from),
        allowed,
        status: 400,
        answer: answer.into(),
        shown: String::new(),
    };
    let named = found(
        "StateRequest",
        "group",
        Some("\"triage\""),
        true,
        "{\"non_field_errors\":[\"Cannot create triage state\"]}",
    );
    let (left, used) = explain(KNOWN, vec![named]);
    assert!(left.is_empty() && used.len() == 1, "the named refusal is explained, by one entry");
    for (what, d) in [
        (
            "another schema",
            found("PatchedLabelCreateUpdateRequest", "group", Some("\"triage\""), true, "Cannot create triage state"),
        ),
        ("another key", found("StateRequest", "name", Some("\"triage\""), true, "Cannot create triage state")),
        ("another value", found("StateRequest", "group", Some("\"started\""), true, "Cannot create triage state")),
        ("another answer", found("StateRequest", "group", Some("\"triage\""), true, "This field may not be null.")),
        (
            "a value the schema forbids and the route took",
            found("StateRequest", "group", Some("\"triage\""), false, "Cannot create triage state"),
        ),
    ] {
        let (left, used) = explain(KNOWN, vec![d]);
        assert!(left.len() == 1 && used.is_empty(), "{what}: explained by an entry that does not name it");
    }
    // nothing met: every entry is unused, which the test of each schema reads as stale
    assert!(explain(KNOWN, Vec::new()).1.is_empty());
}
