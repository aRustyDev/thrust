//! Shared helpers for the `thrust-core` integration tests: proptest configuration and a fixed world built from
//! the memory adapters (the reference implementation of every port).
#![allow(dead_code)]

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use proptest::test_runner::{Config, FailurePersistence, FileFailurePersistence};
use thrust_adapters_memory::*;
use thrust_contracts::chrono::DateTime;
use thrust_contracts::*;
use thrust_core::*;
use thrust_ports::*;

/// Proptest config for integration tests (from `.agents/skills/rust-test-logic/references/examples/`).
/// * `PROPTEST_CASES` raises or lowers the case count without editing tests.
/// * Counterexamples are saved beside the test file (`tests/<file>.proptest-regressions`).
/// * `PROPTEST_DISABLE_FAILURE_PERSISTENCE=1` turns saving off (use it for mutation runs).
pub fn proptest_config(default_cases: u32) -> Config {
    let cases = std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default_cases);
    let failure_persistence: Option<Box<dyn FailurePersistence>> =
        if std::env::var_os("PROPTEST_DISABLE_FAILURE_PERSISTENCE").is_some() {
            None
        } else {
            Some(Box::new(FileFailurePersistence::WithSource("proptest-regressions")))
        };
    Config { cases, failure_persistence, ..Config::default() }
}

/// Time is injected (Rule 6): every call returns the same instant.
pub struct FixedClock(pub Timestamp);
impl Clock for FixedClock {
    fn now(&self) -> Timestamp {
        self.0
    }
}

/// Deterministic, unique ids. The high bits keep them clear of the fixture ids below.
pub struct CounterIds(AtomicU64);
impl IdGen for CounterIds {
    fn new_uuid(&self) -> Uuid {
        Uuid::from_u128((1u128 << 64) | u128::from(self.0.fetch_add(1, Ordering::Relaxed)))
    }
}

/// The default world runs with the outbox fast path off, so nothing should ever be spawned. Fail loudly if it is.
pub struct NoBackground;
impl Background for NoBackground {
    fn spawn(&self, _fut: BoxFuture<()>) {
        panic!("a core test spawned background work; keep CoreConfig::outbox_fastpath = false");
    }
}

/// Collects spawned work so a test can run it after the use-case returns, the way "after commit" work runs in
/// production. (Running it inside `spawn` would nest `block_on`, which panics.)
#[derive(Default)]
pub struct DeferredBackground(std::sync::Mutex<Vec<BoxFuture<()>>>);
impl DeferredBackground {
    /// Runs everything spawned so far; returns how many tasks ran.
    pub fn run_all(&self) -> usize {
        let tasks: Vec<_> = std::mem::take(&mut *self.0.lock().unwrap());
        let n = tasks.len();
        for t in tasks {
            futures_executor::block_on(t);
        }
        n
    }
}
impl Background for DeferredBackground {
    fn spawn(&self, fut: BoxFuture<()>) {
        self.0.lock().unwrap().push(fut);
    }
}

/// A queue that records every send with its options, and can be told to fail.
#[derive(Default)]
pub struct RecordingQueue {
    pub sent: std::sync::Mutex<Vec<(String, Envelope, SendOpts)>>,
    pub fail: std::sync::atomic::AtomicBool,
}
#[async_trait::async_trait]
impl Queue for RecordingQueue {
    async fn send(&self, topic: &str, msg: &Envelope, opts: SendOpts) -> Result<(), QueueError> {
        if self.fail.load(Ordering::Relaxed) {
            return Err(QueueError::Backend("down (test)".into()));
        }
        self.sent.lock().unwrap().push((topic.to_string(), msg.clone(), opts));
        Ok(())
    }
}
impl RecordingQueue {
    pub fn sent_len(&self) -> usize {
        self.sent.lock().unwrap().len()
    }
}

/// Who is acting. Each one is a fixed user with a fixed relationship to the two projects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Actor {
    AdminA,
    MemberA,
    GuestA,
    /// In workspace A, no membership of project A.
    OutsiderA,
    /// Member of project B in workspace B; a stranger to everything in workspace A.
    MemberB,
    /// A member of both workspaces: Member of project A, Guest of project B. One session, two workspaces.
    Dual,
}

pub const ACTORS: [Actor; 6] =
    [Actor::AdminA, Actor::MemberA, Actor::GuestA, Actor::OutsiderA, Actor::MemberB, Actor::Dual];

/// Which workspace slug a request names, relative to the project it addresses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Slug {
    /// The slug of the project's own workspace.
    Own,
    /// The slug of the other workspace.
    Other,
    /// A slug that names no workspace.
    Unknown,
}

pub const SLUGS: [Slug; 3] = [Slug::Own, Slug::Other, Slug::Unknown];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Proj {
    A,
    B,
}

pub struct World {
    pub app: App,
    pub ws_a: WorkspaceId,
    pub ws_b: WorkspaceId,
    pub slug_a: WorkspaceSlug,
    pub slug_b: WorkspaceSlug,
    pub project_a: ProjectId,
    pub project_b: ProjectId,
}

fn id(n: u128) -> Uuid {
    Uuid::from_u128(n)
}

