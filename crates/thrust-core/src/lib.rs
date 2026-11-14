//! Application core: the permission seam and the use-cases of the vertical slice.
//!
//! Nothing in here knows whether it runs in a Worker or in a native server. The only inputs are
//! the port trait objects in [`Ports`].

use std::sync::Arc;

use sha2::{Digest, Sha256};
use thiserror::Error;

// The unit tests in `auth/key_hash_tests.rs` reuse the integration tests' world (`tests/common`), which names this crate
// the way an integration test does.
#[cfg(test)]
extern crate self as thrust_core;

mod auth;
pub use auth::*;
mod password_reset;
pub use password_reset::*;
mod profile;
pub use profile::*;
mod reserved_slugs;
#[cfg(test)]
mod reserved_slugs_tests;
mod timezones;
mod workspace;
pub use workspace::*;
mod workspace_load;
pub use workspace_load::*;
mod assets;
pub use assets::*;
mod projects;
pub use projects::*;
mod issues;
pub use issues::*;
mod labels;
pub use labels::*;
mod states;
pub use states::*;
#[cfg(test)]
mod timezones_tests;
use thrust_contracts::chrono::TimeDelta;
use thrust_contracts::*;
use thrust_ports::*;

// ---------------------------------------------------------------------------------------------
// Wiring
// ---------------------------------------------------------------------------------------------

#[derive(Clone)]
pub struct Ports {
    pub db: Arc<dyn Database>,
    pub queue: Arc<dyn Queue>,
    pub blobs: Arc<dyn BlobStore>,
    pub limiter: Arc<dyn RateLimiter>,
    pub clock: Arc<dyn Clock>,
    pub ids: Arc<dyn IdGen>,
    pub background: Arc<dyn Background>,
    /// Secure random bytes: tokens, keys, salts (issue #38).
    pub rng: Arc<dyn Rng>,
    /// Password hashing, counted in tests (issue #38).
    pub hasher: Arc<dyn PasswordHasher>,
    /// The one OIDC provider, when the deployment configured one (ADR-0050; issue #38). `None`: the login routes are
    /// 404 and nothing else changes: passwords and API keys never involve it.
    pub oidc: Option<Arc<dyn OidcProvider>>,
    /// The deployment's mail service, when it has one (ADR-0073, Q-0073.1; issue #46). `None`: nothing that needs mail is on, a
    /// password reset included, and the app is told so (`is_smtp_configured`).
    pub email: Option<Arc<dyn EmailSender>>,
}

#[derive(Debug, Clone)]
pub struct CoreConfig {
    /// ADR-0046 relay: publish right after commit (true) or leave every row to the cron sweeper.
    pub outbox_fastpath: bool,
    pub max_attachment_bytes: usize,
    /// How long a session lives from its creation (ADR-0052: 30 days).
    pub session_ttl_secs: i64,
    /// `authenticate` rewrites a session's `last_seen_at` only when it is at least this old, so a busy
    /// session costs one write an hour, not one per request (ADR-0052 round 29: updated lazily).
    pub session_touch_after_secs: i64,
    /// Cost of a new password hash. The default is OWASP's minimum; a test target sets a cheap one through its
    /// configuration, and one test pins the default into the encoded hash (issue #38).
    pub argon2: Argon2Params,
    /// Sign-in attempts per email inside `sign_in_rate_window_secs`, counted before any hashing (so a refused attempt
    /// costs nothing, and a known and an unknown email are limited alike).
    pub sign_in_rate_limit: u32,
    pub sign_in_rate_window_secs: u64,
    /// How long a started OIDC login may wait for its callback before the stored `state` is useless.
    pub oidc_login_ttl_secs: i64,
    /// The digest scheme new API keys are stored under, and the one a key is moved to the next time it is used while
    /// it is stored under another (Q-0052.3). Defaults to [`API_KEY_HASH_CURRENT`]: version 1, plain SHA-256. Not read
    /// from the environment: changing it means shipping a scheme (see `api_key_hash`), which is a code change.
    pub api_key_hash_current: i16,
    /// Every scheme an `X-Api-Key` is looked up under. The current one is always tried, whether listed or not (a key
    /// this build writes must be one it can read); dropping an older version from here is how its keys are retired,
    /// on purpose. Defaults to [`API_KEY_HASH_SUPPORTED`].
    pub api_key_hash_supported: Vec<i16>,
    /// Whether a mutation authenticated by the session COOKIE must carry the CSRF token (ADR-0052, double submit,
    /// Q-0050.8). The rule as the ADR wrote it is the core's default: `true`. The composition roots read
    /// `THRUST_CSRF_ENFORCE` and default it to `false` while the stock Plane frontend is what talks to this server,
    /// because that app sends the token only on its auth form posts, so enforcing it would refuse its JSON mutations;
    /// the flip is to `true` when the custom frontend replaces it. Off changes exactly one thing: the token is not
    /// consulted for a cookie mutation. The cookie, `GET /auth/get-csrf-token/`, `csrf_ok`, and a bearer token or an API key (which
    /// never needed the token) are untouched.
    pub csrf_enforce: bool,
    /// How long the address an asset's bytes are uploaded to keeps working (ADR-0047; the pin's `SIGNED_URL_EXPIRATION`, an hour).
    pub asset_upload_ttl_secs: i64,
    /// The address the links in emails start with (`https://projects.example.com`), from the deployment's configuration and never
    /// from a request's `Host` header, which a caller controls: with a forged one a reset mail would carry a working token to a
    /// page that is not ours. `None`: no link can be made, so a password reset is off (D-041).
    pub public_base_url: Option<String>,
    /// How long a mailed password-reset link works (the pin's is three days; an hour is what a person needs and less for a stolen mailbox).
    pub password_reset_ttl_secs: i64,
    /// A request for an account whose link is younger than this makes and sends nothing: the form is not a way to mail someone over and over.
    pub password_reset_cooldown_secs: i64,
}

