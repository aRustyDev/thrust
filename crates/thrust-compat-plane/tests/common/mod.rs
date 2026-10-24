//! Shared helpers for the compat parity tests: a world built from the memory adapters (the reference implementation of
//! every port), the in-process driver, and the replay of a recorded fixture.
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

/// The compat routes over an empty memory world, with a fixed clock and counting ids.
pub fn app() -> Router {
    let background: Arc<dyn Background> = Arc::new(NoBackground);
    let queue: Arc<dyn Queue> = Arc::new(MemoryQueue::new(background.clone(), ConsumerSlot::default()));
    let clock: Arc<dyn Clock> = Arc::new(FixedClock(DateTime::from_timestamp(1_760_000_000, 0).unwrap()));
    let cfg = CoreConfig {
        outbox_fastpath: false,
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

/// The row ids that have a fixture file.
pub fn fixture_rows() -> Vec<String> {
    let mut rows: Vec<String> = std::fs::read_dir(fixtures_dir())
        .expect("fixtures dir")
        .filter_map(|e| e.ok()?.file_name().to_str()?.strip_suffix(".json").map(str::to_string))
        .collect();
    rows.sort();
    rows
}

/// Send one recorded request to `router` and return what it answered, as a raw exchange (the shape the normaliser takes).
pub fn send(router: &Router, name: &Value, req: &Value) -> Value {
    let method: Method = req["method"].as_str().expect("method").parse().expect("method");
    let mut builder = Request::builder().method(method).uri(req["path"].as_str().expect("path"));
    let body = match req.get("body").filter(|b| !b.is_null()) {
        Some(b) if b.get("form").is_some() => panic!("form bodies are not replayed yet; found one in {name}"),
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
        headers.insert(k.as_str().to_string(), Value::String(v.to_str().unwrap_or("<non-utf8>").to_string()));
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

/// Replay every exchange of `fixture` against `router`, normalise what comes back and compare it with the recording.
/// Returns one message per diverging exchange (empty means parity).
pub fn divergences(router: &Router, fixture: &Value) -> Vec<String> {
    let rules = normalise::Rules::load();
    let mut ids = normalise::Ids::new(&rules);
    let mut out = Vec::new();
    for want in fixture["exchanges"].as_array().expect("exchanges") {
        let raw = send(router, &want["name"], &want["request"]);
        let got = normalise::exchange(&rules, &mut ids, &raw);
        if &got != want {
            let mut paths = Vec::new();
            diff_paths(want, &got, "", &mut paths);
            out.push(format!("exchange {}:\n{}", want["name"], paths.join("\n")));
        }
    }
    out
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
