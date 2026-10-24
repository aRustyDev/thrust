//! Shared helpers for the compat parity tests: a world built from the memory adapters (the reference implementation of
//! every port), the in-process driver, the projection of what the pin answered onto what Thrust answers, and the replay
//! of a recorded journey.
#![allow(dead_code)]

pub mod normalise;

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Method, Request};
use axum::{Extension, Router};
use serde_json::{json, Map, Value};
use thrust_adapters_memory::*;
use thrust_contracts::chrono::DateTime;
use thrust_contracts::*;
use thrust_core::*;
use thrust_ports::*;
use tower::ServiceExt;

/// Proptest config for integration tests (from `.agents/skills/rust-test-logic/references/examples/`, as in thrust-core).
/// * `PROPTEST_CASES` raises or lowers the case count without editing tests.
/// * Counterexamples are saved beside the test file (`tests/<file>.proptest-regressions`).
/// * `PROPTEST_DISABLE_FAILURE_PERSISTENCE=1` turns saving off (use it for mutation runs).
pub fn proptest_config(default_cases: u32) -> proptest::test_runner::Config {
    use proptest::test_runner::{Config, FailurePersistence, FileFailurePersistence};
    let cases = std::env::var("PROPTEST_CASES").ok().and_then(|v| v.parse().ok()).unwrap_or(default_cases);
    let failure_persistence: Option<Box<dyn FailurePersistence>> =
        if std::env::var_os("PROPTEST_DISABLE_FAILURE_PERSISTENCE").is_some() {
            None
        } else {
            Some(Box::new(FileFailurePersistence::WithSource("proptest-regressions")))
        };
    Config { cases, failure_persistence, ..Config::default() }
}

/// Time is injected (AGENTS.md testing rule 6): every call returns the same instant.
pub struct FixedClock(pub Timestamp);
impl Clock for FixedClock {
    fn now(&self) -> Timestamp {
        self.0
    }
}

/// Deterministic, unique ids.
#[derive(Default)]
pub struct CounterIds(AtomicU64);
impl IdGen for CounterIds {
    fn new_uuid(&self) -> Uuid {
        Uuid::from_u128((1u128 << 64) | u128::from(self.0.fetch_add(1, Ordering::Relaxed)))
    }
}

/// A deterministic stream of distinct bytes: every call returns different bytes, so two tokens made one after the other
/// differ and a run can be repeated.
#[derive(Default)]
pub struct CounterRng(AtomicU64);
impl Rng for CounterRng {
    fn fill(&self, buf: &mut [u8]) -> Result<(), RngError> {
        for chunk in buf.chunks_mut(8) {
            // splitmix64 over a counter
            let mut z = self.0.fetch_add(0x9E37_79B9_7F4A_7C15, Ordering::Relaxed).wrapping_add(0x9E37_79B9_7F4A_7C15);
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^= z >> 31;
            let bytes = z.to_le_bytes();
            chunk.copy_from_slice(&bytes[..chunk.len()]);
        }
        Ok(())
    }
}

/// The default world runs with the outbox fast path off, so nothing should ever be spawned. Fail loudly if it is.
pub struct NoBackground;
impl Background for NoBackground {
    fn spawn(&self, _fut: BoxFuture<()>) {
        panic!("a compat parity test spawned background work; keep CoreConfig::outbox_fastpath = false");
    }
}

/// The compat routes over an empty memory world, with a fixed clock and counting ids. Configured as the composition roots
/// are (CSRF not enforced while the stock frontend exists, Q-0050.8).
pub fn app() -> Router {
    let background: Arc<dyn Background> = Arc::new(NoBackground);
    let queue: Arc<dyn Queue> = Arc::new(MemoryQueue::new(background.clone(), ConsumerSlot::default()));
    let clock: Arc<dyn Clock> = Arc::new(FixedClock(DateTime::from_timestamp(1_760_000_000, 0).unwrap()));
    let cfg = CoreConfig {
        outbox_fastpath: false,
        csrf_enforce: false,
        // a cheap argon2id: the cost is pinned by a core test, not by these
        argon2: Argon2Params { m_cost_kib: 32, t_cost: 1, p_cost: 1 },
        ..CoreConfig::default()
    };
    let hasher = Arc::new(Argon2idHasher::new(cfg.argon2));
    let app = App::new(
        Ports {
            db: Arc::new(MemoryDatabase::new()),
            queue,
            blobs: Arc::new(MemoryBlobStore::new()),
            limiter: Arc::new(MemoryRateLimiter::new(clock.clone())),
            clock,
            ids: Arc::new(CounterIds::default()),
            background,
            rng: Arc::new(CounterRng::default()),
            hasher,
            oidc: None,
        },
        cfg,
        SystemScope::mint(),
    );
    thrust_compat_plane::routes().layer(Extension(app))
}

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/parity/fixtures")
}