impl Default for CoreConfig {
    fn default() -> Self {
        Self {
            outbox_fastpath: true,
            max_attachment_bytes: 5 * 1024 * 1024,
            session_ttl_secs: 30 * 24 * 60 * 60,
            session_touch_after_secs: 60 * 60,
            argon2: Argon2Params::OWASP_MINIMUM,
            sign_in_rate_limit: 10,
            sign_in_rate_window_secs: 60,
            oidc_login_ttl_secs: 10 * 60,
            api_key_hash_current: API_KEY_HASH_CURRENT,
            api_key_hash_supported: API_KEY_HASH_SUPPORTED.to_vec(),
            csrf_enforce: true,
            asset_upload_ttl_secs: 60 * 60,
            public_base_url: None,
            password_reset_ttl_secs: 60 * 60,
            password_reset_cooldown_secs: 60,
        }
    }
}

#[derive(Clone)]
pub struct App {
    pub ports: Ports,
    pub cfg: CoreConfig,
    /// The proof the outbox relay, the sweeper, the consumer and `seed` pass to the untenanted
    /// `Database` calls. Private: only the code in this crate that does that work reads it.
    system: Arc<SystemScope>,
}

impl App {
    /// `system` is minted by the composition root that builds the `App` (ADR-0045 round 29).
    pub fn new(ports: Ports, cfg: CoreConfig, system: SystemScope) -> Self {
        Self { ports, cfg, system: Arc::new(system) }
    }
}

pub const EVENTS_TOPIC: &str = "events";

// ---------------------------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("unauthenticated")]
    Unauthenticated,
    #[error("forbidden")]
    Forbidden,
    /// A cookie-authenticated mutation without a matching CSRF token (ADR-0052, double submit).
    #[error("csrf token missing or wrong")]
    Csrf,
    /// An `X-Api-Key` that does not resolve: unknown, revoked, expired, or its user is inactive. The oracle answers 403
    /// for all four and 401 only for a missing key (ADR-0033, round 27), so the lookup does not say which.
    #[error("given API key is not valid")]
    InvalidApiKey,
    #[error("not found")]
    NotFound,
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("rate limited, retry after {retry_after_secs}s")]
    RateLimited { retry_after_secs: u64 },
    #[error("invalid: {0}")]
    Invalid(String),
    #[error("payload too large")]
    TooLarge,
    /// A server this one depends on (the OIDC provider) failed or answered something unusable.
    #[error("upstream: {0}")]
    Upstream(String),
    #[error("internal: {0}")]
    Internal(String),
}

impl From<DbError> for CoreError {
    fn from(e: DbError) -> Self {
        match e {
            DbError::Conflict(m) => CoreError::Conflict(m),
            DbError::Backend(m) => CoreError::Internal(format!("db: {m}")),
        }
    }
}
impl From<QueueError> for CoreError {
    fn from(e: QueueError) -> Self {
        CoreError::Internal(e.to_string())
    }
}
impl From<BlobError> for CoreError {
    fn from(e: BlobError) -> Self {
        CoreError::Internal(e.to_string())
    }
}
impl From<RateError> for CoreError {
    fn from(e: RateError) -> Self {
        CoreError::Internal(e.to_string())
    }
}

