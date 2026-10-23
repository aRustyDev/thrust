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
pub enum CacheError {
    #[error("cache backend error: {0}")]
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
    /// The project does not exist *in the caller's workspace* (also: it exists in another tenant).
    NoSuchProject,
    NotAMember,
    Member(Role),
}

#[async_trait]
pub trait Database: Send + Sync {
    /// Every method takes the tenant key; an adapter never answers a query without it.
    async fn membership(
        &self,
        ws: WorkspaceId,
        project: ProjectId,
        user: UserId,
    ) -> Result<Membership, DbError>;

    async fn begin(&self) -> Result<Box<dyn Tx>, DbError>;

    async fn list_work_items(
        &self,
        ws: WorkspaceId,
        project: ProjectId,
        vis: Visibility,
    ) -> Result<Vec<WorkItem>, DbError>;

    async fn get_work_item(
        &self,
        ws: WorkspaceId,
        project: ProjectId,
        id: WorkItemId,
        vis: Visibility,
    ) -> Result<Option<WorkItem>, DbError>;

    /// Idempotent on `event_id`; returns `true` if a row was inserted, `false` on a redelivery.
    async fn insert_activity(&self, a: &Activity) -> Result<bool, DbError>;

    async fn list_activity(
        &self,
        ws: WorkspaceId,
        project: ProjectId,
        entity: Option<WorkItemId>,
    ) -> Result<Vec<Activity>, DbError>;

    /// Rows with `dispatched_at IS NULL`, oldest first. Used by the cron sweeper.
    async fn undispatched_outbox(&self, limit: u32) -> Result<Vec<OutboxRow>, DbError>;

    async fn mark_outbox_dispatched(&self, event: EventId, at: Timestamp) -> Result<(), DbError>;

    async fn outbox_depth(&self) -> Result<u64, DbError>;

    /// Test support: insert fixture rows (workspaces/users/projects/memberships).
    async fn seed(&self, fixture: &SeedFixture) -> Result<(), DbError>;
}

/// One transaction. Dropped without `commit` == rolled back (adapters must guarantee this;
/// the Postgres adapter documents the pooled-connection caveat).
#[async_trait]
pub trait Tx: Send {
    /// Atomic counter increment + `sort_order` (ADR-0043 B1). Monotonic, never reused.
    async fn allocate(&mut self, ws: WorkspaceId, project: ProjectId) -> Result<Allocation, DbError>;
    async fn insert_work_item(&mut self, item: &WorkItem) -> Result<(), DbError>;
    async fn soft_delete_work_item(
        &mut self,
        ws: WorkspaceId,
        project: ProjectId,
        id: WorkItemId,
        at: Timestamp,
    ) -> Result<bool, DbError>;
    async fn insert_outbox(&mut self, row: &OutboxRow) -> Result<(), DbError>;
    async fn commit(self: Box<Self>) -> Result<(), DbError>;
    async fn rollback(self: Box<Self>) -> Result<(), DbError>;
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
// Cache / session (ADR-0082) and blobs (ADR-0080) and rate limiting (ADR-0033)
// ---------------------------------------------------------------------------------------------

#[async_trait]
pub trait SessionStore: Send + Sync {
    async fn get(&self, token: &str) -> Result<Option<Principal>, CacheError>;
    async fn put(&self, token: &str, principal: &Principal, ttl_secs: u64) -> Result<(), CacheError>;
}

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