/// A committed fixture, recorded from the pinned Plane by tools/compat-record/record.py.
pub fn load_fixture(row: &str) -> Value {
    let path = fixtures_dir().join(format!("{row}.json"));
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// The row ids that have a fixture file, sorted.
pub fn fixture_rows() -> Vec<String> {
    let mut rows: Vec<String> = std::fs::read_dir(fixtures_dir())
        .expect("fixtures dir")
        .filter_map(|e| e.ok()?.file_name().to_str()?.strip_suffix(".json").map(str::to_string))
        .collect();
    rows.sort();
    rows
}

// ---------------------------------------------------------------------------------------------------------------------
// The projection. The pin answers its sign-in and sign-up form posts with a 302; Thrust's copy of the auth client posts
// JSON and reads JSON (D-022). The fixtures keep the pin's raw answer, so the recording is the pin's and nothing is edited;
// the comparison projects a recorded exchange onto the Thrust shape, by these rules and no others:
//   request  `{"form": {..}}`  becomes the JSON object `{..}`;
//   response 302 whose Location carries `error_code`
//                              becomes 400 (429 for 5900) with the query's pairs as a JSON object, `error_code` a
//                              number, `next_path` left out (it is the client's own);
//   response 302 otherwise     becomes 200 `{"redirect_path": <Location minus the origin, "/" when empty>}`.
// ---------------------------------------------------------------------------------------------------------------------

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < b.len() && b[i + 1].is_ascii_hexdigit() && b[i + 2].is_ascii_hexdigit() => {
                out.push(u8::from_str_radix(&s[i + 1..i + 3], 16).expect("hex"));
                i += 3;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn query_pairs(q: &str) -> Vec<(String, String)> {
    q.split('&')
        .filter(|p| !p.is_empty())
        .map(|p| {
            let (k, v) = p.split_once('=').unwrap_or((p, ""));
            (percent_decode(k), percent_decode(v))
        })
        .collect()
}

/// A recorded exchange as Thrust is expected to answer it. Anything that is not a form request or a 302 is unchanged.
pub fn project(ex: &Value) -> Value {
    let mut ex = ex.clone();
    if let Some(form) = ex["request"]["body"].get("form").cloned() {
        ex["request"]["body"] = form;
    }
    if ex["response"]["status"] == 302 {
        let loc = ex["response"]["headers"]["location"].as_str().expect("a 302 with a location").to_string();
        let rest = loc.strip_prefix("<base_url>").unwrap_or_else(|| panic!("location {loc:?} must start with <base_url>"));
        let query = rest.split_once('?').map_or("", |(_, q)| q);
        let pairs = query_pairs(query);
        let (status, body) = match pairs.iter().find(|(k, _)| k == "error_code") {
            Some((_, code)) => {
                let n: u64 = code.parse().unwrap_or_else(|_| panic!("error_code {code:?}"));
                let mut m = Map::new();
                for (k, v) in &pairs {
                    match k.as_str() {
                        "next_path" => {}
                        "error_code" => {
                            m.insert(k.clone(), json!(n));
                        }
                        _ => {
                            m.insert(k.clone(), json!(v));
                        }
                    }
                }
                (if n == 5900 { 429 } else { 400 }, Value::Object(m))
            }
            None => (200, json!({"redirect_path": if rest.is_empty() { "/" } else { rest }})),
        };
        ex["response"] = json!({"status": status, "headers": {"content-type": "application/json"}, "body": body});
    }
    ex
}

// ---------------------------------------------------------------------------------------------------------------------
// Driving the router
// ---------------------------------------------------------------------------------------------------------------------

/// Send one request to `router` and return what it answered, as a raw exchange (the shape the normaliser takes). A header
/// that came more than once (`set-cookie`) is an array.
pub fn send(router: &Router, name: &Value, req: &Value) -> Value {
    let method: Method = req["method"].as_str().expect("method").parse().expect("method");
    let mut builder = Request::builder().method(method).uri(req["path"].as_str().expect("path"));
    let body = match req.get("body").filter(|b| !b.is_null()) {
        Some(b) if b.get("form").is_some() => panic!("a form body must be projected before it is sent; found one in {name}"),
        Some(b) => {
            builder = builder.header("content-type", "application/json");
            Body::from(b.to_string())
        }
        None => Body::empty(),
    };
    let resp = futures_executor::block_on(router.clone().oneshot(builder.body(body).expect("request"))).expect("infallible");
    let status = resp.status().as_u16();
    let mut headers = Map::new();
    for (k, v) in resp.headers() {
        let v = Value::String(v.to_str().unwrap_or("<non-utf8>").to_string());
        match headers.get_mut(k.as_str()) {
            None => {
                headers.insert(k.as_str().to_string(), v);
            }
            Some(Value::Array(a)) => a.push(v),
            Some(first) => *first = Value::Array(vec![first.clone(), v]),
        }
    }
    let bytes = futures_executor::block_on(axum::body::to_bytes(resp.into_body(), usize::MAX)).expect("body");
    let parsed = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned()))
    };
    json!({
        "name": name,
        "request": req,
        "response": {"status": status, "headers": headers, "body": parsed},
    })
}

