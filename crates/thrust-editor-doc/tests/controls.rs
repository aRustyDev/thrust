//! Controls that MUST fail: a fidelity number is only evidence if the check that produced it can observe a defect.
use base64::Engine;
use serde_json::Value;
use thrust_editor_doc::{
    domparse, html,
    pm::{norm_num, Node},
    schema::{AttrDefault, Schema, Variant},
    ydoc,
};

fn load() -> Value {
    let p = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/corpus/corpus.json");
    serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
}
fn dump() -> Value {
    let p = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/corpus/schema.json");
    serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
}
fn same(a: &Value, b: &Value) -> bool {
    norm_num(a.clone()) == norm_num(b.clone())
}
fn b64d(s: &str) -> Vec<u8> {
    base64::engine::general_purpose::STANDARD.decode(s).unwrap()
}
fn variant(d: &Value) -> Variant {
    if d["variant"] == "rich" {
        Variant::Rich
    } else {
        Variant::Document
    }
}

/// number of documents (of those given) where one of D2/D3/D4 disagrees with the oracle, under `mk_schema`
fn failing_docs(mk: &dyn Fn(Variant) -> Schema, c: &Value) -> Vec<String> {
    let mut bad = vec![];
    for d in c["docs"].as_array().unwrap() {
        if d.get("json").is_none() {
            continue;
        }
        let schema = mk(variant(d));
        let mut fail = false;
        if let Some(b) = d.get("binary").and_then(|b| b.as_str()) {
            match ydoc::decode(&schema, &b64d(b), "default") {
                Ok(n) => fail |= !same(&n.to_json(&schema), &d["json"]),
                Err(_) => fail = true,
            }
        }
        if let Ok(node) = Node::from_json(&schema, &d["json"]) {
            fail |= html::to_html(&schema, &node) != d["html"].as_str().unwrap();
        }
        let h = d
            .get("html_in")
            .and_then(|x| x.as_str())
            .or_else(|| d["html"].as_str())
            .unwrap();
        fail |= !same(
            &domparse::html_to_doc(&schema, h).to_json(&schema),
            &d["html_in_json"],
        );
        if fail {
            bad.push(d["id"].as_str().unwrap().to_string());
        }
    }
    bad
}

#[test]
fn baseline_has_zero_failures_and_no_schema_drift() {
    let c = load();
    let bad = failing_docs(&|v| Schema::new(v), &c);
    assert!(
        bad.is_empty(),
        "unmutated Rust implementation disagrees with the oracle on {bad:?}"
    );
    for v in [Variant::Rich, Variant::Document] {
        let key = if v == Variant::Rich {
            "rich"
        } else {
            "document"
        };
        let drift = Schema::new(v).diff_against_dump(&dump()[key]);
        assert!(
            drift.is_empty(),
            "schema drift against the real dump ({key}): {drift:?}"
        );
    }
}

/// (node-or-mark, attr, new default json)
const MUTANTS: &[(&str, &str, &str)] = &[
    ("image", "width", "\"36%\""),
    ("heading", "level", "2"),
    ("taskItem", "checked", "true"),
    ("tableCell", "colwidth", "[151]"),
    ("calloutComponent", "data-emoji-unicode", "\"128162\""),
    ("orderedList", "start", "2"),
    ("imageComponent", "status", "\"uploaded\""),
    ("tableHeader", "background", "\"white\""),
    ("mark:link", "rel", "\"noopener\""),
    ("mark:customColor", "color", "\"gray\""),
];

fn mutated(v: Variant, which: &(&str, &str, &str)) -> Schema {
    let mut s = Schema::new(v);
    let new: Value = serde_json::from_str(which.2).unwrap();
    if let Some(m) = which.0.strip_prefix("mark:") {
        let t = s.mark_type(m).unwrap();
        let a = s.marks[t]
            .attrs
            .iter_mut()
            .find(|a| a.name == which.1)
            .unwrap();
        a.default = AttrDefault::Val(new);
    } else {
        let t = s.node_type(which.0).unwrap();
        let a = s.nodes[t]
            .attrs
            .iter_mut()
            .find(|a| a.name == which.1)
            .unwrap();
        a.default = AttrDefault::Val(new);
    }
    s
}

#[test]
fn mutated_schema_is_detected_by_the_corpus_and_by_the_dump_diff() {
    let c = load();
    let mut report = vec![];
    let mut by_corpus = 0;
    for m in MUTANTS {
        let bad = failing_docs(&|v| mutated(v, m), &c);
        let drift = mutated(Variant::Document, m).diff_against_dump(&dump()["document"]);
        report.push(format!(
            "{:>18}.{:<18} -> {:>3} corpus docs fail, dump-diff reports {} mismatch(es)",
            m.0,
            m.1,
            bad.len(),
            drift.len()
        ));
        assert!(
            !drift.is_empty(),
            "mutant {m:?} was NOT detected by the schema dump diff"
        );
        assert!(
            !bad.is_empty() || !drift.is_empty(),
            "mutant {m:?} was NOT detected at all"
        );
        if !bad.is_empty() {
            by_corpus += 1;
        }
    }
    eprintln!(
        "MUTANTS (every one must be detected; corpus alone caught {by_corpus}/{}):\n{}",
        MUTANTS.len(),
        report.join("\n")
    );
    // `heading.level` default is a known corpus blind spot (every heading in the corpus carries an explicit level)
    assert!(by_corpus >= MUTANTS.len() - 1);
}