// ---------------------------------------------------------------------------------------------
// Permission seam (ADR-0051 shape: a `Scope` proof value that only `authorize` can mint)
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Permission {
    /// What a member of a project may read about it (its people, states, labels and their own settings).
    ReadProject,
    ReadWorkItems,
    CreateWorkItem,
    /// Change a work item's name, state, priority, dates, people, labels or description (issue #44, X03).
    UpdateWorkItem,
    ReadAttachment,
    WriteAttachment,
    /// Make, change or delete a state of a project (issue #58): for a project admin only, as Plane's `ProjectAdminPermission` has it.
    /// A member of the project is refused, and so is a workspace admin who is not in it.
    WriteState,
    /// Make, change or delete a label of a project (issue #58): for a project admin or member, as Plane's `ProjectMemberPermission`
    /// has it (its comment says "only Project Admins" next to a role list that holds the member too). A guest is refused, and so is
    /// a workspace admin who is not in the project.
    WriteLabel,
}

impl Permission {
    fn min_role(self) -> Role {
        match self {
            Permission::ReadProject | Permission::ReadWorkItems | Permission::ReadAttachment => Role::Guest,
            Permission::CreateWorkItem
            | Permission::UpdateWorkItem
            | Permission::WriteAttachment
            | Permission::WriteLabel => Role::Member,
            Permission::WriteState => Role::Admin,
        }
    }
}

/// The proof types are declared in `thrust-ports`, so that the ports can take them (issue #35).
/// [`authorize`] is the only place a [`Scope`] is minted.
pub use thrust_ports::{Scope, SystemScope, WorkspaceScope};

/// Resolves the route's workspace slug, the caller's membership of that workspace and of the project
/// in one query (ADR-0051 round 29), and mints the [`Scope`]. 404 for anything the caller may not
/// know about: a slug that names no workspace, a workspace the caller does not belong to, or a
/// project that is not in it (cross-tenant probes learn nothing). 403 for a workspace member who is
/// not a member of the project, or whose role is too low. (Plane answers 403 for an unknown slug;
/// that is the compat surface's business, #44.)
///
/// The [`Scope`] it returns is the only key to a repository call (issue #35). A use-case cannot
/// build one by struct literal:
///
/// ```compile_fail
/// use thrust_contracts::*;
/// use thrust_core::Scope;
/// let ids = (WorkspaceId(Uuid::nil()), ProjectId(Uuid::nil()), UserId(Uuid::nil()));
/// let _ = Scope { workspace_id: ids.0, project_id: ids.1, user_id: ids.2 };
/// ```
///
/// and cannot reach a repository with a bare id where the proof goes:
///
/// ```compile_fail
/// use thrust_contracts::*;
/// use thrust_core::*;
/// async fn use_case(app: &App, ws: WorkspaceId, item: WorkItemId) {
///     let _ = app.ports.db.get_work_item(ws, item, Visibility::Visible).await;
/// }
/// ```
///
/// Control for both: the same imports and the same call, through `authorize`, compile. Rustdoc on stable does not check which
/// error a `compile_fail` block gets, so the control is what shows the reason: its call is the failing one's, the same method with
/// the same arguments, and it differs only in having the proof in the first place. The one remaining path, the `pub` constructor
/// `Scope::mint`, is closed by `scripts/check-deps.sh`.
///
/// ```
/// use thrust_contracts::*;
/// use thrust_core::*;
/// async fn use_case(
///     app: &App,
///     who: &Principal,
///     slug: &WorkspaceSlug,
///     project: ProjectId,
///     item: WorkItemId,
/// ) -> Result<Option<WorkItem>, CoreError> {
///     let scope = authorize(app, who, slug, project, Permission::ReadWorkItems).await?;
///     let _ids = (scope.workspace_id(), scope.project_id(), UserId(Uuid::nil()));
///     Ok(app.ports.db.get_work_item(&scope, item, Visibility::Visible).await?)
/// }
/// ```
pub async fn authorize(
    app: &App,
    who: &Principal,
    slug: &WorkspaceSlug,
    project: ProjectId,
    perm: Permission,
) -> Result<Scope, CoreError> {
    authorize_with_role(app, who, slug, project, perm).await.map(|(scope, _)| scope)
}

