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

/// Proof that `user` is an active member of `workspace`, with the role they hold there (issue #44). The workspace-level
/// sibling of [`Scope`]: a [`Scope`] names a project too, and the reads that belong to the workspace itself (its members,
/// the caller's own membership, the caller's roles in its projects) need no project. Fields are private, and
/// `scripts/check-deps.sh` rejects a call to [`WorkspaceScope::mint`] anywhere but `thrust_core::authorize_workspace`.
#[derive(Debug, Clone, Copy)]
pub struct WorkspaceScope {
    workspace_id: WorkspaceId,
    user_id: UserId,
    role: Role,
}

impl WorkspaceScope {
    /// Reserved to `thrust_core::authorize_workspace`, after the membership check passes.
    #[doc(hidden)]
    pub fn mint(workspace_id: WorkspaceId, user_id: UserId, role: Role) -> Self {
        Self { workspace_id, user_id, role }
    }
    pub fn workspace_id(&self) -> WorkspaceId {
        self.workspace_id
    }
    pub fn user_id(&self) -> UserId {
        self.user_id
    }
    pub fn role(&self) -> Role {
        self.role
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

/// What a person says about themselves and how they have set the app up: one row per user, a directory table (keyed by
/// the user, no workspace; `last_workspace_id` is a value, not a tenant key). The typed fields are the ones core or the
/// router acts on; `preferences` holds the rest as a JSON object (theme, language, role, use case, ...), which the
/// compat layer fills with defaults where a key is missing, so a setting costs no migration (issue #44).
#[derive(Debug, Clone, PartialEq)]
pub struct UserProfile {
    /// The profile row's own id (Plane's `Profile.id`), distinct from the user's.
    pub id: Uuid,
    pub user_id: UserId,
    /// Made at sign-up and never shown to anyone as a name: 32 hex digits.
    pub username: String,
    pub first_name: String,
    pub last_name: String,
    pub display_name: String,
    pub avatar: String,
    pub cover_image: Option<String>,
    /// An IANA name from the list `thrust_core::is_known_timezone` knows.
    pub timezone: String,
    pub date_joined: Timestamp,
    pub last_login_at: Option<Timestamp>,
    pub is_onboarded: bool,
    pub is_tour_completed: bool,
    /// Not checked against any workspace (Plane does not either): the app writes it and reads it back.
    pub last_workspace_id: Option<WorkspaceId>,
    /// A JSON object of step name to bool; replaced whole by a patch, as the client sends the merged object.
    pub onboarding_step: serde_json::Value,
    /// A JSON object; a patch sets the keys it names and leaves the others.
    pub preferences: serde_json::Value,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

/// A change to a profile: the fields that are `Some` (and the keys of `preferences`) are set, the rest stay. The adapter
/// applies it in ONE statement, so two requests that change different fields of one profile (the onboarding page sends
/// several at once) cannot lose each other's change.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ProfilePatch {
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub display_name: Option<String>,
    pub avatar: Option<String>,
    /// `Some(None)` clears it.
    pub cover_image: Option<Option<String>>,
    pub timezone: Option<String>,
    pub last_login_at: Option<Timestamp>,
    pub is_onboarded: Option<bool>,
    pub is_tour_completed: Option<bool>,
    /// `Some(None)` clears it.
    pub last_workspace_id: Option<Option<WorkspaceId>>,
    pub onboarding_step: Option<serde_json::Value>,
    /// The keys to set; the others are kept.
    pub preferences: serde_json::Map<String, serde_json::Value>,
}

/// A workspace as its table holds it (issue #44). The slug is unique and is what routes name it by.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Workspace {
    pub id: WorkspaceId,
    pub slug: WorkspaceSlug,
    pub name: String,
    /// The user who made it; none for a workspace a test seeded.
    pub owner_id: Option<UserId>,
    pub organization_size: Option<String>,
    pub timezone: String,
    pub background_color: String,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

/// A user's membership of a workspace. `props` is a JSON object of the per-member view settings (the compat layer
/// fills the keys a member has not set with defaults).
#[derive(Debug, Clone, PartialEq)]
pub struct WorkspaceMember {
    /// The membership row's own id, not the user's.
    pub id: Uuid,
    pub workspace_id: WorkspaceId,
    pub user_id: UserId,
    pub role: Role,
    pub company_role: Option<String>,
    /// A deactivated member keeps the row (and shows in the member list as suspended) but has no access.
    pub is_active: bool,
    pub props: serde_json::Value,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

/// A workspace and its first member, inserted together or not at all.
#[derive(Debug, Clone, PartialEq)]
pub struct NewWorkspace {
    pub workspace: Workspace,
    pub owner: WorkspaceMember,
}

/// A workspace as one of its members sees it: their role and how many active members it has.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceListing {
    pub workspace: Workspace,
    pub role: Role,
    pub total_members: u64,
}

/// What a workspace's member list says about the person behind a row. A user with no profile has empty names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemberUser {
    pub id: UserId,
    pub email: Option<String>,
    pub first_name: String,
    pub last_name: String,
    pub display_name: String,
    pub avatar: String,
}

/// A row of a workspace's member list.
#[derive(Debug, Clone, PartialEq)]
pub struct MemberWithUser {
    pub member: WorkspaceMember,
    pub user: MemberUser,
}

/// An active member's way into a workspace, resolved from its slug.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkspaceAccess {
    pub workspace_id: WorkspaceId,
    pub role: Role,
}

