//! Driven ports. Rules (ADR-0002):
//!
//! * no `worker::*`, `tokio*`, `tokio-postgres`, `sqlx`, `lapin` import in this crate;
//! * **every returned future is `Send`** (plain `#[async_trait]`, not `?Send`). The Cloudflare
//!   adapters wrap their JS-backed futures in `worker::send::SendFuture`, which is sound because
//!   a Worker isolate is single-threaded. Reason: the HTTP layer is one shared axum router, and
//!   axum handlers must be `Send` on *both* targets. See research/0004 section "Port trait shape".

use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use async_trait::async_trait;
use thiserror::Error;
use thrust_contracts::*;

/// Boxed `Send` future used by [`Background`]. No executor is implied by the type.
pub type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send + 'static>>;

// ---------------------------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum DbError {
    /// A uniqueness constraint failed (e.g. live `(project, external_source, external_id)`).
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("database backend error: {0}")]
    Backend(String),
}

#[derive(Debug, Error)]
pub enum QueueError {
    #[error("queue backend error: {0}")]
    Backend(String),
}

#[derive(Debug, Error)]
pub enum BlobError {
    #[error("blob backend error: {0}")]
    Backend(String),
}

#[derive(Debug, Error)]
pub enum RateError {
    #[error("rate-limiter backend error: {0}")]
    Backend(String),
}

// ---------------------------------------------------------------------------------------------
// Ambient capabilities: time, ids, background work
// ---------------------------------------------------------------------------------------------

/// Wall clock. Core cannot call `Utc::now()` (the chrono `clock` feature is off).
pub trait Clock: Send + Sync {
    fn now(&self) -> Timestamp;
}

/// Time-ordered unique ids (UUID v7, ADR-0043 A2).
pub trait IdGen: Send + Sync {
    fn new_uuid(&self) -> Uuid;
}

/// The executor abstraction. Core never calls `tokio::spawn`; it asks this port.
///
/// * native: `tokio::spawn`
/// * Worker: `ctx.wait_until(...)` so the isolate lives until the work finishes
/// * tests: run inline
pub trait Background: Send + Sync {
    fn spawn(&self, fut: BoxFuture<()>);
}

// ---------------------------------------------------------------------------------------------
// Proof types (ADR-0041, ADR-0045, ADR-0051; issue #35)
// ---------------------------------------------------------------------------------------------

/// Proof that `user` may act in `(workspace, project)`. Declared here so that the ports can take
/// it and the adapters can read the tenant key from it; minted only by `thrust_core::authorize`.
/// Fields are private, so a struct literal outside this crate does not compile, and
/// `scripts/check-deps.sh` rejects a call to [`Scope::mint`] anywhere else.
#[derive(Debug, Clone, Copy)]
pub struct Scope {
    workspace_id: WorkspaceId,
    project_id: ProjectId,
    user_id: UserId,
}

impl Scope {
    /// Reserved to `thrust_core::authorize`, after the membership check passes.
    #[doc(hidden)]
    pub fn mint(workspace_id: WorkspaceId, project_id: ProjectId, user_id: UserId) -> Self {
        Self { workspace_id, project_id, user_id }
    }
    pub fn workspace_id(&self) -> WorkspaceId {
        self.workspace_id
    }
    pub fn project_id(&self) -> ProjectId {
        self.project_id
    }
    pub fn user_id(&self) -> UserId {
        self.user_id
    }
}

/// Permission for the untenanted operations: the outbox relay and sweeper, the queue consumer's
/// activity write, and `seed`. Minted only by the composition roots and the consumer entry
/// (ADR-0045 round 29); `scripts/check-deps.sh` rejects a call to [`SystemScope::mint`] elsewhere.
#[derive(Debug)]
pub struct SystemScope {
    _private: (),
}

impl SystemScope {
    /// Reserved to the composition roots and the queue consumer entry.
    #[doc(hidden)]
    pub fn mint() -> Self {
        Self { _private: () }
    }
}

// ---------------------------------------------------------------------------------------------
// Database / unit of work (ADR-0045, shape reduced to what the slice needs)
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Membership {
    /// Nothing the caller may know about: the slug names no workspace, the caller is not a member of
    /// that workspace, or the project is not in it (also: it exists in another tenant). All three
    /// look the same on purpose, so a caller cannot probe for workspaces or projects.
    NoSuchProject,
    /// A member of the workspace, but not of the project.
    NotAMember,
    /// A member of the project. `workspace_id` is what the slug resolved to, for `authorize` to mint
    /// the [`Scope`] with.
    Member { workspace_id: WorkspaceId, role: Role },
}