/// [`authorize`], and the caller's role in the project that let them in (a use-case that treats a guest differently asks for
/// it). This is where a [`Scope`] is minted; `authorize` is this and nothing else.
pub async fn authorize_with_role(
    app: &App,
    who: &Principal,
    slug: &WorkspaceSlug,
    project: ProjectId,
    perm: Permission,
) -> Result<(Scope, Role), CoreError> {
    match app.ports.db.membership(slug, project, who.user_id).await? {
        Membership::NoSuchProject => Err(CoreError::NotFound),
        Membership::NotAMember => Err(CoreError::Forbidden),
        Membership::Member { workspace_id, role } if role >= perm.min_role() => {
            Ok((Scope::mint(workspace_id, project, who.user_id), role))
        }
        Membership::Member { .. } => Err(CoreError::Forbidden),
    }
}

/// Resolves the route's workspace slug and the caller's ACTIVE membership of it, and mints the [`WorkspaceScope`] (issue
/// #44). `NotFound` for a slug that names no workspace and for a workspace the caller is not (or no longer) a member of: the
/// two look the same on purpose, so a caller cannot probe for workspaces. The compat layer maps that to the status each of
/// Plane's routes gives such a caller.
///
/// This is the only place a [`WorkspaceScope`] is minted; `scripts/check-deps.sh` rejects a call to
/// `WorkspaceScope::mint` anywhere else.
pub async fn authorize_workspace(
    app: &App,
    who: &Principal,
    slug: &WorkspaceSlug,
) -> Result<WorkspaceScope, CoreError> {
    match app.ports.db.workspace_access(slug, who.user_id).await? {
        Some(a) => Ok(WorkspaceScope::mint(a.workspace_id, who.user_id, a.role)),
        None => Err(CoreError::NotFound),
    }
}

/// The digest the database stores and compares for a session token. A token is a high-entropy random value
/// (sign-in, #38, must mint at least 128 bits of CSPRNG output), so a plain SHA-256 is enough: there is nothing to
/// guess and nothing to stretch, and a pepper would only add a secret to every target (owner decision,
/// 2026-10-22; ADR-0052, issue #36 note).
pub fn hash_token(token: &str) -> TokenHash {
    TokenHash(Sha256::digest(token.as_bytes()).into())
}

/// A bearer token to the user it belongs to, or `Unauthenticated`: the row must exist, not be revoked
/// and not be expired. The row is read on every request, so a revocation is effective at once
/// (ADR-0052 round 29).
pub async fn authenticate(app: &App, token: &str) -> Result<Principal, CoreError> {
    let hash = hash_token(token);
    let now = app.ports.clock.now();
    let live = app.ports.db.session(&hash, now).await?.ok_or(CoreError::Unauthenticated)?;
    if now - live.last_seen_at >= TimeDelta::seconds(app.cfg.session_touch_after_secs) {
        // `last_seen_at` is bookkeeping: failing to write it must not fail a request that is authentic.
        let _ = app.ports.db.touch_session(&hash, now).await;
    }
    Ok(Principal { user_id: live.user_id })
}

/// Sign-out for one token. Idempotent: an unknown or already revoked token changes nothing.
pub async fn revoke_session(app: &App, token: &str) -> Result<(), CoreError> {
    sign_out(app, &hash_token(token)).await
}

// ---------------------------------------------------------------------------------------------
// Use-cases
// ---------------------------------------------------------------------------------------------

fn validate_name(name: &str) -> Result<(), CoreError> {
    let n = name.trim();
    if n.is_empty() || n.chars().count() > 255 {
        return Err(CoreError::Invalid("name must be 1..=255 characters".into()));
    }
    Ok(())
}

fn event(
    app: &App,
    ws: WorkspaceId,
    actor: UserId,
    event_type: &str,
    entity_id: WorkItemId,
    payload: serde_json::Value,
) -> OutboxRow {
    let now = app.ports.clock.now();
    OutboxRow {
        envelope: Envelope {
            event_id: EventId(app.ports.ids.new_uuid()),
            schema_version: 1,
            workspace_id: ws,
            event_type: event_type.to_string(),
            entity_type: "work_item".to_string(),
            entity_id: entity_id.as_uuid(),
            actor_id: Some(actor),
            occurred_at: now,
            payload,
            trace_id: None,
        },
        created_at: now,
        dispatched_at: None,
        attempts: 0,
    }
}

