//! Spike 0006 fidelity harness: runs every direction against the JS oracle corpus and prints per-node-type tables.
//!
//!   scripts/rs cargo run --release -p thrust-editor-doc --example fidelity -- [--large] [--out target/spike-0006]
//!
//! Directions (all compare to what the REAL @plane/editor code produced for the same input):
//!   D1  json -> yjs binary -> json         (round trip through the Rust writer and reader)
//!   D1s json -> yjs structure              (the Rust writer's XmlFragment equals y-prosemirror's, node for node)
//!   D2  oracle yjs binary -> json          (Rust reader on binaries written by JS, incl. tombstones/2 clients)
//!   D3  json -> html                       (byte equal to generateHTML)
//!   D4  html -> json                       (equal to generateJSON)
//!   D5  html -> all formats                (API v1 write: html -> json -> binary -> json/html == convertHTMLDocumentToAllFormats)
use base64::Engine;
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::time::Instant;
use thrust_editor_doc::{domparse, html, pm::{norm_num, Node}, schema::{Schema, Variant}, ydoc};

fn b64d(s: &str) -> Vec<u8> {
    base64::engine::general_purpose::STANDARD.decode(s).unwrap()
}
fn b64e(b: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(b)
}
fn same(a: &Value, b: &Value) -> bool {
    norm_num(a.clone()) == norm_num(b.clone())
}

fn first_diff(a: &Value, b: &Value, path: &str) -> String {
    match (a, b) {
        (Value::Object(x), Value::Object(y)) => {
            for k in x.keys().chain(y.keys()) {
                match (x.get(k), y.get(k)) {
                    (Some(p), Some(q)) => {
                        if !same(p, q) {
                            return first_diff(p, q, &format!("{path}/{k}"));
                        }
                    }
                    (p, q) => return format!("{path}/{k}: got {:?} want {:?}", p.map(|v| v.to_string()), q.map(|v| v.to_string())),
                }
            }
            String::from("(equal)")
        }
        (Value::Array(x), Value::Array(y)) => {
            for i in 0..x.len().max(y.len()) {
                match (x.get(i), y.get(i)) {
                    (Some(p), Some(q)) => {
                        if !same(p, q) {
                            return first_diff(p, q, &format!("{path}/{i}"));
                        }
                    }
                    (p, q) => return format!("{path}/{i}: got {:?} want {:?}", p.map(|v| v.to_string().chars().take(80).collect::<String>()), q.map(|v| v.to_string().chars().take(80).collect::<String>())),
                }
            }
            String::from("(equal)")
        }
        _ => format!("{path}: got {} want {}", a.to_string().chars().take(100).collect::<String>(), b.to_string().chars().take(100).collect::<String>()),
    }
}

struct DocRec {
    id: String,
    kind: String,
    variant: Variant,
    nodes: Vec<String>,
    marks: Vec<String>,
    v: Value,
}

#[derive(Default)]
struct Tally {
    pass: usize,
    total: usize,
    fails: Vec<String>,
}

