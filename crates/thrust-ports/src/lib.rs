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

/// Cryptographically secure random bytes (issue #38): session tokens, API keys, CSRF tokens, the OIDC `state`, `nonce`
/// and PKCE verifier, and password salts. Core cannot reach an operating-system generator, so it asks this port.
///
/// * native: the operating system's generator (`getrandom`)
/// * Worker: `crypto.getRandomValues`
/// * tests: a deterministic stream, so a test can tell two tokens apart and repeat a run
///
/// A source that cannot deliver is an error, never a short or zero-filled buffer: the caller refuses the request.
pub trait Rng: Send + Sync {
    fn fill(&self, buf: &mut [u8]) -> Result<(), RngError>;
}

#[derive(Debug, Error)]
#[error("random source failed: {0}")]
pub struct RngError(pub String);

/// Password hashing (argon2id, ADR-0050). A port so that a test can count how much hash work a request costs: an
/// unknown email must cost the same as a wrong password, and a stopwatch would be a flaky way to show it.
pub trait PasswordHasher: Send + Sync {
    /// The PHC string (`$argon2id$v=19$m=...,t=...,p=...$salt$hash`) for `password` under the configured cost
    /// parameters. `salt` comes from the [`Rng`] port. One call is one unit of hash work.
    fn hash(&self, password: &str, salt: &[u8]) -> Result<String, PasswordError>;

    /// Whether `password` matches the PHC string `phc`, under the cost parameters the string itself carries. A string
    /// that is not a PHC hash does not match. One call is one unit of hash work, whatever the answer.
    fn verify(&self, password: &str, phc: &str) -> bool;
}

#[derive(Debug, Error)]
#[error("password hashing failed: {0}")]
pub struct PasswordError(pub String);

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

/// A user as the identity tables hold it (ADR-0050; directory tables, ADR-0041: no workspace). `password_hash` is a PHC
/// string, never a password. A user created by id only has neither email nor hash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserRecord {
    pub id: UserId,
    /// Normalised (trimmed, lower case) by the core. Unique when present.
    pub email: Option<String>,
    pub password_hash: Option<String>,
    pub is_active: bool,
}

/// A link from an identity provider's subject to a user: `provider` is the issuer, `provider_account_id` the `sub`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewAccount {
    pub provider: String,
    pub provider_account_id: String,
    pub user_id: UserId,
    /// The email the provider reported when the link was made.
    pub email: Option<String>,
    pub created_at: Timestamp,
}

/// An API key row to insert. Only the digest of the token is stored; `prefix` is the part of the token that is safe to
/// show (ADR-0052 K1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewApiKey {
    pub id: ApiKeyId,
    pub user_id: UserId,
    pub token_hash: TokenHash,
    /// Which digest scheme made `token_hash` (Q-0052.3): `thrust_core::api_key_hash` maps a version and a token to a
    /// digest. Positive; version 1 is plain SHA-256. The adapter stores it and never interprets it.
    pub hash_version: i16,
    pub prefix: String,
    pub label: String,
    pub created_at: Timestamp,
    pub expires_at: Option<Timestamp>,
}

/// What a live API key resolves to: its id (for the usage stamp), its owner, when it was last used, and the scheme its
/// digest was made with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LiveApiKey {
    pub id: ApiKeyId,
    pub user_id: UserId,
    pub last_used_at: Option<Timestamp>,
    pub hash_version: i16,
}