/// Whether a raw response sets a live session cookie (not one that clears it).
fn sets_session_cookie(raw: &Value) -> bool {
    let h = &raw["response"]["headers"]["set-cookie"];
    let all: Vec<&str> = match h {
        Value::String(s) => vec![s.as_str()],
        Value::Array(a) => a.iter().filter_map(Value::as_str).collect(),
        _ => vec![],
    };
    all.iter().any(|c| c.starts_with("__Host-thrust_session=") && !c.starts_with("__Host-thrust_session=;") && !c.contains("Max-Age=0"))
}

/// Replay every exchange of `fixture` against `router`, normalise what comes back and compare it with the recording as
/// projected onto Thrust's shape. Returns one message per diverging exchange (empty means parity). A sign-in or sign-up
/// that the recording says worked must also set the session cookie, and one that failed must not: cookies are transport
/// the normaliser drops, so they are checked here.
pub fn divergences(router: &Router, fixture: &Value) -> Vec<String> {
    let rules = normalise::Rules::load();
    let mut ids = normalise::Ids::new(&rules);
    let mut out = Vec::new();
    for recorded in fixture["exchanges"].as_array().expect("exchanges") {
        let want = project(recorded);
        let raw = send(router, &want["name"], &want["request"]);
        let got = normalise::exchange(&rules, &mut ids, &raw);
        let mut lines = Vec::new();
        if got != want {
            diff_paths(&want, &got, "", &mut lines);
        }
        let signs_in = matches!(want["request"]["path"].as_str(), Some("/auth/sign-in/" | "/auth/sign-up/"));
        if signs_in {
            let expected = want["response"]["status"] == 200;
            if sets_session_cookie(&raw) != expected {
                lines.push(format!("  session cookie: the recording says it {} be set, the answer {} it", if expected { "must" } else { "must not" }, if expected { "did not set" } else { "set" }));
            }
        }
        if !lines.is_empty() {
            out.push(format!("exchange {}:\n{}", want["name"], lines.join("\n")));
        }
    }
    out
}

/// Replay the journey up to `target` on one world, so `target`'s requests find the state the recording had (a user made
/// by an earlier step), then replay `target` itself, after `edit` has had its way with the recording. Only `target`'s
/// divergences are returned: each earlier step has a test of its own.
pub fn journey(rows: &[&str], target: &str, edit: impl FnOnce(&mut Value)) -> Vec<String> {
    let router = app();
    for row in rows.iter().take_while(|r| **r != target) {
        let _ = divergences(&router, &load_fixture(row));
    }
    let mut fixture = load_fixture(target);
    edit(&mut fixture);
    divergences(&router, &fixture)
}

/// The JSON paths at which `recorded` and `answered` differ, one line each.
fn diff_paths(recorded: &Value, answered: &Value, path: &str, out: &mut Vec<String>) {
    match (recorded, answered) {
        (Value::Object(r), Value::Object(a)) => {
            let mut keys: Vec<&String> = r.keys().chain(a.keys()).collect();
            keys.sort();
            keys.dedup();
            for k in keys {
                let p = if path.is_empty() { k.clone() } else { format!("{path}.{k}") };
                match (r.get(k), a.get(k)) {
                    (Some(rv), Some(av)) => diff_paths(rv, av, &p, out),
                    (Some(rv), None) => out.push(format!("  {p}: recorded {rv}, answer has no such field")),
                    (None, Some(av)) => out.push(format!("  {p}: answer has {av}, the recording has no such field")),
                    (None, None) => unreachable!(),
                }
            }
        }
        (Value::Array(r), Value::Array(a)) if r.len() == a.len() => {
            for (i, (rv, av)) in r.iter().zip(a).enumerate() {
                diff_paths(rv, av, &format!("{path}[{i}]"), out);
            }
        }
        _ if recorded != answered => out.push(format!("  {path}: recorded {recorded}, answered {answered}")),
        _ => {}
    }
}
