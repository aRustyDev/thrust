//! A small seeded world for the v1 tests: one workspace, one project, a caller of every role with an API key, and the keys the
//! pin refuses (expired, revoked, a user who is no longer active). Time and ids are injected (AGENTS.md testing rule 6).
#![allow(dead_code, reason = "each test crate compiles this module and uses some of its helpers")]

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use axum::body::Body;
use axum::http::{HeaderMap, Method, Request};
use axum::{Extension, Router};
use serde_json::Value;
use thrust_adapters_memory::*;
use thrust_api_v1::V1Config;
use thrust_contracts::chrono::DateTime;
use thrust_contracts::*;
use thrust_core::*;
use thrust_ports::*;
use tower::ServiceExt;

/// `PROPTEST_CASES` sets the case count; failures are persisted next to the tests, as everywhere else in the workspace.
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

/// Every call returns the same instant, so a rate-limit window never rolls over inside a test.
pub const NOW: i64 = 1_760_000_000;

pub struct FixedClock(pub Timestamp);
impl Clock for FixedClock {
    fn now(&self) -> Timestamp {
        self.0
    }
}

#[derive(Default)]
pub struct CounterIds(AtomicU64);
impl IdGen for CounterIds {
    fn new_uuid(&self) -> Uuid {
        Uuid::from_u128((1u128 << 64) | u128::from(self.0.fetch_add(1, Ordering::Relaxed)))
    }
}

/// A stream of distinct bytes (splitmix64 over a counter), so two tokens made one after the other differ.
#[derive(Default)]
pub struct CounterRng(AtomicU64);
impl Rng for CounterRng {
    fn fill(&self, buf: &mut [u8]) -> Result<(), RngError> {
        for chunk in buf.chunks_mut(8) {
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

/// Authenticating by API key stamps the key's last use, and may re-hash it, in the background (core, `authenticate_api_key`).
/// Neither matters to a v1 route and both are core's to test, so the work is dropped. (The outbox fast path is off in these
/// worlds, so nothing else is ever spawned.)
pub struct DiscardBackground;
impl Background for DiscardBackground {
    fn spawn(&self, _fut: BoxFuture<()>) {}
}

pub const SLUG: &str = "cast";
pub const WS: u128 = 0xA0;
pub const PROJECT: u128 = 0xA1;
pub const STATE_BACKLOG: u128 = 0x5001;
pub const STATE_TODO: u128 = 0x5002;
pub const STATE_TRIAGE: u128 = 0x5003;

/// The people of the world. Each has an API key of the same name (`key-admin`, ...), except `Outsider`, who is in no
/// workspace, and `Inactive`, whose account was switched off after the key was made.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Who {
    Admin,
    Member,
    Guest,
    Outsider,
    Inactive,
}

impl Who {
    pub const ALL: [Who; 5] = [Who::Admin, Who::Member, Who::Guest, Who::Outsider, Who::Inactive];

    pub fn id(self) -> UserId {
        UserId(Uuid::from_u128(match self {
            Who::Admin => 0x10,
            Who::Member => 0x11,
            Who::Guest => 0x12,
            Who::Outsider => 0x13,
            Who::Inactive => 0x14,
        }))
    }

    pub fn key(self) -> &'static str {
        match self {
            Who::Admin => "key-admin",
            Who::Member => "key-member",
            Who::Guest => "key-guest",
            Who::Outsider => "key-outsider",
            Who::Inactive => "key-inactive",
        }
    }

    /// The role in the workspace and the project, if they are in them.
    pub fn role(self) -> Option<Role> {
        match self {
            Who::Admin => Some(Role::Admin),
            Who::Member => Some(Role::Member),
            Who::Guest => Some(Role::Guest),
            Who::Outsider | Who::Inactive => None,
        }
    }
}

pub struct World {
    pub app: App,
    pub router: Router,
}

/// The v1 routes over a seeded memory world, with `routes` mounted beside them and `cfg` as the rate limit.
pub fn world(routes: Router, cfg: V1Config) -> World {
    let app = app();
    let router = thrust_api_v1::assemble(routes).layer(Extension(app.clone())).layer(Extension(cfg));
    World { app, router }
}

/// The crate's own routes (`router()`) over the seeded world, with an allowance big enough that no test meets it.
pub fn real_world() -> World {
    real_world_with(V1Config { rate_limit: 100_000, rate_window_secs: 60 })
}

pub fn real_world_with(cfg: V1Config) -> World {
    let app = app();
    let router = thrust_api_v1::router().layer(Extension(app.clone())).layer(Extension(cfg));
    World { app, router }
}

/// The seeded `App` alone, for a test that mounts the crate's own `router()`.
pub fn app() -> App {
    let db = Arc::new(MemoryDatabase::new());
    let background: Arc<dyn Background> = Arc::new(DiscardBackground);
    let queue: Arc<dyn Queue> = Arc::new(MemoryQueue::new(background.clone(), ConsumerSlot::default()));
    let clock: Arc<dyn Clock> = Arc::new(FixedClock(DateTime::from_timestamp(NOW, 0).expect("a valid instant")));
    let core_cfg = CoreConfig {
        outbox_fastpath: false,
        csrf_enforce: false,
        // a cheap argon2id: the cost is pinned by a core test, not by these
        argon2: Argon2Params { m_cost_kib: 32, t_cost: 1, p_cost: 1 },
        ..CoreConfig::default()
    };
    let hasher = Arc::new(Argon2idHasher::new(core_cfg.argon2));
    let app = App::new(
        Ports {
            db: db.clone(),
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
        core_cfg,
        SystemScope::mint(),
    );
    futures_executor::block_on(seed(&app)).expect("the world seeds");
    app
}

async fn seed(app: &App) -> Result<(), CoreError> {
    let ws = WorkspaceId(Uuid::from_u128(WS));
    let project = ProjectId(Uuid::from_u128(PROJECT));
    let past = DateTime::from_timestamp(NOW - 1_000_000, 0);
    let key = |token: &str, who: Who, expires_at, revoked| SeedApiKey {
        token: token.into(),
        user_id: who.id(),
        label: token.into(),
        expires_at,
        revoked,
    };
    let in_workspace: Vec<Who> = Who::ALL.iter().copied().filter(|w| *w != Who::Outsider).collect();
    let fixture = SeedFixture {
        workspaces: vec![SeedWorkspace { id: ws, slug: WorkspaceSlug::new(SLUG) }],
        users: Who::ALL.iter().map(|w| w.id()).collect(),
        workspace_members: in_workspace
            .iter()
            .map(|w| SeedWorkspaceMember { workspace_id: ws, user_id: w.id() })
            .collect(),
        workspace_roles: Who::ALL
            .iter()
            .filter_map(|w| w.role().map(|role| SeedWorkspaceRole { workspace_id: ws, user_id: w.id(), role }))
            .collect(),
        projects: vec![SeedProject { id: project, workspace_id: ws, name: "Cast Project".into() }],
        memberships: Who::ALL
            .iter()
            .filter_map(|w| {
                w.role().map(|role| SeedMembership { project_id: project, workspace_id: ws, user_id: w.id(), role })
            })
            .collect(),
        // the project's states: a default, one more, and the intake one (which a list of states leaves out)
        states: [
            (STATE_BACKLOG, "Backlog", 15000.0, StateGroup::Backlog, true),
            (STATE_TODO, "Todo", 30000.0, StateGroup::Unstarted, false),
            (STATE_TRIAGE, "Triage", 90000.0, StateGroup::Triage, false),
        ]
        .iter()
        .map(|(n, name, sequence, group, is_default)| SeedState {
            id: Uuid::from_u128(*n),
            project_id: project,
            workspace_id: ws,
            name: (*name).into(),
            color: "#000000".into(),
            sequence: *sequence,
            group: *group,
            is_default: *is_default,
        })
        .collect(),
        api_keys: vec![
            key("key-admin", Who::Admin, None, false),
            key("key-member", Who::Member, None, false),
            key("key-guest", Who::Guest, None, false),
            key("key-outsider", Who::Outsider, None, false),
            key("key-inactive", Who::Inactive, None, false),
            // a second key of the admin, to show that two keys of one user do not share an allowance
            key("key-admin-2", Who::Admin, None, false),
            key("key-expired", Who::Admin, past, false),
            key("key-revoked", Who::Admin, None, true),
        ],
        sessions: vec![SeedSession { token: "session-admin".into(), user_id: Who::Admin.id() }],
        ..SeedFixture::default()
    };
    thrust_core::seed(app, &fixture).await?;
    // the account is switched off after its key was made
    set_user_active(app, Who::Inactive.id(), false).await?;
    Ok(())
}

/// An answer, read.
#[derive(Debug)]
pub struct Reply {
    pub status: u16,
    pub body: Value,
    pub headers: HeaderMap,
}

impl Reply {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).and_then(|v| v.to_str().ok())
    }
    pub fn has_rate_headers(&self) -> bool {
        self.headers.contains_key("x-ratelimit-remaining") || self.headers.contains_key("x-ratelimit-reset")
    }
}

