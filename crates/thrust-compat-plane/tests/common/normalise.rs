//! Normalisation of exchanges, the Rust half of a pair.
//!
//! The Python half is tools/compat-record/normalise.py, which records the fixtures. Both read
//! tests/parity/normalise.json and must give the same output for the same input, in the same traversal order: object
//! keys sorted, then per exchange the request path, the request body, the response body, numbering ids across the whole
//! fixture. `parity_normaliser_cases` (tests/parity.rs) runs the cases both halves share. Change a rule in both, and add
//! a case.

use std::collections::BTreeMap;

use serde_json::{json, Map, Value};

pub struct Rules {
    uuid_token: String,
    dt_suffixes: Vec<String>,
    dt_exact: Vec<String>,
    dt_token: String,
    date_suffixes: Vec<String>,
    date_token: String,
    opaque: BTreeMap<String, String>,
    headers_keep: Vec<String>,
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array().expect("array").iter().map(|s| s.as_str().expect("string").to_string()).collect()
}

impl Rules {
    pub fn load() -> Rules {
        let v: Value = serde_json::from_str(include_str!("../parity/normalise.json")).expect("normalise.json");
        let opaque = v["opaque_paths"]
            .as_object()
            .expect("opaque_paths")
            .iter()
            .filter(|(k, _)| !k.starts_with('_'))
            .map(|(k, t)| (k.clone(), t.as_str().expect("token").to_string()))
            .collect();
        Rules {
            uuid_token: v["uuid_token"].as_str().expect("uuid_token").to_string(),
            dt_suffixes: strings(&v["datetime"]["key_suffixes"]),
            dt_exact: strings(&v["datetime"]["key_exact"]),
            dt_token: v["datetime"]["token"].as_str().expect("token").to_string(),
            date_suffixes: strings(&v["date"]["key_suffixes"]),
            date_token: v["date"]["token"].as_str().expect("token").to_string(),
            opaque,
            headers_keep: strings(&v["headers_keep"]),
        }
    }
}

fn is_hex(c: char) -> bool {
    c.is_ascii_hexdigit()
}

fn hex_run(s: &[char], start: usize, len: usize) -> bool {
    start + len <= s.len() && s[start..start + len].iter().all(|c| is_hex(*c))
}

fn uuid_at(s: &[char], i: usize) -> bool {
    if i + 36 > s.len() {
        return false;
    }
    [(0, 8), (9, 4), (14, 4), (19, 4), (24, 12)].iter().all(|(pos, len)| hex_run(s, i + pos, *len))
        && [8, 13, 18, 23].iter().all(|p| s[i + p] == '-')
}

/// Ordinal numbering of ids by first appearance within one fixture.
pub struct Ids {
    token: String,
    seen: Vec<String>,
}

impl Ids {
    pub fn new(rules: &Rules) -> Ids {
        Ids { token: rules.uuid_token.clone(), seen: Vec::new() }
    }

    fn ordinal(&mut self, raw: &[char]) -> String {
        let h: String = raw.iter().filter(|c| **c != '-').collect::<String>().to_lowercase();
        let key = format!("{}-{}-{}-{}-{}", &h[0..8], &h[8..12], &h[12..16], &h[16..20], &h[20..32]);
        let n = match self.seen.iter().position(|k| *k == key) {
            Some(p) => p + 1,
            None => {
                self.seen.push(key);
                self.seen.len()
            }
        };
        self.token.replace("{n}", &n.to_string())
    }

    pub fn scrub(&mut self, s: &str) -> String {
        let cs: Vec<char> = s.chars().collect();
        let mut out = String::new();
        let mut i = 0;
        while i < cs.len() {
            let clear_before = !(i > 0 && is_hex(cs[i - 1]));
            if uuid_at(&cs, i) && clear_before && !(i + 36 < cs.len() && is_hex(cs[i + 36])) {
                out.push_str(&self.ordinal(&cs[i..i + 36]));
                i += 36;
            } else if hex_run(&cs, i, 32) && clear_before && !(i + 32 < cs.len() && is_hex(cs[i + 32])) {
                out.push_str(&self.ordinal(&cs[i..i + 32]));
                i += 32;
            } else {
                out.push(cs[i]);
                i += 1;
            }
        }
        out
    }
}

fn is_datetime(s: &str) -> bool {
    // ^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}
    let b = s.as_bytes();
    b.len() >= 19
        && b[..19].iter().enumerate().all(|(i, c)| match i {
            4 | 7 => *c == b'-',
            10 => *c == b'T',
            13 | 16 => *c == b':',
            _ => c.is_ascii_digit(),
        })
}

fn is_date(s: &str) -> bool {
    // ^\d{4}-\d{2}-\d{2}$
    let b = s.as_bytes();
    b.len() == 10
        && b.iter().enumerate().all(|(i, c)| match i {
            4 | 7 => *c == b'-',
            _ => c.is_ascii_digit(),
        })
}

fn key_matches(key: &str, suffixes: &[String], exact: &[String]) -> bool {
    exact.iter().any(|e| e == key) || suffixes.iter().any(|s| key.ends_with(s.as_str()))
}

/// Normalise one JSON value; `path` is its dotted path with `[]` for array elements.
pub fn value(rules: &Rules, ids: &mut Ids, v: &Value, path: &str) -> Value {
    match v {
        Value::Object(m) => {
            let mut keys: Vec<&String> = m.keys().collect();
            keys.sort();
            let mut out = Map::new();
            for k in keys {
                let p = if path.is_empty() { k.clone() } else { format!("{path}.{k}") };
                let child = &m[k];
                let normalised = if let (Some(token), false) = (rules.opaque.get(&p), child.is_null()) {
                    Value::String(token.clone())
                } else if child.as_str().is_some_and(|s| key_matches(k, &rules.dt_suffixes, &rules.dt_exact) && is_datetime(s)) {
                    Value::String(rules.dt_token.clone())
                } else if child.as_str().is_some_and(|s| key_matches(k, &rules.date_suffixes, &[]) && is_date(s)) {
                    Value::String(rules.date_token.clone())
                } else {
                    value(rules, ids, child, &p)
                };
                out.insert(k.clone(), normalised);
            }
            Value::Object(out)
        }
        Value::Array(a) => Value::Array(a.iter().map(|x| value(rules, ids, x, &format!("{path}[]"))).collect()),
        Value::String(s) => Value::String(ids.scrub(s)),
        other => other.clone(),
    }
}

fn headers(rules: &Rules, hs: Option<&Value>) -> Value {
    let mut out = Map::new();
    if let Some(Value::Object(m)) = hs {
        for (k, v) in m {
            let lower = k.to_lowercase();
            if rules.headers_keep.contains(&lower) {
                out.insert(lower, v.clone());
            }
        }
    }
    Value::Object(out)
}

/// Normalise one exchange in the fixed traversal order: request path, request body, response body.
pub fn exchange(rules: &Rules, ids: &mut Ids, ex: &Value) -> Value {
    let (req, resp) = (&ex["request"], &ex["response"]);
    let mut out_req = json!({"method": req["method"], "path": ids.scrub(req["path"].as_str().expect("request.path"))});
    if let Some(body) = req.get("body").filter(|b| !b.is_null()) {
        out_req["body"] = value(rules, ids, body, "");
    }
    let body = match resp.get("body").filter(|b| !b.is_null()) {
        Some(b) => value(rules, ids, b, ""),
        None => Value::Null,
    };
    json!({
        "name": ex["name"],
        "request": out_req,
        "response": {"status": resp["status"], "headers": headers(rules, resp.get("headers")), "body": body},
    })
}
