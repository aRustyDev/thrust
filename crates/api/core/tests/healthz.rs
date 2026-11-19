//! `GET /healthz` and `GET /healthz?deep=1` (issue #42, ADR-0094): the router, driven in process with `oneshot`.
//!
//! Plain `/healthz` is 200 `ok` and touches nothing, whatever query string the caller adds (the deploy's smoke sends `?smoke=N` and the
//! edge's controls send `?edge-control=...`, and both need that 200). With `deep=1` it runs one statement through the database: 200 `ok`
//! when that answers, 503 `unavailable` when it does not, and never a word of the cause in the body (the cause goes on the response as
//! `InternalErrorDetail`, which both roots log).
//!
//! The expected answer is written down here as a table of query-string pieces and what each one means. It is not computed with the
//! code under test. The database is one of three states: up (the memory adapters), down (the Postgres adapter over a `Connector` that
//! always refuses: the one `Database` that can fail with no server), and no `App` at all (a root that never installed one).

mod common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use axum::body::Body;
use axum::http::header::CACHE_CONTROL;
use axum::http::{Request, StatusCode};
use axum::{Extension, Router};
use common::proptest_config;
use futures_executor::block_on;
use proptest::prelude::*;
use rstest::rstest;
use thrust_adapters_memory::*;
use thrust_adapters_postgres::{ClientHandle, Connector, PgDatabase};
use thrust_api::InternalErrorDetail;
use thrust_contracts::chrono::DateTime;
use thrust_contracts::*;
use thrust_core::*;
use thrust_ports::*;
use tower::ServiceExt;

// ---------------------------------------------------------------------------------------------
// The spec: what each piece of a query string means.
// ---------------------------------------------------------------------------------------------

/// One `name=value` piece of a query string, and whether it asks for the deep check on its own. A query is deep when ANY of its
/// pieces is: running a check nobody needed costs one statement, and a caller who wrote `deep=1` meant it.
///
/// Read the way axum's `Query` reads a query: percent-decoded, `+` is a space, names are case-sensitive and a value must be
/// exactly `1`.
const PIECES: &[(&str, bool)] = &[
    ("deep=1", true),
    ("deep=%31", true), // the value, percent-encoded
    ("d%65ep=1", true), // the name, percent-encoded
    ("%64%65%65%70=1", true),
    ("deep=0", false),
    ("deep=11", false),
    ("deep=true", false),
    ("deep=1x", false),
    ("deep=+1", false), // `+` is a space: " 1"
    ("deep", false),    // no value
    ("deep=", false),   // an empty value
    ("Deep=1", false),  // names are case-sensitive
    ("DEEP=1", false),
    ("xdeep=1", false),
    ("deep+=1", false), // the name is "deep "
    ("=1", false),      // no name
    ("%ZZ", false),     // not an escape: kept as it is
    ("%", false),
    ("smoke=7", false),                 // what scripts/smoke-deploy.sh sends
    ("edge-control=forged-pin", false), // what scripts/edge-controls.sh sends
];

/// Every query a request can have, up to two pieces: none at all (no `?`), an empty one (a bare `?`), each piece alone and each ordered
/// pair of pieces. The second field is whether the query asks for the deep check.
fn queries() -> Vec<(Option<String>, bool)> {
    let mut all = vec![(None, false), (Some(String::new()), false)];
    for (a, a_deep) in PIECES {
        all.push((Some((*a).to_string()), *a_deep));
        for (b, b_deep) in PIECES {
            all.push((Some(format!("{a}&{b}")), *a_deep || *b_deep));
        }
    }
    all
}

// ---------------------------------------------------------------------------------------------
// The world.
// ---------------------------------------------------------------------------------------------

/// A `Connector` that refuses every checkout, and counts how often it was asked: the database is down, and a request that never needed
/// the database is shown never to have asked for it.
struct Refusing {
    cause: String,
    asked: Arc<AtomicUsize>,
}