/// One request through the router, in process. `headers` are sent as given.
pub fn send(router: &Router, method: &str, uri: &str, headers: &[(&str, &str)]) -> Reply {
    send_raw(router, method, uri, headers, Vec::new())
}

/// [`send`] with a body, as given.
pub fn send_raw(router: &Router, method: &str, uri: &str, headers: &[(&str, &str)], body: Vec<u8>) -> Reply {
    let mut req = Request::builder().method(Method::from_bytes(method.as_bytes()).expect("a method")).uri(uri);
    for (name, value) in headers {
        req = req.header(*name, *value);
    }
    let req = req.body(Body::from(body)).expect("a request");
    futures_executor::block_on(async {
        let resp = router.clone().oneshot(req).await.expect("the router answers");
        let (parts, body) = resp.into_parts();
        let bytes = axum::body::to_bytes(body, usize::MAX).await.expect("a body");
        let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        Reply { status: parts.status.as_u16(), body, headers: parts.headers }
    })
}

/// A request that carries `key` as the API key.
pub fn send_as(router: &Router, key: &str, method: &str, uri: &str) -> Reply {
    send(router, method, uri, &[("x-api-key", key)])
}

/// A request that carries `key` and a JSON body.
pub fn send_json(router: &Router, key: &str, method: &str, uri: &str, body: &Value) -> Reply {
    let headers = [("x-api-key", key), ("content-type", "application/json")];
    send_raw(router, method, uri, &headers, serde_json::to_vec(body).expect("a body"))
}
