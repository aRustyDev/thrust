//! Application core: the permission seam and the use-cases of the vertical slice.
//!
//! Nothing in here knows whether it runs in a Worker or in a native server. The only inputs are
//! the port trait objects in [`Ports`].

use std::sync::Arc;

use sha2::{Digest, Sha256};
use thiserror::Error;
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
}

#[derive(Debug, Clone)]
pub struct CoreConfig {
    /// Per-user create limit inside `rate_window_secs`.
    pub create_rate_limit: u32,
    pub rate_window_secs: u64,
    /// ADR-0046 relay: publish right after commit (true) or leave every row to the cron sweeper.
    pub outbox_fastpath: bool,
    pub max_attachment_bytes: usize,
    /// How long a session lives from its creation (ADR-0052: 30 days).
    pub session_ttl_secs: i64,
    /// `authenticate` rewrites a session's `last_seen_at` only when it is at least this old, so a busy
    /// session costs one write an hour, not one per request (ADR-0052 round 29: updated lazily).
    pub session_touch_after_secs: i64,
}

impl Default for CoreConfig {
    fn default() -> Self {
        Self {
            create_rate_limit: 1000,
            rate_window_secs: 60,
            outbox_fastpath: true,
            max_attachment_bytes: 5 * 1024 * 1024,
            session_ttl_secs: 30 * 24 * 60 * 60,
            session_touch_after_secs: 60 * 60,
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
    ReadWorkItems,
    ReadDeleted,
    CreateWorkItem,
    DeleteWorkItem,
    ReadAttachment,
    WriteAttachment,
}

impl Permission {
    fn min_role(self) -> Role {
        match self {
            Permission::ReadWorkItems | Permission::ReadAttachment => Role::Guest,
            Permission::CreateWorkItem | Permission::DeleteWorkItem | Permission::WriteAttachment => {
                Role::Member
            }
            Permission::ReadDeleted => Role::Admin,
        }
    }
}

/// The proof types are declared in `thrust-ports`, so that the ports can take them (issue #35).
/// [`authorize`] is the only place a [`Scope`] is minted.
pub use thrust_ports::{Scope, SystemScope};

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
/// and cannot reach a repository with bare ids:
///
/// ```compile_fail
/// use thrust_contracts::*;
/// use thrust_core::*;
/// async fn use_case(app: &App, ws: WorkspaceId, project: ProjectId) {
///     let _ = app.ports.db.list_work_items(ws, project, Visibility::Visible).await;
/// }
/// ```
///
/// Control for both: the same imports and the same call, through `authorize`, compile. (The one
/// remaining path, the `pub` constructor `Scope::mint`, is closed by `scripts/check-deps.sh`.)
///
/// ```
/// use thrust_contracts::*;
/// use thrust_core::*;
/// async fn use_case(
///     app: &App,
///     who: &Principal,
///     slug: &WorkspaceSlug,
///     project: ProjectId,
/// ) -> Result<Vec<WorkItem>, CoreError> {
///     let scope = authorize(app, who, slug, project, Permission::ReadWorkItems).await?;
///     let _ids = (scope.workspace_id(), scope.project_id(), UserId(Uuid::nil()));
///     Ok(app.ports.db.list_work_items(&scope, Visibility::Visible).await?)
/// }
/// ```
pub async fn authorize(
    app: &App,
    who: &Principal,
    slug: &WorkspaceSlug,
    project: ProjectId,
    perm: Permission,
) -> Result<Scope, CoreError> {
    match app.ports.db.membership(slug, project, who.user_id).await? {
        Membership::NoSuchProject => Err(CoreError::NotFound),
        Membership::NotAMember => Err(CoreError::Forbidden),
        Membership::Member { workspace_id, role } if role >= perm.min_role() => {
            Ok(Scope::mint(workspace_id, project, who.user_id))
        }
        Membership::Member { .. } => Err(CoreError::Forbidden),
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
    Ok(app.ports.db.revoke_session(&hash_token(token), app.ports.clock.now()).await?)
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

/// create work item = rate limit -> authz -> [tx: sequence allocation -> insert -> outbox row]
/// -> after-commit relay (queue publish) -> (consumer, elsewhere) writes the activity row.
pub async fn create_work_item(
    app: &App,
    who: &Principal,
    slug: &WorkspaceSlug,
    project: ProjectId,
    req: CreateWorkItemRequest,
) -> Result<WorkItem, CoreError> {
    validate_name(&req.name)?;

    let decision = app
        .ports
        .limiter
        .hit(
            &format!("create:{}", who.user_id),
            app.cfg.create_rate_limit,
            app.cfg.rate_window_secs,
        )
        .await?;
    if let RateDecision::Limited { retry_after_secs } = decision {
        return Err(CoreError::RateLimited { retry_after_secs });
    }

    let scope = authorize(app, who, slug, project, Permission::CreateWorkItem).await?;

    let mut tx = app.ports.db.begin(&scope).await?;
    let work = async {
        let alloc = tx.allocate(&scope).await?;
        let item = WorkItem {
            id: WorkItemId(app.ports.ids.new_uuid()),
            workspace_id: scope.workspace_id(),
            project_id: scope.project_id(),
            sequence_id: alloc.sequence_id,
            sort_order: alloc.sort_order,
            name: req.name.trim().to_string(),
            description: req.description.clone(),
            external_source: req.external_source.clone(),
            external_id: req.external_id.clone(),
            created_by: scope.user_id(),
            created_at: app.ports.clock.now(),
            archived_at: None,
            is_draft: false,
            deleted_at: None,
        };
        tx.insert_work_item(&scope, &item).await?;
        let row = event(
            app,
            item.workspace_id,
            scope.user_id(),
            "work_item.created",
            item.id,
            serde_json::json!({
                "project_id": item.project_id,
                "sequence_id": item.sequence_id,
                "changed_fields": ["name"],
            }),
        );
        tx.insert_outbox(&scope, &row).await?;
        Ok::<_, DbError>((item, row))
    }
    .await;

    let (item, row) = match work {
        Ok(v) => v,
        Err(e) => {
            // Roll back explicitly: a pooled connection must not be returned mid-transaction.
            let _ = tx.rollback().await;
            return Err(e.into());
        }
    };
    tx.commit().await?;

    relay_after_commit(app, row);
    Ok(item)
}

pub async fn delete_work_item(
    app: &App,
    who: &Principal,
    slug: &WorkspaceSlug,
    project: ProjectId,
    id: WorkItemId,
) -> Result<(), CoreError> {
    let scope = authorize(app, who, slug, project, Permission::DeleteWorkItem).await?;
    let mut tx = app.ports.db.begin(&scope).await?;
    let now = app.ports.clock.now();
    let work = async {
        let deleted = tx.soft_delete_work_item(&scope, id, now).await?;
        if !deleted {
            return Ok::<_, DbError>(None);
        }
        let row = event(
            app,
            scope.workspace_id(),
            scope.user_id(),
            "work_item.deleted",
            id,
            serde_json::json!({ "project_id": scope.project_id(), "changed_fields": ["deleted_at"] }),
        );
        tx.insert_outbox(&scope, &row).await?;
        Ok(Some(row))
    }
    .await;
    match work {
        Ok(Some(row)) => {
            tx.commit().await?;
            relay_after_commit(app, row);
            Ok(())
        }
        Ok(None) => {
            let _ = tx.rollback().await;
            Err(CoreError::NotFound)
        }
        Err(e) => {
            let _ = tx.rollback().await;
            Err(e.into())
        }
    }
}

pub async fn list_work_items(
    app: &App,
    who: &Principal,
    slug: &WorkspaceSlug,
    project: ProjectId,
    vis: Visibility,
) -> Result<Vec<WorkItem>, CoreError> {
    let perm = match vis {
        Visibility::Visible => Permission::ReadWorkItems,
        Visibility::WithDeleted => Permission::ReadDeleted,
    };
    let scope = authorize(app, who, slug, project, perm).await?;
    Ok(app.ports.db.list_work_items(&scope, vis).await?)
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
    app.ports
        .db
        .mark_outbox_dispatched(&app.system, row.envelope.event_id, app.ports.clock.now())
        .await?;
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
    Ok(())
}

/// Queue consumer body. Idempotent on `event_id`: a redelivery inserts nothing.
pub async fn process_event(app: &App, env: &Envelope) -> Result<bool, CoreError> {
    let Some(project_id) = env.project_id() else {
        return Err(CoreError::Invalid("event without project_id".into()));
    };
    let verb = env
        .event_type
        .rsplit('.')
        .next()
        .unwrap_or(env.event_type.as_str())
        .to_string();
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
    Ok(app.ports.db.insert_activity(&app.system, &a).await?)
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
    app.ports
        .db
        .get_work_item(&scope, item, Visibility::Visible)
        .await?
        .ok_or(CoreError::NotFound)?;
    app.ports
        .blobs
        .put(&blob_key(&scope, item, name), bytes, content_type)
        .await?;
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
    app.ports
        .blobs
        .get(&blob_key(&scope, item, name))
        .await?
        .ok_or(CoreError::NotFound)
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
        Scope::mint(
            WorkspaceId(Uuid::from_u128(1)),
            ProjectId(Uuid::from_u128(2)),
            UserId(Uuid::from_u128(3)),
        )
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
            "name: empty", "name: only whitespace", "name: valid", "name: exactly 255", "name: over 255", "visible: 1",
            "visible: 255", "visible: multi-byte", "visible: 256", "blob: bare dot", "blob: dot dot",
            "blob: rejected for ..", "blob accepted: one character", "blob accepted: with a dot",
            "blob accepted: multi-byte", "slash: leading", "slash: trailing", "slash: inside",
        ];
        let missing: Vec<_> = all.iter().filter(|c| !seen.contains(*c)).collect();
        assert!(missing.is_empty(), "never generated: {missing:?}");
        assert!(accepted.len() * 2 > blobs.len(), "only {} of {} blob names are accepted", accepted.len(), blobs.len());
    }
}