/// A project as its table holds it (issue #44). `intake_view` is what Plane's project list calls `inbox_view`.
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectRow {
    pub id: ProjectId,
    pub workspace_id: WorkspaceId,
    pub name: String,
    /// The project's key, which prefixes its work items (`JRN-1`).
    pub identifier: String,
    pub network: ProjectNetwork,
    /// The icon or emoji the project shows, as a JSON object.
    pub logo_props: serde_json::Value,
    pub archived_at: Option<Timestamp>,
    pub cycle_view: bool,
    pub issue_views_view: bool,
    pub module_view: bool,
    pub page_view: bool,
    pub intake_view: bool,
    pub guest_view_all_features: bool,
    pub project_lead: Option<UserId>,
    pub created_by: Option<UserId>,
    pub updated_by: Option<UserId>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    // What the list leaves out and the project's own routes show (migration 0012).
    pub description: String,
    pub description_text: Option<serde_json::Value>,
    pub description_html: Option<serde_json::Value>,
    pub emoji: Option<String>,
    pub icon_prop: Option<serde_json::Value>,
    pub is_time_tracking_enabled: bool,
    pub is_issue_type_enabled: bool,
    pub cover_image: Option<String>,
    pub cover_image_asset: Option<Uuid>,
    pub estimate: Option<Uuid>,
    /// Months, 0 to 12.
    pub archive_in: i32,
    pub close_in: i32,
    pub default_state: Option<Uuid>,
    pub timezone: String,
    pub external_source: Option<String>,
    pub external_id: Option<String>,
    pub default_assignee: Option<UserId>,
}

impl ProjectRow {
    /// A project made with no options, as the seed and the tests make one: public, nothing archived, only pages on, the identifier
    /// `default_project_identifier` makes from its id, UTC, and `at` for its time.
    pub fn bare(id: ProjectId, workspace_id: WorkspaceId, name: impl Into<String>, at: Timestamp) -> ProjectRow {
        ProjectRow {
            id,
            workspace_id,
            name: name.into(),
            identifier: default_project_identifier(id),
            network: ProjectNetwork::default(),
            logo_props: serde_json::json!({}),
            archived_at: None,
            cycle_view: false,
            issue_views_view: false,
            module_view: false,
            page_view: true,
            intake_view: false,
            guest_view_all_features: false,
            project_lead: None,
            created_by: None,
            updated_by: None,
            created_at: at,
            updated_at: at,
            description: String::new(),
            description_text: None,
            description_html: None,
            emoji: None,
            icon_prop: None,
            is_time_tracking_enabled: false,
            is_issue_type_enabled: false,
            cover_image: None,
            cover_image_asset: None,
            estimate: None,
            archive_in: 0,
            close_in: 0,
            default_state: None,
            timezone: "UTC".to_string(),
            external_source: None,
            external_id: None,
            default_assignee: None,
        }
    }
}

/// A project in the list of one workspace member: the project, the role the member holds in it (none when they are not in
/// it: a project they see only because it is public, or because they administer the workspace), and where their own list
/// puts it (none when they have no row of their own for it).
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectListing {
    pub project: ProjectRow,
    pub role: Option<Role>,
    pub sort_order: Option<f64>,
}