#[async_trait]
impl Connector for Refusing {
    async fn checkout(&self, _exclusive: bool) -> Result<Box<dyn ClientHandle>, DbError> {
        self.asked.fetch_add(1, Ordering::Relaxed);
        Err(DbError::Backend(self.cause.clone()))
    }
}

struct FixedClock(Timestamp);
impl Clock for FixedClock {
    fn now(&self) -> Timestamp {
        self.0
    }
}

struct ZeroIds;
impl IdGen for ZeroIds {
    fn new_uuid(&self) -> Uuid {
        Uuid::nil()
    }
}

struct ZeroRng;
// Spelled out: `proptest::prelude::*` brings a different `Rng` into scope beside the port's.
impl thrust_ports::Rng for ZeroRng {
    fn fill(&self, buf: &mut [u8]) -> Result<(), RngError> {
        buf.fill(0);
        Ok(())
    }
}

/// The health check spawns nothing; if it ever did, the test should say so.
struct NoBackground;
impl Background for NoBackground {
    fn spawn(&self, _fut: BoxFuture<()>) {
        panic!("the health check spawned background work");
    }
}

/// An `App` over the memory adapters, with its database replaced by `db`. Nothing here is read by `/healthz` except the database.
fn app_over(db: Arc<dyn Database>) -> App {
    let background: Arc<dyn Background> = Arc::new(NoBackground);
    let clock: Arc<dyn Clock> =
        Arc::new(FixedClock(DateTime::from_timestamp(1_760_000_000, 0).expect("a valid instant")));
    let cfg = CoreConfig {
        outbox_fastpath: false,
        // a cheap argon2id: nothing here hashes
        argon2: Argon2Params { m_cost_kib: 32, t_cost: 1, p_cost: 1 },
        ..CoreConfig::default()
    };
    let hasher = Arc::new(Argon2idHasher::new(cfg.argon2));
    App::new(
        Ports {
            db,
            queue: Arc::new(MemoryQueue::new(background.clone(), ConsumerSlot::default())),
            blobs: Arc::new(MemoryBlobStore::new()),
            limiter: Arc::new(MemoryRateLimiter::new(clock.clone())),
            clock,
            ids: Arc::new(ZeroIds),
            background,
            rng: Arc::new(ZeroRng),
            hasher,
            oidc: None,
            email: None,
        },
        cfg,
        SystemScope::mint(),
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Db {
    /// The memory adapters: nothing between the handler and its state, so nothing that can be unreachable.
    Up,
    /// The Postgres adapter over a connector that refuses.
    Down,
    /// A router that its composition root never gave an `App`.
    NoApp,
}

/// What came back from one request.
struct Answer {
    status: StatusCode,
    cache_control: Option<String>,
    body: String,
    /// What the root would log for this response (`InternalErrorDetail`), if the response carries one.
    detail: Option<String>,
    /// How many times the database's connector was asked for a connection (always 0 unless the database is `Down`).
    checkouts: usize,
}

fn ask(db: Db, cause: &str, query: &Option<String>) -> Answer {
    let asked = Arc::new(AtomicUsize::new(0));
    let router: Router = match db {
        Db::Up => thrust_api::router().layer(Extension(app_over(Arc::new(MemoryDatabase::new())))),
        Db::Down => {
            let connector = Refusing { cause: cause.to_string(), asked: asked.clone() };
            thrust_api::router().layer(Extension(app_over(Arc::new(PgDatabase::new(Arc::new(connector))))))
        }
        Db::NoApp => thrust_api::router(),
    };
    let uri = match query {
        None => "/healthz".to_string(),
        Some(q) => format!("/healthz?{q}"),
    };
    let req = Request::builder().uri(uri).body(Body::empty()).expect("a valid request");
    let resp = block_on(router.oneshot(req)).expect("the router never fails");
    let (parts, body) = resp.into_parts();
    let bytes = block_on(axum::body::to_bytes(body, 1 << 16)).expect("a readable body");
    Answer {
        status: parts.status,
        cache_control: parts.headers.get(CACHE_CONTROL).map(|v| v.to_str().expect("an ASCII header").to_string()),
        body: String::from_utf8(bytes.to_vec()).expect("a UTF-8 body"),
        detail: parts.extensions.get::<InternalErrorDetail>().map(|d| d.0.clone()),
        checkouts: asked.load(Ordering::Relaxed),
    }
}

/// The spec, as assertions: the answer to a query that does (`deep`) or does not ask for the deep check, with the database in state `db`.
fn assert_answer(db: Db, cause: &str, query: &Option<String>, deep: bool) {
    let a = ask(db, cause, query);
    let what = format!("{db:?}, query {query:?}");
    if !deep {
        // Plain: the 200 the smoke and the edge's controls need, from a database in any state, having asked it nothing.
        assert_eq!(a.status, StatusCode::OK, "{what}");
        assert_eq!(a.body, "ok", "{what}");
        assert_eq!(a.cache_control, None, "{what}: plain /healthz has never carried a Cache-Control");
        assert_eq!(a.detail, None, "{what}");
        assert_eq!(a.checkouts, 0, "{what}: plain /healthz must not ask the database for a connection");
        return;
    }
    assert_eq!(a.cache_control.as_deref(), Some("no-store"), "{what}: a cached answer would measure the cache");
    match db {
        Db::Up => {
            assert_eq!(a.status, StatusCode::OK, "{what}");
            assert_eq!(a.body, "ok", "{what}");
            assert_eq!(a.detail, None, "{what}");
        }
        Db::Down | Db::NoApp => {
            assert_eq!(a.status, StatusCode::SERVICE_UNAVAILABLE, "{what}");
            // Exactly this, and not "does not contain the cause": the body is a fixed word and says nothing of why.
            assert_eq!(a.body, "unavailable", "{what}");
            let detail = a.detail.unwrap_or_else(|| panic!("{what}: a 503 carries its cause for the log"));
            let expected = if db == Db::Down { cause } else { "App extension not installed" };
            assert!(detail.starts_with("deep health check: "), "{what}: {detail}");
            assert!(detail.contains(expected), "{what}: {detail}");
        }
    }
    if db == Db::Down {
        assert_eq!(a.checkouts, 1, "{what}: the deep check is exactly one statement on one connection");
    }
}

// ---------------------------------------------------------------------------------------------
// The tests.
// ---------------------------------------------------------------------------------------------

// Exhaustive over the table above: every query of up to two pieces, with the database in each of its states.
#[rstest]
fn deep_healthz_reports_the_database(#[values(Db::Up, Db::Down, Db::NoApp)] db: Db) {
    for (query, deep) in queries() {
        assert_answer(db, "connection refused (test)", &query, deep);
    }
}

// The same property over generated queries (a mix of the table's pieces and pieces that cannot spell `deep`, in any order and as many as
// six) and generated causes, to see that no cause, however it is spelled, reaches the body.
fn arb_pieces() -> impl Strategy<Value = Vec<(String, bool)>> {
    let table = proptest::sample::select(PIECES.to_vec()).prop_map(|(p, d)| (p.to_string(), d));
    // a name without a `d` and without a `%` cannot decode to `deep`
    let noise = ("[a-ce-z0-9_.-]{1,8}", "[a-z0-9]{0,6}").prop_map(|(k, v)| (format!("{k}={v}"), false));
    prop::collection::vec(prop_oneof![table, noise], 0..=6)
}

proptest! {
    #![proptest_config(proptest_config(256))]

    #[test]
    fn prop_deep_healthz_reports_the_database(
        pieces in arb_pieces(),
        db in prop_oneof![Just(Db::Up), Just(Db::Down), Just(Db::NoApp)],
        cause in "[a-zA-Z0-9 _.:-]{1,40}",
    ) {
        let query = if pieces.is_empty() { None } else { Some(pieces.iter().map(|(p, _)| p.as_str()).collect::<Vec<_>>().join("&")) };
        let deep = pieces.iter().any(|(_, d)| *d);
        assert_answer(db, &cause, &query, deep);
    }
}