/// SHA-256 of a session token. The database stores and compares only this, never the token
/// (ADR-0052, ADR-0141); `thrust_core::hash_token` is the one place that makes one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TokenHash(pub [u8; 32]);

/// A session row to insert: sign-in (#38) and the test seed make these.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewSession {
    pub token_hash: TokenHash,
    pub user_id: UserId,
    pub created_at: Timestamp,
    pub expires_at: Timestamp,
}

/// What a live session row says: whose it is, and when it was last used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LiveSession {
    pub user_id: UserId,
    pub last_seen_at: Timestamp,
}

#[async_trait]
pub trait Database: Send + Sync {
    /// The one call that runs before a [`Scope`] exists: only `thrust_core::authorize` makes it,
    /// to decide whether to mint one (ADR-0045 round 29). It resolves the route's workspace slug,
    /// the caller's membership of that workspace and of the project in one query (ADR-0051 round
    /// 29). Every other method takes a [`Scope`], from which the adapter reads the tenant key, or a
    /// [`SystemScope`]; none takes bare ids.
    async fn membership(
        &self,
        slug: &WorkspaceSlug,
        project: ProjectId,
        user: UserId,
    ) -> Result<Membership, DbError>;

    // --- Sessions: a directory table (ADR-0041), so no call here carries a tenant key. Like
    // `membership`, they run before a `Scope` exists, and take no proof type: the token is the
    // credential (ADR-0052 round 29: the row is authoritative and read on every request).

    /// The session behind `hash`, only while it is live: not revoked and `expires_at > now`.
    async fn session(&self, hash: &TokenHash, now: Timestamp) -> Result<Option<LiveSession>, DbError>;

    /// Record that the session was used at `at`. Core calls it lazily (`session_touch_after`).
    async fn touch_session(&self, hash: &TokenHash, at: Timestamp) -> Result<(), DbError>;

    /// Revoke the session at `at`. A session that is unknown or already revoked is left as it is.
    async fn revoke_session(&self, hash: &TokenHash, at: Timestamp) -> Result<(), DbError>;

    /// Insert a session. A hash that already exists is left as it is.
    async fn insert_session(&self, s: &NewSession) -> Result<(), DbError>;

    /// `scope` is the proof that the caller was authorised to open a unit of work.
    async fn begin(&self, scope: &Scope) -> Result<Box<dyn Tx>, DbError>;

    async fn list_work_items(&self, scope: &Scope, vis: Visibility) -> Result<Vec<WorkItem>, DbError>;

    async fn get_work_item(
        &self,
        scope: &Scope,
        id: WorkItemId,
        vis: Visibility,
    ) -> Result<Option<WorkItem>, DbError>;

    /// The visible work items of the project (`Visibility::Visible`) whose name or description contains `query`, in any
    /// case: Plane's `icontains`. In sequence order, at most `limit`. `query` is literal text, so `%`, `_` and `\`
    /// are ordinary characters; it is not empty (the use-case refuses that). Spike 0002, ADR-0083.
    async fn search_work_items(&self, scope: &Scope, query: &str, limit: u32) -> Result<Vec<WorkItem>, DbError>;

    /// Bulk path behind `/internal/seed-work-items` (test and bench support, like `seed`): insert `items` as live work
    /// items of one project, numbered from the project's own counter in one transaction, so the numbers continue after
    /// whatever is there and stay unique and never reused (ADR-0043: a bulk path calls the same allocator). Returns the
    /// sequence number of the first item. A project that is not in `workspace` is refused (`Backend`).
    async fn seed_work_items(
        &self,
        sys: &SystemScope,
        workspace: WorkspaceId,
        project: ProjectId,
        items: &[SeedWorkItem],
    ) -> Result<i64, DbError>;

    /// Idempotent on `event_id`; returns `true` if a row was inserted, `false` on a redelivery.
    /// The queue consumer's write: its tenant is the one the envelope carries (ADR-0141).
    async fn insert_activity(&self, sys: &SystemScope, a: &Activity) -> Result<bool, DbError>;