pub async fn list_activity(
    app: &App,
    who: &Principal,
    slug: &WorkspaceSlug,
    project: ProjectId,
    entity: Option<WorkItemId>,
) -> Result<Vec<Activity>, CoreError> {
    let scope = authorize(app, who, slug, project, Permission::ReadWorkItems).await?;
    Ok(app.ports.db.list_activity(&scope, entity).await?)
}

// --- ADR-0046 relay -------------------------------------------------------------------------

/// Publish one outbox row and mark it dispatched. Publish failure leaves the row for the sweeper.
pub async fn dispatch(app: &App, row: &OutboxRow) -> Result<(), CoreError> {
    app.ports
        .queue
        .send(
            EVENTS_TOPIC,
            &row.envelope,
            SendOpts {
                idempotency_key: Some(row.envelope.event_id.to_string()),
                ordering_key: Some(format!("{}:{}", row.envelope.workspace_id, row.envelope.entity_id)),
                ..Default::default()
            },
        )
        .await?;
    app.ports.db.mark_outbox_dispatched(&app.system, row.envelope.event_id, app.ports.clock.now()).await?;
    Ok(())
}

fn relay_after_commit(app: &App, row: OutboxRow) {
    if !app.cfg.outbox_fastpath {
        return;
    }
    let app2 = app.clone();
    app.ports.background.spawn(Box::pin(async move {
        // Errors are deliberately swallowed: the row stays undispatched and the sweeper retries.
        let _ = dispatch(&app2, &row).await;
    }));
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SweepReport {
    pub found: usize,
    pub published: usize,
    pub failed: usize,
}

/// The cron sweeper (ADR-0046 Q-0046.1): publishes rows the fast path did not.
pub async fn sweep_outbox(app: &App, limit: u32) -> Result<SweepReport, CoreError> {
    let rows = app.ports.db.undispatched_outbox(&app.system, limit).await?;
    let mut rep = SweepReport { found: rows.len(), published: 0, failed: 0 };
    for row in &rows {
        match dispatch(app, row).await {
            Ok(()) => rep.published += 1,
            Err(_) => rep.failed += 1,
        }
    }
    Ok(rep)
}

/// Rows the relay has not published yet (`/internal/outbox-depth`).
pub async fn outbox_depth(app: &App) -> Result<u64, CoreError> {
    Ok(app.ports.db.outbox_depth(&app.system).await?)
}

/// Test support behind `/internal/seed`: insert fixture rows.
pub async fn seed(app: &App, fixture: &SeedFixture) -> Result<(), CoreError> {
    app.ports.db.seed(&app.system, fixture).await?;
    auth::seed_identities(app, fixture).await?;
    // The fixture carries plaintext tokens; the database only ever sees their hashes.
    let now = app.ports.clock.now();
    for s in &fixture.sessions {
        let session = NewSession {
            token_hash: hash_token(&s.token),
            user_id: s.user_id,
            created_at: now,
            expires_at: now + TimeDelta::seconds(app.cfg.session_ttl_secs),
        };
        app.ports.db.insert_session(&session).await?;
    }
    profile::seed_profiles(app, fixture).await?;
    Ok(())
}

/// Test support behind `/internal/reset` (issue #58): empty the database, so that a fixture can be loaded into what it was when
/// it was made. Only the database: the rate limiter's counters, the blob store and the queue are not touched, and a window that
/// has begun runs out. The route that reaches this is the one that `seed` is behind, and is as dangerous as it sounds.
pub async fn reset(app: &App) -> Result<(), CoreError> {
    app.ports.db.reset(&app.system).await?;
    Ok(())
}

/// Test and bench support behind `/internal/seed-work-items`: insert the request's items as live work items of one
/// project, in bulk, numbered from its counter.
pub async fn seed_work_items(app: &App, req: &SeedWorkItemsRequest) -> Result<SeedWorkItemsReport, CoreError> {
    if req.items.is_empty() {
        return Err(CoreError::Invalid("no items".into()));
    }
    let now = app.ports.clock.now();
    let items: Vec<SeedWorkItem> = req
        .items
        .iter()
        .map(|i| SeedWorkItem {
            id: WorkItemId(app.ports.ids.new_uuid()),
            name: i.name.clone(),
            description: i.description.clone(),
            created_by: req.created_by,
            created_at: now,
        })
        .collect();
    let first = app.ports.db.seed_work_items(&app.system, req.workspace_id, req.project_id, &items).await?;
    Ok(SeedWorkItemsReport { first_sequence_id: first, count: items.len() as u64 })
}

/// Queue consumer body. Idempotent on `event_id`: a redelivery inserts nothing.
pub async fn process_event(app: &App, env: &Envelope) -> Result<bool, CoreError> {
    let Some(project_id) = env.project_id() else {
        return Err(CoreError::Invalid("event without project_id".into()));
    };
    let verb = env.event_type.rsplit('.').next().unwrap_or(env.event_type.as_str()).to_string();
    let a = Activity {
        id: ActivityId(app.ports.ids.new_uuid()),
        event_id: env.event_id,
        workspace_id: env.workspace_id,
        project_id,
        entity_id: env.entity_id,
        verb,
        actor_id: env.actor_id,
        created_at: app.ports.clock.now(),
    };
    let inserted = app.ports.db.insert_activity(&app.system, &a).await?;
    // The history rows an event names are written whether or not the event was seen before: the write is idempotent on the event and
    // the place of the row in it, and an event whose first delivery failed half way must still finish.
    issues::project_history(app, env).await?;
    Ok(inserted)
}

// --- attachments (BlobStore) ----------------------------------------------------------------

/// An attachment name is one ordinary path segment, so the blob key cannot leave its
/// `<workspace>/<project>/<item>/` prefix. `.` is a dot segment too: RFC 3986 clients normalise
/// `.../attachments/.` away, so an object stored under it could not be fetched through the route.
fn validate_blob_name(name: &str) -> Result<(), CoreError> {
    if name.is_empty() || name.len() > 128 || name.contains('/') || name.contains("..") || name == "." {
        return Err(CoreError::Invalid("bad attachment name".into()));
    }
    Ok(())
}

fn blob_key(scope: &Scope, item: WorkItemId, name: &str) -> String {
    format!("{}/{}/{}/{}", scope.workspace_id(), scope.project_id(), item, name)
}

#[allow(
    clippy::too_many_arguments,
    reason = "a use-case entry point takes who is asking, the route (slug, project, item) and the upload (name, bytes, type) as \
              separate arguments, like its siblings; a parameter struct would only rename them"
)]
pub async fn put_attachment(
    app: &App,
    who: &Principal,
    slug: &WorkspaceSlug,
    project: ProjectId,
    item: WorkItemId,
    name: &str,
    bytes: Vec<u8>,
    content_type: Option<String>,
) -> Result<(), CoreError> {
    validate_blob_name(name)?;
    if bytes.len() > app.cfg.max_attachment_bytes {
        return Err(CoreError::TooLarge);
    }
    let scope = authorize(app, who, slug, project, Permission::WriteAttachment).await?;
    app.ports.db.get_work_item(&scope, item, Visibility::Visible).await?.ok_or(CoreError::NotFound)?;
    app.ports.blobs.put(&blob_key(&scope, item, name), bytes, content_type).await?;
    Ok(())
}

