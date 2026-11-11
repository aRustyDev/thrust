//! Normalisation of exchanges, the Rust half of a pair.
//!
//! The Python half is tools/compat-record/normalise.py, which records the fixtures. Both read
//! tests/parity/normalise.json and must give the same output for the same input, in the same traversal order: object
//! keys sorted, then per exchange the request path, the request body, the response body, numbering ids across the whole
//! fixture. `parity_normaliser_cases` (tests/parity.rs) runs the cases both halves share. Change a rule in both, and add
//! a case.

use std::collections::BTreeMap;

use serde_json::{json, Map, Value};
use thrust_contracts::Uuid;

pub struct Rules {
    uuid_token: String,
    dt_suffixes: Vec<String>,
    dt_exact: Vec<String>,
    dt_token: String,
    date_suffixes: Vec<String>,
    date_token: String,
    opaque: BTreeMap<String, String>,
    opaque_keys: BTreeMap<String, String>,
    headers_keep: Vec<String>,
    origin_headers: Vec<String>,
    origin_token: String,
    presigned: Presigned,
}

/// The `presigned_url` rule of normalise.json: what a URL of the object store looks like once what a correct implementation cannot
/// reproduce is taken out of it.
struct Presigned {
    marker: String,
    body_paths: Vec<String>,
    headers: Vec<String>,
    origin_token: String,
    query_tokens: BTreeMap<String, String>,
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
        let opaque_keys = v["opaque_keys"]
            .as_object()
            .expect("opaque_keys")
            .iter()
            .filter(|(k, _)| !k.starts_with('_'))
            .map(|(k, t)| (k.clone(), t.as_str().expect("token").to_string()))
            .collect();
        Rules {
            opaque_keys,
            uuid_token: v["uuid_token"].as_str().expect("uuid_token").to_string(),
            dt_suffixes: strings(&v["datetime"]["key_suffixes"]),
            dt_exact: strings(&v["datetime"]["key_exact"]),
            dt_token: v["datetime"]["token"].as_str().expect("token").to_string(),
            date_suffixes: strings(&v["date"]["key_suffixes"]),
            date_token: v["date"]["token"].as_str().expect("token").to_string(),
            opaque,
            headers_keep: strings(&v["headers_keep"]),
            origin_headers: strings(&v["header_origin_rewrite"]["headers"]),
            origin_token: v["header_origin_rewrite"]["token"].as_str().expect("token").to_string(),
            presigned: Presigned {
                marker: v["presigned_url"]["marker"].as_str().expect("marker").to_string(),
                body_paths: strings(&v["presigned_url"]["body_paths"]),
                headers: strings(&v["presigned_url"]["headers"]),
                origin_token: v["presigned_url"]["origin_token"].as_str().expect("origin_token").to_string(),
                query_tokens: v["presigned_url"]["query_tokens"]
                    .as_object()
                    .expect("query_tokens")
                    .iter()
                    .map(|(k, t)| (k.clone(), t.as_str().expect("token").to_string()))
                    .collect(),
            },
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

    /// Number an id before anything is normalised, so it is the next ordinal whatever comes first. A fixture whose requests name
    /// an id that an earlier fixture made (the project the journey created) lists it under `uses`, and the replay gives it
    /// here the id THIS implementation made for it; the recorder did the same with the pin's (`Ids.preseed` in normalise.py).
    pub fn preseed(&mut self, id: &str) {
        let cs: Vec<char> = id.chars().collect();
        self.ordinal(&cs);
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

    /// The reverse of [`Ids::scrub`] for a request: each `<uuid:N>` is replaced by the id that is N-th in order of first
    /// appearance so far in this fixture, which is the id THIS implementation made for it. A recorded request that carries an
    /// id from an earlier response (the workspace a profile names as its last one) is replayed with the matching id of the
    /// replay, because ids differ between the pin and Thrust and only their identity is the contract.
    pub fn resolve(&self, s: &str) -> String {
        let (pre, post) = self.token.split_once("{n}").expect("the id token holds {n}");
        let (mut out, mut rest) = (String::new(), s);
        while let Some(i) = rest.find(pre) {
            let after = &rest[i + pre.len()..];
            let digits: String = after.chars().take_while(char::is_ascii_digit).collect();
            if !digits.is_empty() && after[digits.len()..].starts_with(post) {
                let n: usize = digits.parse().expect("digits");
                // An id that no earlier exchange produced is not a reference but a constant the recorder chose (the pin
                // ignores a `user` and an `id` in a profile patch, say): any id will do, so a fixed fresh one stands in
                // for it, the same each time, and the normaliser then numbers it where the recording did.
                let id = match n.checked_sub(1).and_then(|k| self.seen.get(k)) {
                    Some(seen) => seen.clone(),
                    None => {
                        Uuid::from_u128(0xF0F0_0000_0000_0000_0000_0000_0000_0000 + n as u128).hyphenated().to_string()
                    }
                };
                out.push_str(&rest[..i]);
                out.push_str(&id);
                rest = &after[digits.len() + post.len()..];
            } else {
                out.push_str(&rest[..i + pre.len()]);
                rest = after;
            }
        }
        out.push_str(rest);
        out
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
    value_with(rules, ids, v, path, false)
}

/// [`value`], and `keep_times` leaves a date or a timestamp as it is: a request body is what the client sent, and a replay must
/// be able to send the same date (an implementation answers a date that is no date, and a start that is after a target), where a
/// response's times are the server's own.
pub fn value_with(rules: &Rules, ids: &mut Ids, v: &Value, path: &str, keep_times: bool) -> Value {
    match v {
        Value::Object(m) => {
            let mut keys: Vec<&String> = m.keys().collect();
            keys.sort();
            let mut out = Map::new();
            for k in keys {
                let p = if path.is_empty() { k.clone() } else { format!("{path}.{k}") };
                let child = &m[k];
                let token = rules.opaque.get(&p).or_else(|| rules.opaque_keys.get(k));
                let presigned = child
                    .as_str()
                    .filter(|_| rules.presigned.body_paths.contains(&p))
                    .and_then(|s| presigned_url(rules, s));
                let normalised = if let (Some(token), false) = (token, child.is_null()) {
                    Value::String(token.clone())
                } else if let Some(signed) = presigned {
                    // the ids in the path (the workspace, the random part of the object key) are numbered with the others
                    Value::String(ids.scrub(&signed))
                } else if !keep_times
                    && child
                        .as_str()
                        .is_some_and(|s| key_matches(k, &rules.dt_suffixes, &rules.dt_exact) && is_datetime(s))
                {
                    Value::String(rules.dt_token.clone())
                } else if !keep_times
                    && child.as_str().is_some_and(|s| key_matches(k, &rules.date_suffixes, &[]) && is_date(s))
                {
                    Value::String(rules.date_token.clone())
                } else {
                    value_with(rules, ids, child, &p, keep_times)
                };
                out.insert(k.clone(), normalised);
            }
            Value::Object(out)
        }
        Value::Array(a) => {
            Value::Array(a.iter().map(|x| value_with(rules, ids, x, &format!("{path}[]"), keep_times)).collect())
        }
        Value::String(s) => Value::String(ids.scrub(s)),
        other => other.clone(),
    }
}

/// `^https?://[^/?#]+` replaced by `token`; a value that does not start that way is unchanged.
fn replace_origin(v: &str, token: &str) -> String {
    for scheme in ["http://", "https://"] {
        if let Some(rest) = v.strip_prefix(scheme) {
            let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
            if end > 0 {
                return format!("{token}{}", &rest[end..]);
            }
        }
    }
    v.to_string()
}

/// A presigned object-store URL, normalised; `None` when `s` is not one (the Python half is `presigned_url` in normalise.py).
///
/// A URL is presigned when it has a query that holds the parameter named by the rule's `marker`. Then the scheme and authority are
/// replaced by the rule's origin token, the value of each parameter the rule names is replaced by its token, and the parameters are
/// put in order of name (a stable sort on the text before the first `=`). Names, the other values and their percent-encoding, the
/// path and any fragment are kept. The caller numbers the ids in the result.
fn presigned_url(rules: &Rules, s: &str) -> Option<String> {
    let cfg = &rules.presigned;
    let (base, rest) = s.split_once('?')?;
    let (query, tail) = match rest.find('#') {
        Some(i) => rest.split_at(i),
        None => (rest, ""),
    };
    // the text before the first `=`, or all of it when there is none, as Python's `partition("=")[0]`
    let name_of = |p: &str| p.split_once('=').map_or_else(|| p.to_string(), |(n, _)| n.to_string());
    let params: Vec<&str> = query.split('&').collect();
    if !params.iter().any(|p| name_of(p) == cfg.marker) {
        return None;
    }
    let mut out: Vec<String> = params
        .iter()
        .map(|p| match (cfg.query_tokens.get(&name_of(p)), p.contains('=')) {
            (Some(token), true) => format!("{}={token}", name_of(p)),
            _ => (*p).to_string(),
        })
        .collect();
    out.sort_by_key(|p| name_of(p));
    Some(format!("{}?{}{tail}", replace_origin(base, &cfg.origin_token), out.join("&")))
}

/// The response headers that are contract. A header the presigned rule names that holds a presigned URL is normalised as one, with
/// its ids numbered; any other `location` has its scheme and authority replaced. Visited in order of lower-cased name, as the
/// Python half does, because that is the order ids are numbered in.
fn headers(rules: &Rules, ids: &mut Ids, hs: Option<&Value>) -> Value {
    let mut out = Map::new();
    if let Some(Value::Object(m)) = hs {
        let mut sorted: Vec<(String, &Value)> = m.iter().map(|(k, v)| (k.to_lowercase(), v)).collect();
        sorted.sort_by(|a, b| a.0.cmp(&b.0));
        for (lower, v) in sorted {
            if !rules.headers_keep.contains(&lower) {
                continue;
            }
            let signed =
                v.as_str().filter(|_| rules.presigned.headers.contains(&lower)).and_then(|s| presigned_url(rules, s));
            let value = match (signed, v.as_str()) {
                (Some(signed), _) => Value::String(ids.scrub(&signed)),
                (None, Some(s)) if rules.origin_headers.contains(&lower) => {
                    Value::String(replace_origin(s, &rules.origin_token))
                }
                _ => v.clone(),
            };
            out.insert(lower, value);
        }
    }
    Value::Object(out)
}

/// Normalise one exchange in the fixed traversal order: request path, request body, response headers (only a presigned URL in one
/// has ids to number), response body.
pub fn exchange(rules: &Rules, ids: &mut Ids, ex: &Value) -> Value {
    let (req, resp) = (&ex["request"], &ex["response"]);
    let mut out_req = json!({"method": req["method"], "path": ids.scrub(req["path"].as_str().expect("request.path"))});
    if let Some(who) = req.get("as").filter(|a| !a.is_null()) {
        out_req["as"] = who.clone(); // whose session the request used: part of the contract, never scrubbed
    }
    if let Some(body) = req.get("body").filter(|b| !b.is_null()) {
        out_req["body"] = value_with(rules, ids, body, "", true);
    }
    let out_headers = headers(rules, ids, resp.get("headers"));
    let body = match resp.get("body").filter(|b| !b.is_null()) {
        Some(b) => value(rules, ids, b, ""),
        None => Value::Null,
    };
    json!({
        "name": ex["name"],
        "request": out_req,
        "response": {"status": resp["status"], "headers": out_headers, "body": body},
    })
}