    async fn list_activity(
        &self,
        scope: &Scope,
        entity: Option<WorkItemId>,
    ) -> Result<Vec<Activity>, DbError>;

    /// Rows with `dispatched_at IS NULL`, oldest first. Used by the cron sweeper.
    async fn undispatched_outbox(&self, sys: &SystemScope, limit: u32) -> Result<Vec<OutboxRow>, DbError>;

    async fn mark_outbox_dispatched(
        &self,
        sys: &SystemScope,
        event: EventId,
        at: Timestamp,
    ) -> Result<(), DbError>;

    async fn outbox_depth(&self, sys: &SystemScope) -> Result<u64, DbError>;

    /// Test support: insert fixture rows (workspaces, workspace members, projects, project members).
    async fn seed(&self, sys: &SystemScope, fixture: &SeedFixture) -> Result<(), DbError>;
}

/// One transaction. Dropped without `commit` == rolled back (adapters must guarantee this;
/// the Postgres adapter documents the pooled-connection caveat). Each write names the [`Scope`]
/// it writes under, and a work item or outbox row of another tenant is refused ([`check_tenant`]).
#[async_trait]
pub trait Tx: Send {
    /// Atomic counter increment + `sort_order` (ADR-0043 B1). Monotonic, never reused.
    async fn allocate(&mut self, scope: &Scope) -> Result<Allocation, DbError>;
    async fn insert_work_item(&mut self, scope: &Scope, item: &WorkItem) -> Result<(), DbError>;
    async fn soft_delete_work_item(
        &mut self,
        scope: &Scope,
        id: WorkItemId,
        at: Timestamp,
    ) -> Result<bool, DbError>;
    async fn insert_outbox(&mut self, scope: &Scope, row: &OutboxRow) -> Result<(), DbError>;
    async fn commit(self: Box<Self>) -> Result<(), DbError>;
    async fn rollback(self: Box<Self>) -> Result<(), DbError>;
}

/// What every adapter checks before a `Tx` write: the row belongs to the scope it is written
/// under (its workspace, and its project when the row has one).
pub fn check_tenant(
    scope: &Scope,
    workspace_id: WorkspaceId,
    project_id: Option<ProjectId>,
) -> Result<(), DbError> {
    if workspace_id != scope.workspace_id() || project_id.is_some_and(|p| p != scope.project_id()) {
        return Err(DbError::Backend("row is outside the scope it is written under".into()));
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Queue (ADR-0070)
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct SendOpts {
    pub delay: Option<Duration>,
    pub idempotency_key: Option<String>,
    pub ordering_key: Option<String>,
}

#[async_trait]
pub trait Queue: Send + Sync {
    async fn send(&self, topic: &str, msg: &Envelope, opts: SendOpts) -> Result<(), QueueError>;
}

/// Driving-side callback invoked by a queue adapter for each delivered message. Core provides the
/// implementation (`ActivityConsumer`); every queue adapter and every consumer entrypoint
/// (Worker `queue` event, native consumer loop) calls the same body.
#[async_trait]
pub trait EventConsumer: Send + Sync {
    async fn handle(&self, env: &Envelope) -> Result<(), String>;
}

// ---------------------------------------------------------------------------------------------
// Blobs (ADR-0080) and rate limiting (ADR-0033)
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Blob {
    pub bytes: Vec<u8>,
    pub content_type: Option<String>,
}

#[async_trait]
pub trait BlobStore: Send + Sync {
    async fn put(&self, key: &str, bytes: Vec<u8>, content_type: Option<String>) -> Result<(), BlobError>;
    async fn get(&self, key: &str) -> Result<Option<Blob>, BlobError>;
    async fn delete(&self, key: &str) -> Result<(), BlobError>;
    async fn list(&self, prefix: &str) -> Result<Vec<String>, BlobError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RateDecision {
    Allowed { remaining: u32 },
    Limited { retry_after_secs: u64 },
}

/// Strongly-consistent fixed-window counter (KV is eventually consistent, so the Cloudflare
/// adapter is a Durable Object; ADR-0082).
#[async_trait]
pub trait RateLimiter: Send + Sync {
    async fn hit(&self, key: &str, limit: u32, window_secs: u64) -> Result<RateDecision, RateError>;
}