#[test]
fn comparator_discriminates_between_different_documents() {
    // decode(binary of doc i) compared with the JSON of a DIFFERENT doc must not be equal
    let c = load();
    let docs: Vec<&Value> = c["docs"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|d| d.get("binary").is_some() && d["kind"] != "foreign")
        .collect();
    let (mut checked, mut equal) = (0, 0);
    for w in docs.windows(2) {
        if same(&w[0]["json"], &w[1]["json"]) {
            continue; // identical documents are legitimately equal
        }
        let schema = Schema::new(Variant::Document);
        let got = ydoc::decode(&schema, &b64d(w[0]["binary"].as_str().unwrap()), "default")
            .unwrap()
            .to_json(&schema);
        checked += 1;
        if same(&got, &w[1]["json"]) {
            equal += 1;
        }
    }
    eprintln!("cross-document control: {equal} false matches in {checked} comparisons");
    assert_eq!(equal, 0);
    assert!(checked > 100);
}

/// Deterministic corruption. Returns (name, bytes).
fn corruptions(bin: &[u8], salt: usize) -> Vec<(&'static str, Vec<u8>)> {
    let n = bin.len();
    let mut out = vec![];
    out.push(("truncate-50%", bin[..n / 2].to_vec()));
    out.push(("truncate-last-byte", bin[..n.saturating_sub(1)].to_vec()));
    let mut f = bin.to_vec();
    let at = (salt * 7919 + n / 3) % n.max(1);
    f[at] ^= 0xff;
    out.push(("flip-byte", f));
    let mut z = bin.to_vec();
    let at = (salt * 104729 + n / 2) % n.max(1);
    for i in at..(at + 4).min(n) {
        z[i] = 0;
    }
    out.push(("zero-4-bytes", z));
    out.push(("garbage", vec![0xde, 0xad, 0xbe, 0xef, 1, 2, 3]));
    out.push(("empty", vec![]));
    out
}

#[test]
fn corrupted_binaries_are_detected() {
    let c = load();
    let schema = Schema::new(Variant::Document);
    let mut stats: std::collections::BTreeMap<&str, (usize, usize, usize, usize)> =
        Default::default(); // total, err, differs, undetected
    let mut records = vec![];
    for (i, d) in c["docs"].as_array().unwrap().iter().enumerate() {
        let Some(bs) = d.get("binary").and_then(|b| b.as_str()) else {
            continue;
        };
        if d["kind"] == "foreign" || d["id"] == "empty" {
            continue;
        }
        let bin = b64d(bs);
        for (name, bad) in corruptions(&bin, i) {
            let e = stats.entry(name).or_default();
            e.0 += 1;
            let verdict = match ydoc::decode(&schema, &bad, "default") {
                Err(_) => {
                    e.1 += 1;
                    "error"
                }
                Ok(n) => {
                    if same(&n.to_json(&schema), &d["json"]) {
                        e.3 += 1;
                        "undetected"
                    } else {
                        e.2 += 1;
                        "differs"
                    }
                }
            };
            records.push(serde_json::json!({"id": d["id"], "corruption": name, "rust": verdict, "binary": base64::engine::general_purpose::STANDARD.encode(&bad), "expected_json": d["json"]}));
        }
    }
    eprintln!("CORRUPTION CONTROL (Rust yrs reader): corruption -> total / error / decoded-but-different / UNDETECTED");
    for (k, v) in &stats {
        eprintln!("  {k:20} {:4} / {:4} / {:4} / {:4}", v.0, v.1, v.2, v.3);
    }
    // Evidence for tools/spike-0006/oracle/verify-rust-out.ts, which replays these corruptions through Yjs. Written
    // only on request (SPIKE0006_OUT=<dir>, relative to the workspace root), so the default test loop writes nothing
    // outside a temp dir.
    if let Some(dir) = std::env::var_os("SPIKE0006_OUT") {
        let dir = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../..")).join(dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("corrupt.json"),
            serde_json::to_string(&records).unwrap(),
        )
        .unwrap();
    }
    // structural corruption (truncation, garbage, empty) must never decode to the original document
    for k in ["truncate-50%", "garbage", "empty"] {
        assert_eq!(
            stats[k].3, 0,
            "{k}: a corrupted binary decoded to the ORIGINAL document"
        );
    }
}
