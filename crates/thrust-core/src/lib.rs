//! Application core: the permission seam and the use-cases of the vertical slice.
//!
//! Nothing in here knows whether it runs in a Worker or in a native server. The only inputs are
//! the port trait objects in [`Ports`].

use std::sync::Arc;

use thiserror::Error;
use thrust_contracts::*;
use thrust_ports::*;

// ---------------------------------------------------------------------------------------------
// Wiring
// ---------------------------------------------------------------------------------------------

#[derive(Clone)]
pub struct Ports {
    pub db: Arc<dyn Database>,
    pub queue: Arc<dyn Queue>,
    pub sessions: Arc<dyn SessionStore>,
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
}

impl Default for CoreConfig {
    fn default() -> Self {
        Self {
            create_rate_limit: 1000,
            rate_window_secs: 60,
            outbox_fastpath: true,
            max_attachment_bytes: 5 * 1024 * 1024,
        }
    }
}

#[derive(Clone)]
pub struct App {
    pub ports: Ports,
    pub cfg: CoreConfig,
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
impl From<CacheError> for CoreError {
    fn from(e: CacheError) -> Self {
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

/// Proof that `user` may act in `(workspace, project)`. Fields are private: the only way to get
/// one is [`authorize`], so repository calls that take a `Scope` cannot be reached unchecked.
#[derive(Debug, Clone, Copy)]
pub struct Scope {
    workspace_id: WorkspaceId,
    project_id: ProjectId,
    user_id: UserId,
}

impl Scope {
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

/// 404 for a project that is not in the caller's workspace (cross-tenant probes learn nothing),
/// 403 for a project the caller can see exists but may not touch.
pub async fn authorize(
    app: &App,
    who: &Principal,
    project: ProjectId,
    perm: Permission,
) -> Result<Scope, CoreError> {
    match app.ports.db.membership(who.workspace_id, project, who.user_id).await? {
        Membership::NoSuchProject => Err(CoreError::NotFound),
        Membership::NotAMember => Err(CoreError::Forbidden),
        Membership::Member(role) if role >= perm.min_role() => Ok(Scope {
            workspace_id: who.workspace_id,
            project_id: project,
            user_id: who.user_id,
        }),
        Membership::Member(_) => Err(CoreError::Forbidden),
    }
}

pub async fn authenticate(app: &App, token: &str) -> Result<Principal, CoreError> {
    app.ports
        .sessions
        .get(token)
        .await?
        .ok_or(CoreError::Unauthenticated)
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

    let scope = authorize(app, who, project, Permission::CreateWorkItem).await?;

    let mut tx = app.ports.db.begin().await?;
    let work = async {
        let alloc = tx.allocate(scope.workspace_id(), scope.project_id()).await?;
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
        tx.insert_work_item(&item).await?;
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
        tx.insert_outbox(&row).await?;
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
    project: ProjectId,
    id: WorkItemId,
) -> Result<(), CoreError> {
    let scope = authorize(app, who, project, Permission::DeleteWorkItem).await?;
    let mut tx = app.ports.db.begin().await?;
    let now = app.ports.clock.now();
    let work = async {
        let deleted = tx
            .soft_delete_work_item(scope.workspace_id(), scope.project_id(), id, now)
            .await?;
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
        tx.insert_outbox(&row).await?;
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
    project: ProjectId,
    vis: Visibility,
) -> Result<Vec<WorkItem>, CoreError> {
    let perm = match vis {
        Visibility::Visible => Permission::ReadWorkItems,
        Visibility::WithDeleted => Permission::ReadDeleted,
    };
    let scope = authorize(app, who, project, perm).await?;
    Ok(app
        .ports
        .db
        .list_work_items(scope.workspace_id(), scope.project_id(), vis)
        .await?)
}

pub async fn list_activity(
    app: &App,
    who: &Principal,
    project: ProjectId,
    entity: Option<WorkItemId>,
) -> Result<Vec<Activity>, CoreError> {
    let scope = authorize(app, who, project, Permission::ReadWorkItems).await?;
    Ok(app
        .ports
        .db
        .list_activity(scope.workspace_id(), scope.project_id(), entity)
        .await?)
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
        .mark_outbox_dispatched(row.envelope.event_id, app.ports.clock.now())
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
    let rows = app.ports.db.undispatched_outbox(limit).await?;
    let mut rep = SweepReport { found: rows.len(), published: 0, failed: 0 };
    for row in &rows {
        match dispatch(app, row).await {
            Ok(()) => rep.published += 1,
            Err(_) => rep.failed += 1,
        }
    }
    Ok(rep)
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
    Ok(app.ports.db.insert_activity(&a).await?)
}

// --- attachments (BlobStore) ----------------------------------------------------------------

fn validate_blob_name(name: &str) -> Result<(), CoreError> {
    if name.is_empty() || name.len() > 128 || name.contains('/') || name.contains("..") {
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
    let scope = authorize(app, who, project, Permission::WriteAttachment).await?;
    app.ports
        .db
        .get_work_item(scope.workspace_id(), scope.project_id(), item, Visibility::Visible)
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
    project: ProjectId,
    item: WorkItemId,
    name: &str,
) -> Result<Blob, CoreError> {
    validate_blob_name(name)?;
    let scope = authorize(app, who, project, Permission::ReadAttachment).await?;
    app.ports
        .blobs
        .get(&blob_key(&scope, item, name))
        .await?
        .ok_or(CoreError::NotFound)
}

pub async fn delete_attachment(
    app: &App,
    who: &Principal,
    project: ProjectId,
    item: WorkItemId,
    name: &str,
) -> Result<(), CoreError> {
    validate_blob_name(name)?;
    let scope = authorize(app, who, project, Permission::WriteAttachment).await?;
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