impl World {
    pub fn principal(&self, a: Actor) -> Principal {
        let user = match a {
            Actor::AdminA => 1,
            Actor::MemberA => 2,
            Actor::GuestA => 3,
            Actor::OutsiderA => 4,
            Actor::MemberB => 5,
            Actor::Dual => 6,
        };
        Principal { user_id: UserId(id(user)) }
    }

    /// The slug a request addressed to `proj` names.
    pub fn slug(&self, proj: Proj, slug: Slug) -> WorkspaceSlug {
        let (own, other) = match proj {
            Proj::A => (&self.slug_a, &self.slug_b),
            Proj::B => (&self.slug_b, &self.slug_a),
        };
        match slug {
            Slug::Own => own.clone(),
            Slug::Other => other.clone(),
            Slug::Unknown => WorkspaceSlug::new("no-such-workspace"),
        }
    }

    pub fn project(&self, p: Proj) -> ProjectId {
        match p {
            Proj::A => self.project_a,
            Proj::B => self.project_b,
        }
    }

    pub fn workspace_of(&self, p: Proj) -> WorkspaceId {
        match p {
            Proj::A => self.ws_a,
            Proj::B => self.ws_b,
        }
    }
}

/// Two tenants, one project each, and the five actors above; memory queue, fast path off.
pub fn world() -> World {
    let background: Arc<dyn Background> = Arc::new(NoBackground);
    let queue: Arc<dyn Queue> = Arc::new(MemoryQueue::new(background.clone(), ConsumerSlot::default()));
    world_with(CoreConfig { outbox_fastpath: false, ..CoreConfig::default() }, queue, background)
}

/// The same world with a chosen config, queue and background (keep your own `Arc` to inspect them).
pub fn world_with(cfg: CoreConfig, queue: Arc<dyn Queue>, background: Arc<dyn Background>) -> World {
    let clock: Arc<dyn Clock> = Arc::new(FixedClock(DateTime::from_timestamp(1_760_000_000, 0).unwrap()));
    let db = MemoryDatabase::new();
    let app = App::new(
        Ports {
            db: Arc::new(db.clone()),
            queue,
            blobs: Arc::new(MemoryBlobStore::new()),
            limiter: Arc::new(MemoryRateLimiter::new(clock.clone())),
            clock,
            ids: Arc::new(CounterIds(AtomicU64::new(0))),
            background,
        },
        cfg,
        SystemScope::mint(),
    );
    let w = World {
        app,
        ws_a: WorkspaceId(id(0xA0)),
        ws_b: WorkspaceId(id(0xB0)),
        slug_a: WorkspaceSlug::new("ws-a"),
        slug_b: WorkspaceSlug::new("ws-b"),
        project_a: ProjectId(id(0xA1)),
        project_b: ProjectId(id(0xB1)),
    };
    let member = |a: Actor, p: Proj, role: Role| SeedMembership {
        project_id: w.project(p),
        workspace_id: w.workspace_of(p),
        user_id: w.principal(a).user_id,
        role,
    };
    let in_ws = |a: Actor, ws: WorkspaceId| SeedWorkspaceMember { workspace_id: ws, user_id: w.principal(a).user_id };
    let fixture = SeedFixture {
        workspaces: vec![
            SeedWorkspace { id: w.ws_a, slug: w.slug_a.clone() },
            SeedWorkspace { id: w.ws_b, slug: w.slug_b.clone() },
        ],
        users: ACTORS.iter().map(|a| w.principal(*a).user_id).collect(),
        workspace_members: vec![
            in_ws(Actor::AdminA, w.ws_a),
            in_ws(Actor::MemberA, w.ws_a),
            in_ws(Actor::GuestA, w.ws_a),
            in_ws(Actor::OutsiderA, w.ws_a),
            in_ws(Actor::MemberB, w.ws_b),
            in_ws(Actor::Dual, w.ws_a),
            in_ws(Actor::Dual, w.ws_b),
        ],
        projects: vec![
            SeedProject { id: w.project_a, workspace_id: w.ws_a, name: "A".into() },
            SeedProject { id: w.project_b, workspace_id: w.ws_b, name: "B".into() },
        ],
        memberships: vec![
            member(Actor::AdminA, Proj::A, Role::Admin),
            member(Actor::MemberA, Proj::A, Role::Member),
            member(Actor::GuestA, Proj::A, Role::Guest),
            member(Actor::MemberB, Proj::B, Role::Member),
            member(Actor::Dual, Proj::A, Role::Member),
            member(Actor::Dual, Proj::B, Role::Guest),
        ],
        sessions: vec![],
    };
    futures_executor::block_on(db.seed(&SystemScope::mint(), &fixture)).unwrap();
    w
}

/// `CoreError` has no `PartialEq`; tests compare this instead.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Ok,
    NotFound,
    Forbidden,
    Invalid,
    TooLarge,
    Other(String),
}

pub fn outcome<T>(r: &Result<T, CoreError>) -> Outcome {
    match r {
        Ok(_) => Outcome::Ok,
        Err(CoreError::NotFound) => Outcome::NotFound,
        Err(CoreError::Forbidden) => Outcome::Forbidden,
        Err(CoreError::Invalid(_)) => Outcome::Invalid,
        Err(CoreError::TooLarge) => Outcome::TooLarge,
        Err(e) => Outcome::Other(e.to_string()),
    }
}