const DIRS: &[(&str, &str)] = &[
    ("D1", "json -> binary -> json"),
    ("D1s", "json -> yjs structure == y-prosemirror"),
    ("D2", "oracle binary -> json"),
    ("D3", "json -> html"),
    ("D4", "html -> json"),
    ("D5", "html -> all formats (v1 write)"),
];

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let large = args.iter().any(|a| a == "--large");
    let out_dir = args.iter().position(|a| a == "--out").and_then(|i| args.get(i + 1)).cloned().unwrap_or_else(|| concat!(env!("CARGO_MANIFEST_DIR"), "/../../target/spike-0006").to_string());
    let corpus_path = if large { format!("{out_dir}/corpus-large.json") } else { concat!(env!("CARGO_MANIFEST_DIR"), "/tests/corpus/corpus.json").to_string() };
    let corpus: Value = serde_json::from_str(&std::fs::read_to_string(&corpus_path).expect("corpus")).unwrap();
    let docs: Vec<DocRec> = corpus["docs"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|d| d.get("json").is_some())
        .map(|d| DocRec {
            id: d["id"].as_str().unwrap().to_string(),
            kind: d["kind"].as_str().unwrap().to_string(),
            variant: if d["variant"] == "rich" { Variant::Rich } else { Variant::Document },
            nodes: d["nodes"].as_array().unwrap().iter().map(|x| x.as_str().unwrap().to_string()).collect(),
            marks: d["marks"].as_array().unwrap().iter().map(|x| x.as_str().unwrap().to_string()).collect(),
            v: d.clone(),
        })
        .collect();
    let rich = Schema::new(Variant::Rich);
    let docs_schema = Schema::new(Variant::Document);
    let schema_of = |v: Variant| if v == Variant::Rich { &rich } else { &docs_schema };

    // by direction -> tally; by direction -> (type -> tally)
    let mut dir_tally: BTreeMap<&str, Tally> = DIRS.iter().map(|(k, _)| (*k, Tally::default())).collect();
    let mut by_type: BTreeMap<&str, BTreeMap<String, (usize, usize)>> = DIRS.iter().map(|(k, _)| (*k, BTreeMap::new())).collect();
    let mut time_us: BTreeMap<&str, u128> = BTreeMap::new();
    let mut rust_out: Vec<Value> = vec![];
    let mut kind_tally: BTreeMap<(String, &str), (usize, usize)> = BTreeMap::new();
    let mut instances: BTreeMap<&str, (usize, usize)> = BTreeMap::new();

    for d in &docs {
        let schema = schema_of(d.variant);
        let v = &d.v;
        let want_json = &v["json"];
        let node_count = count_nodes(want_json);
        let mut rec = |dir: &'static str, pass: bool, msg: String, micros: u128| {
            let t = dir_tally.get_mut(dir).unwrap();
            t.total += 1;
            if pass {
                t.pass += 1;
            } else {
                t.fails.push(format!("{} [{}]: {msg}", d.id, d.kind));
            }
            *time_us.entry(dir).or_default() += micros;
            for ty in d.nodes.iter().chain(d.marks.iter()) {
                let e = by_type.get_mut(dir).unwrap().entry(ty.clone()).or_default();
                e.1 += 1;
                if pass {
                    e.0 += 1;
                }
            }
            let k = kind_tally.entry((d.kind.clone(), dir)).or_default();
            k.1 += 1;
            if pass {
                k.0 += 1;
            }
            let ins = instances.entry(dir).or_default();
            ins.1 += node_count;
            if pass {
                ins.0 += node_count;
            }
        };

        let mut out = Map::new();
        out.insert("id".into(), Value::String(d.id.clone()));

        // D1 / D1s : json -> binary -> json
        let src = v.get("authored").unwrap_or(want_json);
        if d.kind != "foreign" || v.get("binary").is_some() {
            let node = Node::from_json(schema, src).expect("corpus json is valid");
            let title_node = v.get("title").and_then(|t| t.as_str()).map(|t| {
                let tj = json!({"type":"doc","content":[{"type":"heading","attrs":{"level":1},"content": if t.is_empty() { Value::Null } else { json!([{"type":"text","text":t}]) }}]});
                let mut tj = tj;
                if t.is_empty() {
                    tj["content"][0].as_object_mut().unwrap().remove("content");
                }
                Node::from_json(schema, &tj).unwrap()
            });
            let t0 = Instant::now();
            let bin = ydoc::encode(schema, &node, title_node.as_ref(), Some(1000));
            let back = ydoc::decode(schema, &bin, "default");
            let us = t0.elapsed().as_micros();
            let (pass, msg) = match &back {
                Ok(n) => {
                    let j = n.to_json(schema);
                    let mut ok = same(&j, want_json);
                    let mut msg = if ok { String::new() } else { first_diff(&j, want_json, "") };
                    if ok {
                        if let (Some(t), Some(tj)) = (v.get("title"), v.get("title_json")) {
                            let tn = ydoc::decode(schema, &bin, "title").map(|n| n.to_json(schema));
                            if !matches!(&tn, Ok(x) if same(x, tj)) {
                                ok = false;
                                msg = format!("title fragment differs for {t}: {}", match &tn {
                                    Ok(x) => first_diff(x, tj, ""),
                                    Err(e) => e.to_string(),
                                });
                            }
                        }
                    }
                    (ok, msg)
                }
                Err(e) => (false, e.to_string()),
            };
            rec("D1", pass, msg, us);
            out.insert("binary".into(), Value::String(b64e(&bin)));
            if d.kind != "history" && d.kind != "legacy" {
                if let (Some(ys), Ok(mine)) = (v.get("ystruct"), ydoc::struct_dump(&bin, "default")) {
                    let ok = same(&mine, ys);
                    rec("D1s", ok, if ok { String::new() } else { first_diff(&mine, ys, "") }, 0);
                }
            }
        }

        // D2 : oracle binary -> json
        if let Some(bs) = v.get("binary").and_then(|b| b.as_str()) {
            let bin = b64d(bs);
            let t0 = Instant::now();
            let r = ydoc::decode(schema, &bin, "default");
            let us = t0.elapsed().as_micros();
            match r {
                Ok(n) => {
                    let j = n.to_json(schema);
                    let ok = same(&j, want_json);
                    rec("D2", ok, if ok { String::new() } else { first_diff(&j, want_json, "") }, us);
                }
                Err(e) => rec("D2", false, e.to_string(), us),
            }
        }

        // D3 : json -> html
        {
            let node = Node::from_json(schema, want_json).unwrap();
            let t0 = Instant::now();
            let got = html::to_html(schema, &node);
            let us = t0.elapsed().as_micros();
            let want = v["html"].as_str().unwrap();
            let ok = got == want;
            let msg = if ok {
                String::new()
            } else {
                let at = got.bytes().zip(want.bytes()).position(|(a, b)| a != b).unwrap_or(got.len().min(want.len()));
                format!("first diff at byte {at}")
            };
            rec("D3", ok, msg, us);
            out.insert("html".into(), Value::String(got));
        }

        // D4 : html -> json
        {
            let h = v.get("html_in").and_then(|x| x.as_str()).or_else(|| v["html"].as_str()).unwrap();
            let t0 = Instant::now();
            let doc = domparse::html_to_doc(schema, h);
            let us = t0.elapsed().as_micros();
            let j = doc.to_json(schema);
            let ok = same(&j, &v["html_in_json"]);
            rec("D4", ok, if ok { String::new() } else { first_diff(&j, &v["html_in_json"], "") }, us);
            out.insert("html_json".into(), j);
        }

        // D5 : v1 write pipeline (foreign html only)
        if d.kind == "foreign" {
            let h = v["html_in"].as_str().unwrap();
            let t0 = Instant::now();
            let doc = domparse::html_to_doc(schema, h);
            let bin = ydoc::encode(schema, &doc, None, Some(2000));
            let back = ydoc::decode(schema, &bin, "default");
            let us = t0.elapsed().as_micros();
            match back {
                Ok(n) => {
                    let j = n.to_json(schema);
                    let hh = html::to_html(schema, &n);
                    let ok = same(&j, want_json) && hh == v["html"].as_str().unwrap();
                    rec("D5", ok, if ok { String::new() } else { first_diff(&j, want_json, "") }, us);
                    out.insert("v1_binary".into(), Value::String(b64e(&bin)));
                    out.insert("v1_json".into(), j);
                    out.insert("v1_html".into(), Value::String(hh));
                }
                Err(e) => rec("D5", false, e.to_string(), us),
            }
        }
        rust_out.push(Value::Object(out));
    }

    // ---------------------------------------------------------------- report
    println!("# Fidelity report ({} documents from {})\n", docs.len(), corpus_path);
    println!("| dir | what | pass / total | % |");
    println!("|---|---|---|---|");
    let (mut p, mut t) = (0, 0);
    for (k, label) in DIRS {
        let tl = &dir_tally[k];
        println!("| {k} | {label} | {} / {} | {:.2} |", tl.pass, tl.total, 100.0 * tl.pass as f64 / tl.total.max(1) as f64);
        p += tl.pass;
        t += tl.total;
    }
    println!("| all | every (document, direction) pair | {p} / {t} | {:.2} |\n", 100.0 * p as f64 / t.max(1) as f64);
    println!("Instance-weighted (share of node instances in documents that pass):");
    for (k, _) in DIRS {
        let (a, b) = instances.get(k).copied().unwrap_or((0, 0));
        println!("  {k}: {a}/{b} = {:.2}%", 100.0 * a as f64 / b.max(1) as f64);
    }
    println!("\nBy document kind (pass/total per direction):");
    for ((kind, dir), (a, b)) in &kind_tally {
        println!("  {kind:8} {dir:3} {a}/{b}");
    }
    println!("\nPer node/mark type, documents containing the type that pass (D1 / D2 / D3 / D4):");
    let mut all_types: Vec<String> = by_type["D1"].keys().cloned().collect();
    all_types.sort();
    println!("| type | docs | D1 | D2 | D3 | D4 |");
    println!("|---|---|---|---|---|---|");
    for ty in &all_types {
        let cell = |dir: &str| {
            let (a, b) = by_type[dir].get(ty).copied().unwrap_or((0, 0));
            format!("{a}/{b} ({:.1}%)", 100.0 * a as f64 / b.max(1) as f64)
        };
        let n = by_type["D1"][ty].1;
        println!("| {ty} | {n} | {} | {} | {} | {} |", cell("D1"), cell("D2"), cell("D3"), cell("D4"));
    }
    println!("\nCPU (native release, whole corpus): {}", DIRS.iter().map(|(k, _)| format!("{k}={}us", time_us.get(k).copied().unwrap_or(0))).collect::<Vec<_>>().join("  "));
    for (k, _) in DIRS {
        let tl = &dir_tally[k];
        if !tl.fails.is_empty() {
            println!("\nFailures in {k} ({}):", tl.fails.len());
            for f in tl.fails.iter().take(30) {
                println!("  - {f}");
            }
        }
    }
    std::fs::create_dir_all(&out_dir).ok();
    let name = if large { "rust-out-large.json" } else { "rust-out.json" };
    std::fs::write(format!("{out_dir}/{name}"), serde_json::to_string(&json!({"docs": rust_out})).unwrap()).unwrap();
    let fid: Vec<Value> = DIRS.iter().map(|(k, l)| json!({"dir": k, "what": l, "pass": dir_tally[k].pass, "total": dir_tally[k].total})).collect();
    std::fs::write(format!("{out_dir}/{}", if large { "fidelity-large.json" } else { "fidelity.json" }), serde_json::to_string_pretty(&fid).unwrap()).unwrap();
}

fn count_nodes(v: &Value) -> usize {
    let mut n = 1;
    if let Some(c) = v.get("content").and_then(|c| c.as_array()) {
        for x in c {
            n += count_nodes(x);
        }
    }
    n
}