/// One entry of a member's sidebar for a workspace: whether it is pinned and where it goes.
#[derive(Debug, Clone, PartialEq)]
pub struct SidebarPreference {
    pub key: String,
    pub is_pinned: bool,
    pub sort_order: f64,
}

/// One widget of a member's home page for a workspace: whether it shows, what it is set to and where it goes. `created_at`
/// is when the member's row was made, which is the order the pin lists them in.
#[derive(Debug, Clone, PartialEq)]
pub struct HomePreference {
    pub key: String,
    pub is_enabled: bool,
    pub config: serde_json::Value,
    pub sort_order: f64,
    pub created_at: Timestamp,
}

/// A member's settings for the work-item lists of one workspace. The four JSON values are objects.
#[derive(Debug, Clone, PartialEq)]
pub struct UserProperties {
    pub id: Uuid,
    pub filters: serde_json::Value,
    pub display_filters: serde_json::Value,
    pub display_properties: serde_json::Value,
    pub rich_filters: serde_json::Value,
    pub navigation_project_limit: i32,
    /// `ACCORDION` or `TABBED`.
    pub navigation_control_preference: String,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

/// A person's row in a project (issue #44, migration 0012): their role, their view settings (JSON objects, in which the compat
/// layer fills the keys a row has not set), their place in the project's own member list, and who made the row and when.
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectMemberRow {
    /// The row's own id, not the user's.
    pub id: Uuid,
    pub project_id: ProjectId,
    /// The workspace the row was written with, which is the project's own for every row but a stray one.
    pub workspace_id: WorkspaceId,
    pub user_id: UserId,
    pub role: Role,
    pub comment: Option<String>,
    pub view_props: serde_json::Value,
    pub default_props: serde_json::Value,
    pub preferences: serde_json::Value,
    pub sort_order: f64,
    pub created_by: Option<UserId>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

/// A member's own settings for one project, and where their list puts it.
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectUserProperties {
    pub id: Uuid,
    pub project_id: ProjectId,
    pub workspace_id: WorkspaceId,
    pub user_id: UserId,
    pub filters: serde_json::Value,
    pub display_filters: serde_json::Value,
    pub display_properties: serde_json::Value,
    pub rich_filters: serde_json::Value,
    pub preferences: serde_json::Value,
    pub sort_order: f64,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

/// A work-item state of a project.
#[derive(Debug, Clone, PartialEq)]
pub struct StateRow {
    pub id: Uuid,
    pub workspace_id: WorkspaceId,
    pub project_id: ProjectId,
    pub name: String,
    pub description: String,
    pub color: String,
    /// The state's place among its project's, and what the lists order by.
    pub sequence: f64,
    pub group: StateGroup,
    pub is_default: bool,
    pub created_by: Option<UserId>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

/// A label of a project.
#[derive(Debug, Clone, PartialEq)]
pub struct LabelRow {
    pub id: Uuid,
    pub workspace_id: WorkspaceId,
    pub project_id: ProjectId,
    pub parent_id: Option<Uuid>,
    pub name: String,
    pub description: String,
    pub color: String,
    pub sort_order: f64,
    pub created_by: Option<UserId>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

/// A person to add to a project that is being made, with their own settings row. The adapter replaces
/// `properties.sort_order` with the place the pin gives a project a person has just joined: 10000 less than the lowest place they
/// hold in the workspace, or 65535 when they hold none.
#[derive(Debug, Clone, PartialEq)]
pub struct NewProjectMember {
    pub member: ProjectMemberRow,
    pub properties: ProjectUserProperties,
}

/// A project, its first members and its states, inserted together or not at all.
#[derive(Debug, Clone, PartialEq)]
pub struct NewProject {
    pub project: ProjectRow,
    pub members: Vec<NewProjectMember>,
    pub states: Vec<StateRow>,
}

/// Whether a name or a key is already a project's in the workspace (exact match, `Plane` and `plane` are two names). A field that
/// was not asked about is `false`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ProjectClashes {
    pub name: bool,
    pub identifier: bool,
}

/// A project with what the app shows beside it: the caller's role and place in it, who is in it, and the number the next work
/// item will have.
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectDetail {
    pub listing: ProjectListing,
    /// The people in the project, newest row first.
    pub members: Vec<UserId>,
    pub next_sequence: i64,
}

/// A member's row with the person behind it, the project and the workspace, as the member's own route shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectMemberWithUser {
    pub member: ProjectMemberRow,
    pub user: MemberUser,
    pub project: ProjectRow,
    pub workspace: Workspace,
}

/// What a workspace's project statistics say of one project.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProjectStat {
    pub id: ProjectId,
    /// Work items that count: not deleted, not archived, not a draft.
    pub total_issues: i64,
    pub total_members: i64,
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

    /// The user with this id, active or not.
    async fn user_by_id(&self, id: UserId) -> Result<Option<UserRecord>, DbError>;

    /// Create a profile (issue #44). `false`, and nothing written, when the user already has one: signing up and the
    /// first read of an older account both ask, and the first to arrive wins.
    async fn insert_profile(&self, profile: &UserProfile) -> Result<bool, DbError>;

    /// The user's profile, if they have one.
    async fn profile(&self, user: UserId) -> Result<Option<UserProfile>, DbError>;

    /// Apply `patch` to the user's profile at `at` (`updated_at`), atomically, and return the row as it is after. `None`
    /// when the user has no profile.
    async fn update_profile(&self, user: UserId, patch: &ProfilePatch, at: Timestamp) -> Result<Option<UserProfile>, DbError>;

    // --- Workspaces (issue #44). The first four run before there is a `WorkspaceScope` (they are how one is made, or they
    // are about the caller's own workspaces), the rest take one.

    /// Create a workspace and its first member in one transaction. A taken slug is `DbError::Conflict`, raised by the UNIQUE
    /// index, so two creations racing for one slug cannot both win, and the loser leaves no member row behind.
    async fn insert_workspace(&self, w: &NewWorkspace) -> Result<(), DbError>;

    /// Whether some workspace has this slug (exact, case-sensitive).
    async fn workspace_slug_taken(&self, slug: &str) -> Result<bool, DbError>;

    /// Whether any workspace exists at all (what `workspaces_exist` on the instance reports).
    async fn any_workspace(&self) -> Result<bool, DbError>;

    /// The workspaces `user` is an ACTIVE member of, newest first (ties by id), with their role and the member count.
    async fn workspaces_of(&self, user: UserId) -> Result<Vec<WorkspaceListing>, DbError>;

    /// The workspace `slug` names and the role `user` holds in it, when `user` is an active member. `None` for a slug that
    /// names nothing and for a user who is not (or no longer) a member, which look the same on purpose.
    async fn workspace_access(&self, slug: &WorkspaceSlug, user: UserId) -> Result<Option<WorkspaceAccess>, DbError>;

    /// Whether `user` (anyone, not only the caller) is an active member of the scope's workspace.
    async fn is_workspace_member(&self, scope: &WorkspaceScope, user: UserId) -> Result<bool, DbError>;

    /// Deactivate or reactivate a membership: the row stays (the member list shows the person as suspended) but an inactive
    /// member has no access to the workspace's slug, its projects or its member list. Test support and the seam of the real
    /// deactivation (ADR-0053), like `set_user_active`. `true` if the membership exists.
    async fn set_workspace_member_active(
        &self,
        sys: &SystemScope,
        workspace: WorkspaceId,
        user: UserId,
        active: bool,
    ) -> Result<bool, DbError>;

    /// The caller's own membership row.
    async fn workspace_member(&self, scope: &WorkspaceScope) -> Result<Option<WorkspaceMember>, DbError>;

    /// Every member of the workspace, deactivated ones included, newest first (ties by id).
    async fn workspace_members(&self, scope: &WorkspaceScope) -> Result<Vec<MemberWithUser>, DbError>;

    /// The caller's role in each project of the workspace they are an active member of, by project id.
    async fn project_roles(&self, scope: &WorkspaceScope) -> Result<Vec<(ProjectId, Role)>, DbError>;

    /// The projects of the workspace the caller sees, newest first (ties by id), in ONE statement (issue #39's budget). An
    /// admin of the workspace sees all of them, archived and secret ones included; a member sees the ones they are in and
    /// the public ones; a guest sees only the ones they are in. A project's member row counts only when it names the
    /// project's own workspace, as in `project_roles`.
    async fn workspace_projects(&self, scope: &WorkspaceScope) -> Result<Vec<ProjectListing>, DbError>;

    /// Make the caller's sidebar rows for this workspace that are not there yet, from `defaults` (at time `at`), and return
    /// all of them: by `sort_order`, ties by key. A row that exists is left exactly as it is, so two first visits at once
    /// make one row each.
    async fn ensure_sidebar_preferences(
        &self,
        scope: &WorkspaceScope,
        defaults: &[SidebarPreference],
        at: Timestamp,
    ) -> Result<Vec<SidebarPreference>, DbError>;

    /// As [`Database::ensure_sidebar_preferences`], for the home page's widgets: a row made here keeps the `created_at` of
    /// its default, and the rows come back by key.
    async fn ensure_home_preferences(&self, scope: &WorkspaceScope, defaults: &[HomePreference]) -> Result<Vec<HomePreference>, DbError>;

    /// The caller's work-item list settings for this workspace: `new` is stored (at its own time) if there is no row yet, and
    /// whichever row is there is returned. A row that exists is left exactly as it is.
    async fn ensure_user_properties(&self, scope: &WorkspaceScope, new: &UserProperties) -> Result<UserProperties, DbError>;

    // --- Projects (issue #44, the project stage). The writes take the workspace's scope: who may make or change a project is
    // the core's to decide (it reads the caller's roles), and the adapter writes under the workspace the scope names.

    /// Make a project, its first members (each with a settings row) and its states, together or not at all. A name or key that
    /// another project of the workspace has is `DbError::Conflict`, raised by the UNIQUE indexes, so two creations racing for one
    /// name cannot both win and the loser leaves nothing behind.
    async fn insert_project(&self, scope: &WorkspaceScope, p: &NewProject) -> Result<(), DbError>;

    /// Whether the workspace has a project with this name, and one with this key (exact), leaving out `except` (the project being
    /// renamed). A name or key that is `None` is not asked about.
    async fn project_clashes(
        &self,
        scope: &WorkspaceScope,
        name: Option<&str>,
        identifier: Option<&str>,
        except: Option<ProjectId>,
    ) -> Result<ProjectClashes, DbError>;

    /// Replace everything but the id, the workspace and the creation of the project `row.id` of the scope's workspace with
    /// `row`'s values (the pin saves every column, as this does). `false` when the workspace has no such project; a name or key
    /// another project has is `DbError::Conflict`.
    async fn update_project(&self, scope: &WorkspaceScope, row: &ProjectRow) -> Result<bool, DbError>;

    /// The project of the scope's workspace with the caller's role and place in it, who is in it and its next sequence number;
    /// `None` when the workspace has no such project. It does not ask whether the caller may see it: that is the core's rule.
    async fn project_detail(&self, scope: &WorkspaceScope, id: ProjectId) -> Result<Option<ProjectDetail>, DbError>;

    /// The caller's own row in the project, with who they are; `None` when they have none (also for a project of another
    /// workspace, a row that names another workspace than the project's is no row).
    async fn project_member(&self, scope: &WorkspaceScope, project: ProjectId) -> Result<Option<ProjectMemberWithUser>, DbError>;

    /// The people in the project who are active members of its workspace, newest row first (ties by id).
    async fn project_members(&self, scope: &Scope) -> Result<Vec<ProjectMemberRow>, DbError>;

    /// The caller's settings for the project: `new` is stored if they have none (a row the project's creation did not make), and
    /// whichever row is there is returned. A row that exists is left exactly as it is.
    async fn ensure_project_user_properties(&self, scope: &Scope, new: &ProjectUserProperties) -> Result<ProjectUserProperties, DbError>;

    /// The project's labels by `sort_order` (ties by id).
    async fn project_labels(&self, scope: &Scope) -> Result<Vec<LabelRow>, DbError>;

    /// The project's states, the intake one left out, by `sequence` (ties by id).
    async fn project_states(&self, scope: &Scope) -> Result<Vec<StateRow>, DbError>;

    /// The states of every project of the workspace that the caller is in and that is not archived, the intake ones left out, by
    /// `sequence` (ties by project, then id).
    async fn workspace_states(&self, scope: &WorkspaceScope) -> Result<Vec<StateRow>, DbError>;

    /// The project's intake state (group `triage`), if it has one.
    async fn triage_state(&self, scope: &Scope) -> Result<Option<StateRow>, DbError>;

    /// Counts for the projects of the scope's workspace (all of them, or those in `only`), newest first (ties by id), whoever the
    /// caller is in them.
    async fn project_stats(&self, scope: &WorkspaceScope, only: Option<&[ProjectId]>) -> Result<Vec<ProjectStat>, DbError>;

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
