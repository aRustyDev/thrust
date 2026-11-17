//! The world the workload runs against: the v1 routes over a memory world with one workspace, one project with a default state, and
//! an admin with an API key. Deterministic (AGENTS.md rule 6): a fixed clock, ids and randomness from counters, the outbox fast
//! path off and background work dropped. In `tests/` because it mints the system's proof (`SystemScope::mint`), which only a
//! composition root and a crate's own tests may do (scripts/check-deps.sh); the same shape as `crates/api/v1/tests/common`.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use axum::{Extension, Router};
use thrust_adapters_memory::*;
use thrust_api_v1::V1Config;
use thrust_contracts::chrono::DateTime;
use thrust_contracts::*;
use thrust_core::*;
use thrust_ports::*;
use thrust_profiling::{ADMIN, KEY, PROJECT, SLUG, WORKSPACE};

/// The fixed instant every request sees.
const NOW: i64 = 1_760_000_000;

struct FixedClock(Timestamp);
impl Clock for FixedClock {
    fn now(&self) -> Timestamp {
        self.0
    }
}

#[derive(Default)]
struct CounterIds(AtomicU64);
impl IdGen for CounterIds {
    fn new_uuid(&self) -> Uuid {
        Uuid::from_u128((1u128 << 64) | u128::from(self.0.fetch_add(1, Ordering::Relaxed)))
    }
}

/// Distinct bytes from a counter (splitmix64), so two tokens made one after the other differ.
#[derive(Default)]
struct CounterRng(AtomicU64);
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

/// An API key's last use is stamped in the background; the world drops that work, so what is measured is the request's own.
struct DiscardBackground;
impl Background for DiscardBackground {
    fn spawn(&self, _fut: BoxFuture<()>) {}
}

/// Build the world and return the v1 router over it. Panics if the seed is refused, which would be a bug here, not a measurement.
pub fn world() -> Router {
    let db = Arc::new(MemoryDatabase::new());
    let background: Arc<dyn Background> = Arc::new(DiscardBackground);
    let queue: Arc<dyn Queue> = Arc::new(MemoryQueue::new(background.clone(), ConsumerSlot::default()));
    let clock: Arc<dyn Clock> = Arc::new(FixedClock(DateTime::from_timestamp(NOW, 0).expect("a valid instant")));
    let core_cfg = CoreConfig {
        outbox_fastpath: false,
        csrf_enforce: false,
        // a cheap argon2id: the workload measures the routes, not the password hash
        argon2: Argon2Params { m_cost_kib: 32, t_cost: 1, p_cost: 1 },
        ..CoreConfig::default()
    };
    let hasher = Arc::new(Argon2idHasher::new(core_cfg.argon2));
    let app = App::new(
        Ports {
            db,
            queue,
            blobs: Arc::new(MemoryBlobStore::new()),
            limiter: Arc::new(MemoryRateLimiter::new(clock.clone())),
            clock,
            ids: Arc::new(CounterIds::default()),
            background,
            rng: Arc::new(CounterRng::default()),
            hasher,
            oidc: None,
            email: None,
        },
        core_cfg,
        SystemScope::mint(),
    );
    futures_executor::block_on(seed(&app)).expect("the workload's world seeds");
    // an allowance no workload meets: the limiter is measured with the rest, but never refuses
    let cfg = V1Config { rate_limit: 1_000_000, rate_window_secs: 60 };
    thrust_api_v1::router().layer(Extension(app)).layer(Extension(cfg))
}

async fn seed(app: &App) -> Result<(), CoreError> {
    let ws = WorkspaceId(Uuid::from_u128(WORKSPACE));
    let project = ProjectId(Uuid::from_u128(PROJECT));
    let admin = UserId(Uuid::from_u128(ADMIN));
    let fixture = SeedFixture {
        workspaces: vec![SeedWorkspace { id: ws, slug: WorkspaceSlug::new(SLUG) }],
        users: vec![admin],
        workspace_members: vec![SeedWorkspaceMember { workspace_id: ws, user_id: admin }],
        workspace_roles: vec![SeedWorkspaceRole { workspace_id: ws, user_id: admin, role: Role::Admin }],
        projects: vec![SeedProject { id: project, workspace_id: ws, name: "Profiled Project".into() }],
        memberships: vec![SeedMembership { project_id: project, workspace_id: ws, user_id: admin, role: Role::Admin }],
        states: vec![SeedState {
            id: Uuid::from_u128(0x5001),
            project_id: project,
            workspace_id: ws,
            name: "Backlog".into(),
            color: "#000000".into(),
            sequence: 15000.0,
            group: StateGroup::Backlog,
            is_default: true,
        }],
        api_keys: vec![SeedApiKey {
            token: KEY.into(),
            user_id: admin,
            label: KEY.into(),
            expires_at: None,
            revoked: false,
        }],
        ..SeedFixture::default()
    };
    thrust_core::seed(app, &fixture).await
}