/// One pending OIDC login: written when the browser is sent to the provider, taken (and deleted) by the callback. The
/// `state` is stored as its digest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewOidcLogin {
    pub state_hash: TokenHash,
    pub nonce: String,
    pub code_verifier: String,
    pub created_at: Timestamp,
    pub expires_at: Timestamp,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OidcLogin {
    pub nonce: String,
    pub code_verifier: String,
    pub expires_at: Timestamp,
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

    /// The session behind `hash`, only while it is live: not revoked, `expires_at > now`, and its user exists and is
    /// active (a deactivated user's sessions stop working at once, ADR-0052 Tests).
    async fn session(&self, hash: &TokenHash, now: Timestamp) -> Result<Option<LiveSession>, DbError>;

    /// Record that the session was used at `at`. Core calls it lazily (`session_touch_after`).
    async fn touch_session(&self, hash: &TokenHash, at: Timestamp) -> Result<(), DbError>;

    /// Revoke the session at `at`. A session that is unknown or already revoked is left as it is.
    async fn revoke_session(&self, hash: &TokenHash, at: Timestamp) -> Result<(), DbError>;

    /// Insert a session. A hash that already exists is left as it is.
    async fn insert_session(&self, s: &NewSession) -> Result<(), DbError>;

    /// Sign-out everywhere: revoke every session of `user` that is not revoked yet, at `at`. Returns how many it
    /// revoked. Another user's sessions are not touched.
    async fn revoke_user_sessions(&self, user: UserId, at: Timestamp) -> Result<u64, DbError>;

    // --- Users, accounts, API keys, pending OIDC logins (issue #38; migration 0005). Directory tables like
    // `sessions`: no call carries a tenant key, and none takes a proof type, because they run before there is a
    // caller to authorise. Nothing here ever receives or returns a password or a token, only hashes.

    /// The user with this (already normalised) email, active or not: the caller decides what an inactive user may do.
    async fn user_by_email(&self, email: &str) -> Result<Option<UserRecord>, DbError>;

    /// Create a user (sign-up, issue #44). A taken email is `DbError::Conflict`, raised by the UNIQUE index, so two
    /// sign-ups racing for one email cannot both win. An id that exists is the caller's bug, and also a `Conflict`.
    async fn insert_user(&self, user: &UserRecord) -> Result<(), DbError>;

    /// The user an identity provider's subject is linked to, active or not.
    async fn user_for_account(
        &self,
        provider: &str,
        provider_account_id: &str,
    ) -> Result<Option<UserRecord>, DbError>;

    /// Link a provider subject to a user. A link that exists is left as it is.
    async fn link_account(&self, a: &NewAccount) -> Result<(), DbError>;

    /// Insert an API key. A `Conflict` for a digest that exists; a backend error for a `hash_version` that is not
    /// positive (the column's check).
    async fn insert_api_key(&self, k: &NewApiKey) -> Result<(), DbError>;

    /// The key behind `hash`, only while it is live: not revoked, not expired (`expires_at > now`, or none) and its
    /// owner exists and is active. The digest alone is the key; the row says which scheme made it
    /// ([`LiveApiKey::hash_version`]) and core checks that it is the one it hashed with.
    async fn api_key(&self, hash: &TokenHash, now: Timestamp) -> Result<Option<LiveApiKey>, DbError>;

    /// Stamp the key's last use. Core calls it through [`Background`], so a request does not wait for the write.
    async fn touch_api_key(&self, id: ApiKeyId, at: Timestamp) -> Result<(), DbError>;

    /// Move key `id` to another digest scheme (Q-0052.3): store `new_hash` and `new_version`, but only while the row
    /// is still stored under `from_version` (a compare-and-set, so a re-hash that lost a race, or that was decided on
    /// a row that has since moved, is a no-op). `true` when the row changed, `false` when it did not (an unknown id, or
    /// a row no longer at `from_version`). Nothing else about the key changes: its owner, expiry, revocation and last
    /// use stay. A `new_hash` that another row already holds is a `Conflict`. Core calls it through [`Background`]
    /// after a successful use and ignores a failure.
    async fn rehash_api_key(
        &self,
        id: ApiKeyId,
        from_version: i16,
        new_hash: &TokenHash,
        new_version: i16,
    ) -> Result<bool, DbError>;

    /// `user`'s keys, oldest first, revoked ones included.
    async fn list_api_keys(&self, user: UserId) -> Result<Vec<ApiKeyInfo>, DbError>;

    /// Revoke `user`'s key `id` at `at`. `true` when `user` owns such a key (also when it was already revoked: the
    /// first revocation time is kept), `false` for an unknown id or another user's key.
    async fn revoke_api_key(&self, user: UserId, id: ApiKeyId, at: Timestamp) -> Result<bool, DbError>;

    /// Store a pending login, and drop the ones that have expired by `l.created_at`.
    async fn insert_oidc_login(&self, l: &NewOidcLogin) -> Result<(), DbError>;

    /// Take a pending login: it is returned once and deleted by the same call, so a replayed callback finds nothing.
    /// Expiry is the caller's check (`expires_at` is returned).
    async fn take_oidc_login(&self, state: &TokenHash) -> Result<Option<OidcLogin>, DbError>;

    /// Test support and deactivation until ADR-0053 provides the real one: set `is_active`. `true` if the user exists.
    async fn set_user_active(&self, sys: &SystemScope, user: UserId, active: bool) -> Result<bool, DbError>;

    /// Test support: upsert users with their email, password hash and state. The fixture's id-only users are created
    /// by [`Database::seed`].
    async fn seed_users(&self, sys: &SystemScope, users: &[UserRecord]) -> Result<(), DbError>;

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

// ---------------------------------------------------------------------------------------------
// Outbound HTTP and the OIDC relying party (ADR-0050; issue #38)
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchRequest {
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

#[derive(Debug, Error)]
#[error("outbound request failed: {0}")]
pub struct FetchError(pub String);

/// Outbound HTTP, the only way the application reaches a server it does not own (so far: an OIDC provider's discovery,
/// token and key endpoints). Never follows a redirect on its own: a redirect is a response like any other.
///
/// * native: `reqwest`
/// * Worker: `worker::Fetch`
#[async_trait]
pub trait Fetch: Send + Sync {
    async fn send(&self, req: FetchRequest) -> Result<FetchResponse, FetchError>;
}

/// What a provider vouched for in a verified ID token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OidcClaims {
    pub issuer: String,
    pub subject: String,
    pub email: Option<String>,
    /// The `email_verified` claim; absent counts as `false`.
    pub email_verified: bool,
}

#[derive(Debug, Error)]
pub enum OidcError {
    /// The provider could not be reached or answered something unusable (discovery, token endpoint, keys).
    #[error("identity provider unavailable: {0}")]
    Provider(String),
    /// The provider answered, and what it answered does not authenticate: an error from the token endpoint (a wrong
    /// code or PKCE verifier), or an ID token that fails verification (signature, issuer, audience, expiry, nonce).
    #[error("identity provider response rejected: {0}")]
    Rejected(String),
}

/// One OIDC provider, as a relying party (authorization code with PKCE, discovery, ID-token verification). The
/// `state`, `nonce` and PKCE verifier are the caller's: core makes them with the [`Rng`] port, stores them, and hands
/// them back, so the adapter holds no per-login state and a Worker isolate that dies between the two legs loses nothing.
#[async_trait]
pub trait OidcProvider: Send + Sync {
    /// The authorization endpoint's URL for this login, with the S256 challenge of `code_verifier`.
    async fn authorization_url(&self, state: &str, nonce: &str, code_verifier: &str) -> Result<String, OidcError>;

    /// Exchange the authorization `code` (proving possession of `code_verifier`) and verify the ID token against
    /// `nonce`. Returns only what verification vouches for.
    async fn exchange(&self, code: &str, code_verifier: &str, nonce: &str) -> Result<OidcClaims, OidcError>;
}