pub async fn get_attachment(
    app: &App,
    who: &Principal,
    slug: &WorkspaceSlug,
    project: ProjectId,
    item: WorkItemId,
    name: &str,
) -> Result<Blob, CoreError> {
    validate_blob_name(name)?;
    let scope = authorize(app, who, slug, project, Permission::ReadAttachment).await?;
    app.ports.blobs.get(&blob_key(&scope, item, name)).await?.ok_or(CoreError::NotFound)
}

pub async fn delete_attachment(
    app: &App,
    who: &Principal,
    slug: &WorkspaceSlug,
    project: ProjectId,
    item: WorkItemId,
    name: &str,
) -> Result<(), CoreError> {
    validate_blob_name(name)?;
    let scope = authorize(app, who, slug, project, Permission::WriteAttachment).await?;
    app.ports.blobs.delete(&blob_key(&scope, item, name)).await?;
    Ok(())
}

/// The queue consumer for the `events` topic: projects every event into an activity row.
#[derive(Clone)]
pub struct ActivityConsumer(pub App);

#[async_trait::async_trait]
impl EventConsumer for ActivityConsumer {
    async fn handle(&self, env: &Envelope) -> Result<(), String> {
        process_event(&self.0, env).await.map(|_| ()).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
pub(crate) mod test_support;

// Unit tests for the private validators. The use-cases are tested through the public API, against the
// memory adapters, in `tests/`.
#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use rstest::rstest;

    #[rstest]
    #[case::empty("", false)]
    #[case::only_whitespace(" \t\n ", false)]
    #[case::one_char("a", true)]
    #[case::padded(" a ", true)]
    #[case::ascii_255("x".repeat(255), true)]
    #[case::ascii_256("x".repeat(256), false)]
    // the limit counts characters, not bytes: 255 two-byte characters are 510 bytes and still valid
    #[case::multibyte_255("é".repeat(255), true)]
    #[case::multibyte_256("é".repeat(256), false)]
    // padding is trimmed before counting
    #[case::padded_255(format!("  {}  ", "x".repeat(255)), true)]
    fn validate_name_cases(#[case] name: impl AsRef<str>, #[case] valid: bool) {
        assert_eq!(validate_name(name.as_ref()).is_ok(), valid);
    }

    #[rstest]
    #[case::plain("report.pdf", true)]
    #[case::one_char("a", true)]
    #[case::inner_dot("a.b", true)]
    #[case::empty("", false)]
    #[case::slash("a/b", false)]
    #[case::leading_slash("/etc", false)]
    #[case::dot_dot("..", false)]
    #[case::dot_dot_inside("a..b", false)]
    // found by prop_accepted_blob_names_stay_inside_their_scope (proptest-regressions/lib.txt)
    #[case::single_dot(".", false)]
    // the limit counts bytes: 64 two-byte characters are exactly 128 bytes
    #[case::bytes_128("é".repeat(64), true)]
    #[case::bytes_130("é".repeat(65), false)]
    #[case::ascii_128("x".repeat(128), true)]
    #[case::ascii_129("x".repeat(129), false)]
    fn validate_blob_name_cases(#[case] name: impl AsRef<str>, #[case] valid: bool) {
        assert_eq!(validate_blob_name(name.as_ref()).is_ok(), valid);
    }

    fn scope() -> Scope {
        Scope::mint(WorkspaceId(Uuid::from_u128(1)), ProjectId(Uuid::from_u128(2)), UserId(Uuid::from_u128(3)))
    }

    /// Names built from mostly ordinary characters with the occasional `.` or multi-byte character, never `/`
    /// (the slash property below covers that), so most generated names are valid and few are rejected.
    /// The bare dot segments are drawn on their own as well, because a random string almost never is one.
    fn blob_name() -> impl Strategy<Value = String> {
        let general = prop::collection::vec(
            prop_oneof![8 => "[a-z0-9_-]", 1 => Just(".".to_string()), 1 => Just("\u{e9}".to_string())],
            1..=60,
        )
        .prop_map(|parts| parts.concat());
        prop_oneof![9 => general, 1 => prop::sample::select(vec![".".to_string(), "..".to_string()])]
    }

    /// Visible characters (multi-byte ones included, since the limit counts characters), with the two end lengths of
    /// the range drawn on their own as well: a uniform length almost never is one.
    fn non_whitespace(len: std::ops::RangeInclusive<usize>) -> impl Strategy<Value = String> {
        let (lo, hi) = (*len.start(), *len.end());
        prop_oneof![1 => Just(lo), 1 => Just(hi), 8 => len].prop_flat_map(|n| {
            prop::collection::vec(any::<char>().prop_filter("not whitespace", |c| !c.is_whitespace()), n)
                .prop_map(|cs| cs.into_iter().collect::<String>())
        })
    }

    /// Any printable text, plus the cases a uniform draw almost never makes: empty, only whitespace, and names right at
    /// the 255-character limit and one past it.
    fn any_name() -> impl Strategy<Value = String> {
        prop_oneof![
            6 => "\\PC{0,300}",
            1 => Just(String::new()),
            1 => "[ \t\n]{1,6}",
            1 => Just("x".repeat(255)),
            1 => Just("\u{e9}".repeat(256)),
        ]
    }

    /// One side of a slash: empty (a leading or trailing slash) or a word.
    fn segment() -> impl Strategy<Value = String> {
        prop_oneof![1 => Just(String::new()), 4 => "[a-z]{1,60}"]
    }

    proptest! {
        #![proptest_config(crate::test_support::proptest_config(256))]

        /// Surrounding whitespace never changes whether a name is accepted.
        #[test]
        fn prop_name_validity_ignores_surrounding_whitespace(name in any_name(), pad in "[ \t\n]{0,4}") {
            let padded = format!("{pad}{name}{pad}");
            prop_assert_eq!(validate_name(&padded).is_ok(), validate_name(&name).is_ok());
        }

        #[test]
        fn prop_names_of_1_to_255_visible_chars_are_accepted(name in non_whitespace(1..=255)) {
            prop_assert!(validate_name(&name).is_ok());
        }

        #[test]
        fn prop_names_longer_than_255_chars_are_rejected(name in non_whitespace(256..=320)) {
            prop_assert!(validate_name(&name).is_err());
        }

        /// The tenant boundary for attachments: an accepted name is exactly one path segment, so the blob
        /// key stays under `<workspace>/<project>/<item>/` and cannot reach another tenant's objects.
        #[test]
        fn prop_accepted_blob_names_stay_inside_their_scope(name in blob_name()) {
            prop_assume!(validate_blob_name(&name).is_ok());
            let s = scope();
            let item = WorkItemId(Uuid::from_u128(4));
            let key = blob_key(&s, item, &name);
            let segments: Vec<&str> = key.split('/').collect();
            prop_assert_eq!(segments.len(), 4, "key {:?} has extra segments", key);
            let want = [s.workspace_id().to_string(), s.project_id().to_string(), item.to_string()];
            prop_assert_eq!(&segments[..3], &want[..]);
            prop_assert_eq!(segments[3], name.as_str());
            prop_assert!(segments.iter().all(|seg| *seg != ".." && *seg != "."));
        }

        /// Any name containing a slash is rejected, wherever the slash is.
        #[test]
        fn prop_blob_names_with_a_slash_are_rejected(before in segment(), after in segment()) {
            let name = format!("{before}/{after}");
            prop_assert!(validate_blob_name(&name).is_err());
        }
    }

    /// Rule 8 for the properties above: each strategy reaches the inputs its property is about within one run's
    /// budget (`CASES`, the properties' own default), and `prop_accepted_blob_names_stay_inside_their_scope` (which
    /// discards rejected names) is not mostly discarding.
    #[test]
    fn strategies_reach_every_input_class() {
        use proptest::strategy::ValueTree;
        use proptest::test_runner::TestRunner;
        use std::collections::BTreeSet;

        const CASES: usize = 256;
        fn sample<S: Strategy>(s: S) -> Vec<S::Value> {
            let mut runner = TestRunner::deterministic();
            (0..CASES).map(|_| s.new_tree(&mut runner).unwrap().current()).collect()
        }
        let mut seen = BTreeSet::new();
        let mut note = |class: &'static str, hit: bool| {
            if hit {
                seen.insert(class);
            }
        };

        for name in sample(any_name()) {
            let trimmed = name.trim().chars().count();
            note("name: empty", name.is_empty());
            note("name: only whitespace", !name.is_empty() && name.trim().is_empty());
            note("name: valid", (1..=255).contains(&trimmed));
            note("name: exactly 255", trimmed == 255);
            note("name: over 255", trimmed > 255);
        }
        for name in sample(non_whitespace(1..=255)) {
            let n = name.chars().count();
            note("visible: 1", n == 1);
            note("visible: 255", n == 255);
            note("visible: multi-byte", name.len() > n);
        }
        for name in sample(non_whitespace(256..=320)) {
            note("visible: 256", name.chars().count() == 256);
        }
        let blobs = sample(blob_name());
        let accepted: Vec<&String> = blobs.iter().filter(|b| validate_blob_name(b).is_ok()).collect();
        for b in &blobs {
            note("blob: bare dot", b == ".");
            note("blob: dot dot", b == "..");
            note("blob: rejected for ..", b.contains("..") && b != "..");
        }
        for b in &accepted {
            note("blob accepted: one character", b.len() == 1);
            note("blob accepted: with a dot", b.contains('.'));
            note("blob accepted: multi-byte", b.chars().count() < b.len());
        }
        for (before, after) in sample((segment(), segment())) {
            note("slash: leading", before.is_empty());
            note("slash: trailing", after.is_empty());
            note("slash: inside", !before.is_empty() && !after.is_empty());
        }

        let all = [
            "name: empty",
            "name: only whitespace",
            "name: valid",
            "name: exactly 255",
            "name: over 255",
            "visible: 1",
            "visible: 255",
            "visible: multi-byte",
            "visible: 256",
            "blob: bare dot",
            "blob: dot dot",
            "blob: rejected for ..",
            "blob accepted: one character",
            "blob accepted: with a dot",
            "blob accepted: multi-byte",
            "slash: leading",
            "slash: trailing",
            "slash: inside",
        ];
        let missing: Vec<_> = all.iter().filter(|c| !seen.contains(*c)).collect();
        assert!(missing.is_empty(), "never generated: {missing:?}");
        assert!(accepted.len() * 2 > blobs.len(), "only {} of {} blob names are accepted", accepted.len(), blobs.len());
    }
}
