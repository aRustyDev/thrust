//! Postgres adapter for the `Database` port, written once against `tokio_postgres::Client`.
//!
//! The only platform-specific part is *how a connected `Client` is obtained*, and that is the
//! [`Connector`] trait:
//!   * native  -> `thrust-adapters-selfhosted`: deadpool-postgres pool over TCP
//!   * Worker  -> `thrust-adapters-cloudflare`: Hyperdrive `Socket` + `Config::connect_raw`
//!
//! Transactions use plain `BEGIN`/`COMMIT` on a connection the transaction owns, so a `Tx` is a
//! self-contained `Send` value (no borrowed `tokio_postgres::Transaction<'_>`).
//!
//! FR-22 (ADR-0043 round 29): the sequence number is taken in its own short transaction, BEFORE the create's. A `Tx`
//! checks its connection out at `begin` but opens no transaction until its first write, so `allocate` (the counter
//! upsert, one statement in autocommit) holds the counter row's lock for one round trip, not for the insert, the outbox
//! row and the commit as when the upsert was the first statement of the transaction. The price is a gap: a number taken
//! by a create that then fails or is abandoned is not given back.

/// TLS for a native connection (`verify-ca`) and a `connect` that never goes on without it when it was asked for. Only `migrate` uses it.
#[cfg(feature = "native")]
pub mod tls;

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use thrust_contracts::*;
use thrust_ports::*;
use tokio_postgres::error::SqlState;
use tokio_postgres::types::{ToSql, Type};
use tokio_postgres::{Client, Row};

pub const MIGRATION_0001: &str = include_str!("../migrations/0001_init.sql");
pub const MIGRATION_0002: &str = include_str!("../migrations/0002_sequence_tenant_key.sql");
pub const MIGRATION_0003: &str = include_str!("../migrations/0003_workspaces.sql");
pub const MIGRATION_0004: &str = include_str!("../migrations/0004_sessions.sql");
pub const MIGRATION_0005: &str = include_str!("../migrations/0005_identity.sql");
pub const MIGRATION_0006: &str = include_str!("../migrations/0006_search.sql");
pub const MIGRATION_0007: &str = include_str!("../migrations/0007_api_key_hash_version.sql");
pub const MIGRATION_0008: &str = include_str!("../migrations/0008_user_profiles.sql");
pub const MIGRATION_0009: &str = include_str!("../migrations/0009_workspace_details.sql");
pub const MIGRATION_0010: &str = include_str!("../migrations/0010_project_details.sql");
pub const MIGRATION_0011: &str = include_str!("../migrations/0011_workspace_user_settings.sql");
pub const MIGRATION_0012: &str = include_str!("../migrations/0012_projects.sql");
pub const MIGRATION_0013: &str = include_str!("../migrations/0013_work_item_details.sql");
pub const MIGRATION_0014: &str = include_str!("../migrations/0014_work_item_history.sql");
pub const MIGRATION_0015: &str = include_str!("../migrations/0015_v1_surface.sql");
pub const MIGRATION_0016: &str = include_str!("../migrations/0016_assets.sql");

pub async fn migrate(client: &Client) -> Result<(), tokio_postgres::Error> {
    client.batch_execute(MIGRATION_0001).await?;
    client.batch_execute(MIGRATION_0002).await?;
    client.batch_execute(MIGRATION_0003).await?;
    client.batch_execute(MIGRATION_0004).await?;
    client.batch_execute(MIGRATION_0005).await?;
    client.batch_execute(MIGRATION_0006).await?;
    client.batch_execute(MIGRATION_0007).await?;
    client.batch_execute(MIGRATION_0008).await?;
    client.batch_execute(MIGRATION_0009).await?;
    client.batch_execute(MIGRATION_0010).await?;
    client.batch_execute(MIGRATION_0011).await?;
    client.batch_execute(MIGRATION_0012).await?;
    client.batch_execute(MIGRATION_0013).await?;
    client.batch_execute(MIGRATION_0014).await?;
    client.batch_execute(MIGRATION_0015).await?;
    client.batch_execute(MIGRATION_0016).await
}

pub trait ClientHandle: Send {
    fn client(&self) -> &Client;
    /// Where the statements sent through this handle are counted, if anywhere. Only the handles of a
    /// [`CountingConnector`] say; the pool's and Hyperdrive's do not, and need no change.
    fn round_trips(&self) -> Option<&RoundTrips> {
        None
    }
    /// The way to send a statement: every statement of this adapter goes through the [`Pg`] this returns.
    fn pg(&self) -> Pg<'_> {
        Pg { client: self.client(), rt: self.round_trips() }
    }
    /// A transaction was abandoned without COMMIT/ROLLBACK: the connection must not be reused.
    fn discard(self: Box<Self>) {}
}

impl ClientHandle for Arc<Client> {
    fn client(&self) -> &Client {
        self
    }
}

#[async_trait]
pub trait Connector: Send + Sync {
    /// `exclusive == true` -> a connection no other statement will touch until it is dropped
    /// (used by transactions). `false` -> may be shared.
    async fn checkout(&self, exclusive: bool) -> Result<Box<dyn ClientHandle>, DbError>;
}

/// A connector that counts. Its handles are those of `inner`, with their statements added to one [`RoundTrips`]. A request
/// makes its own, so the count is that request's, on a process that serves many (the native server) as on an isolate that
/// builds the connector per request (the Worker). It sits between [`PgDatabase`] and the real connector, which is the one
/// place every checkout passes: `begin` and the second connection of `allocate` included, which `conn()` alone would miss.
pub struct CountingConnector {
    inner: Arc<dyn Connector>,
    rt: RoundTrips,
}

impl CountingConnector {
    pub fn new(inner: Arc<dyn Connector>, rt: RoundTrips) -> Self {
        Self { inner, rt }
    }
}

#[async_trait]
impl Connector for CountingConnector {
    async fn checkout(&self, exclusive: bool) -> Result<Box<dyn ClientHandle>, DbError> {
        let inner = self.inner.checkout(exclusive).await?;
        Ok(Box::new(Counted { inner, rt: self.rt.clone() }))
    }
}

/// A checked-out handle with a counter on it. `discard` has to reach the inner handle: its own `discard` is what keeps a
/// connection that is still inside a transaction out of the pool, and the trait's default would put it back.
struct Counted {
    inner: Box<dyn ClientHandle>,
    rt: RoundTrips,
}

impl ClientHandle for Counted {
    fn client(&self) -> &Client {
        self.inner.client()
    }
    fn round_trips(&self) -> Option<&RoundTrips> {
        Some(&self.rt)
    }
    fn discard(self: Box<Self>) {
        let Counted { inner, .. } = *self;
        inner.discard();
    }
}

fn db_err(e: tokio_postgres::Error) -> DbError {
    if e.code() == Some(&SqlState::UNIQUE_VIOLATION) {
        let c = e.as_db_error().and_then(|d| d.constraint()).unwrap_or("?");
        return DbError::Conflict(format!("unique constraint '{c}' violated"));
    }
    // `Display` of a database error is just "db error": the server's own words say which statement and constraint
    DbError::Backend(match e.as_db_error() {
        Some(d) => format!(
            "{} ({}{})",
            d.message(),
            d.code().code(),
            d.constraint().map(|c| format!(", constraint {c}")).unwrap_or_default()
        ),
        None => e.to_string(),
    })
}

/// The columns of an asset, in the order `asset_row` reads them by name.
const ASSET_COLS: &str =
    "id, workspace_id, created_by, entity_type, project_id, name, content_type, size, object_key, state, \
    upload_token_hash, upload_expires_at, created_at, uploaded_at, deleted_at";

/// An asset from a row of [`ASSET_COLS`]. A value the CHECKs would have refused is a backend error, not a panic.
fn asset_row(r: &Row) -> Result<AssetRow, DbError> {
    let entity_text: String = r.get("entity_type");
    let entity = AssetEntity::parse(&entity_text)
        .ok_or_else(|| DbError::Backend(format!("assets.entity_type '{entity_text}' is not one this build knows")))?;
    let state_text: String = r.get("state");
    let state = match state_text.as_str() {
        "pending" => AssetState::Pending,
        "uploaded" => AssetState::Uploaded,
        other => return Err(DbError::Backend(format!("assets.state '{other}' is not one this build knows"))),
    };
    let token: Option<Vec<u8>> = r.get("upload_token_hash");
    let expires: Option<Timestamp> = r.get("upload_expires_at");
    let upload = match (token, expires) {
        (Some(t), Some(expires_at)) => {
            let hash: [u8; 32] =
                t.try_into().map_err(|_| DbError::Backend("assets.upload_token_hash is not 32 bytes".into()))?;
            Some(UploadGrant { token_hash: TokenHash(hash), expires_at })
        }
        _ => None,
    };
    Ok(AssetRow {
        id: AssetId(r.get("id")),
        workspace_id: WorkspaceId(r.get("workspace_id")),
        created_by: UserId(r.get("created_by")),
        entity,
        project_id: r.get::<_, Option<Uuid>>("project_id").map(ProjectId),
        name: r.get("name"),
        content_type: r.get("content_type"),
        size: r.get("size"),
        object_key: r.get("object_key"),
        state,
        upload,
        created_at: r.get("created_at"),
        uploaded_at: r.get("uploaded_at"),
        deleted_at: r.get("deleted_at"),
    })
}

// ---------------------------------------------------------------------------------------------
// Statements run as ONE pipeline on the UNNAMED statement
// ---------------------------------------------------------------------------------------------
//
// tokio-postgres' default `query`/`execute(&str, ..)` PREPARES a named statement (`Parse`,
// `Describe`, `Sync`) and then `Bind`/`Execute`s it in a second round trip. Through Hyperdrive that
// flow was unreliable in Stage 2 (research/0004 §10): statements with no result columns (INSERT or
// UPDATE without RETURNING) failed every time (`connection closed`) and SELECTs failed intermittently
// (`unexpected message from server`). The `*_typed` calls send `Parse`, `Bind`, `Describe`, `Execute`,
// `Sync` as one pipeline on the unnamed statement; that passed every time, in one round trip instead
// of two. The price: no server-side statement caching, and every parameter must carry its Postgres
// type, which is what `PgParam` is for.

/// The Postgres type of a bind parameter, known statically so `Option::<T>::None` still has one.
pub trait PgType {
    const TY: Type;
}

macro_rules! pg_type {
    ($($t:ty => $ty:expr),* $(,)?) => { $(impl PgType for $t { const TY: Type = $ty; })* };
}
pg_type!(
    Uuid => Type::UUID,
    i64 => Type::INT8,
    i32 => Type::INT4,
    // `api_keys.hash_version` is a smallint
    i16 => Type::INT2,
    bool => Type::BOOL,
    String => Type::TEXT,
    Timestamp => Type::TIMESTAMPTZ,
    chrono::NaiveDate => Type::DATE,
    serde_json::Value => Type::JSONB,
    f64 => Type::FLOAT8,
);
/// Whole columns at once, for the bulk seed's `unnest`.
impl PgType for Vec<Uuid> {
    const TY: Type = Type::UUID_ARRAY;
}
/// A column that may be null (`work_item_activities.actor_id`).
impl PgType for Vec<Option<Uuid>> {
    const TY: Type = Type::UUID_ARRAY;
}
impl PgType for Vec<i32> {
    const TY: Type = Type::INT4_ARRAY;
}
impl PgType for Vec<i64> {
    const TY: Type = Type::INT8_ARRAY;
}
impl PgType for Vec<String> {
    const TY: Type = Type::TEXT_ARRAY;
}
impl PgType for Vec<Option<String>> {
    const TY: Type = Type::TEXT_ARRAY;
}
impl PgType for Vec<&str> {
    const TY: Type = Type::TEXT_ARRAY;
}
impl PgType for Vec<bool> {
    const TY: Type = Type::BOOL_ARRAY;
}
impl PgType for Vec<f64> {
    const TY: Type = Type::FLOAT8_ARRAY;
}
impl PgType for Vec<serde_json::Value> {
    const TY: Type = Type::JSONB_ARRAY;
}
impl PgType for Vec<Timestamp> {
    const TY: Type = Type::TIMESTAMPTZ_ARRAY;
}
impl PgType for &str {
    const TY: Type = Type::TEXT;
}
/// A session token's hash (`sessions.token_hash`).
impl PgType for &[u8] {
    const TY: Type = Type::BYTEA;
}
impl<T: PgType> PgType for Option<T> {
    const TY: Type = T::TY;
}

/// Object-safe view of a bind parameter that can tell the server its type.
pub trait PgParam: ToSql + Sync {
    fn pg_type(&self) -> Type;
}
impl<T: PgType + ToSql + Sync> PgParam for T {
    fn pg_type(&self) -> Type {
        T::TY
    }
}

type Params<'a> = &'a [&'a dyn PgParam];

fn typed<'a>(p: Params<'a>) -> Vec<(&'a (dyn ToSql + Sync), Type)> {
    p.iter().map(|x| (*x as &(dyn ToSql + Sync), x.pg_type())).collect()
}

/// A count of the statements sent through [`Pg`], shared by every handle one [`CountingConnector`] gives out (issue #39).
///
/// The unit is a statement, and a statement is one wire round trip because no code in the adapter, the core or the API
/// crates puts two statements in flight on one client. It is not a count of connections: a Hyperdrive handshake is no
/// statement. And it does not wait for what a request leaves running after its answer (the background touch of an API
/// key or a session): that work is counted only if it runs before the count is read, so whoever needs a stable number
/// has to keep such work out of the request they measure.
#[derive(Clone, Debug, Default)]
pub struct RoundTrips(Arc<AtomicU32>);

impl RoundTrips {
    pub fn new() -> Self {
        Self::default()
    }

    /// The statements counted so far.
    pub fn get(&self) -> u32 {
        self.0.load(Ordering::Relaxed)
    }

    /// One statement is about to be sent. It counts when it is sent and not when it succeeds: a statement that fails or is
    /// cancelled still cost its round trip.
    fn tick(&self) {
        // MUTANT (control run only): with `mutant_count_nothing` the counter counts nothing, and the budget test must
        // notice. Nothing but `scripts/mutants.sh` builds this.
        #[cfg(not(feature = "mutant_count_nothing"))]
        self.0.fetch_add(1, Ordering::Relaxed);
    }
}

/// A connected client whose statements all go through the single-pipeline typed calls, and are counted when the handle it
/// came from keeps a [`RoundTrips`]. It is made by [`ClientHandle::pg`] and by nothing else (its fields are private), so a
/// statement cannot reach the client without passing the counter.
#[derive(Clone, Copy)]
pub struct Pg<'a> {
    client: &'a Client,
    rt: Option<&'a RoundTrips>,
}

impl Pg<'_> {
    fn sent(&self) {
        if let Some(rt) = self.rt {
            rt.tick();
        }
    }

    pub async fn query(&self, sql: &str, p: Params<'_>) -> Result<Vec<Row>, tokio_postgres::Error> {
        self.sent();
        self.client.query_typed(sql, &typed(p)).await
    }
    pub async fn query_opt(&self, sql: &str, p: Params<'_>) -> Result<Option<Row>, tokio_postgres::Error> {
        self.sent();
        self.client.query_typed_opt(sql, &typed(p)).await
    }
    pub async fn query_one(&self, sql: &str, p: Params<'_>) -> Result<Row, tokio_postgres::Error> {
        self.sent();
        self.client.query_typed_one(sql, &typed(p)).await
    }
    pub async fn execute(&self, sql: &str, p: Params<'_>) -> Result<u64, tokio_postgres::Error> {
        self.sent();
        self.client.execute_typed(sql, &typed(p)).await
    }
    /// Simple query protocol; no parameters (BEGIN / COMMIT / ROLLBACK).
    pub async fn batch_execute(&self, sql: &str) -> Result<(), tokio_postgres::Error> {
        self.sent();
        self.client.batch_execute(sql).await
    }
}

pub struct PgDatabase {
    connector: Arc<dyn Connector>,
}

impl PgDatabase {
    pub fn new(connector: Arc<dyn Connector>) -> Self {
        Self { connector }
    }

    async fn conn(&self) -> Result<Box<dyn ClientHandle>, DbError> {
        self.connector.checkout(false).await
    }
}

const WI_COLS: &str = "id, workspace_id, project_id, sequence_id, sort_order, name, description, \
    external_source, external_id, created_by, created_at, archived_at, is_draft, deleted_at";

fn work_item(r: &Row) -> WorkItem {
    WorkItem {
        id: WorkItemId(r.get("id")),
        workspace_id: WorkspaceId(r.get("workspace_id")),
        project_id: ProjectId(r.get("project_id")),
        sequence_id: r.get("sequence_id"),
        sort_order: r.get("sort_order"),
        name: r.get("name"),
        description: r.get("description"),
        external_source: r.get("external_source"),
        external_id: r.get("external_id"),
        created_by: r.get::<_, Option<Uuid>>("created_by").map(UserId),
        created_at: r.get("created_at"),
        archived_at: r.get("archived_at"),
        is_draft: r.get("is_draft"),
        deleted_at: r.get("deleted_at"),
    }
}

/// What a page or a single read of the compat work item selects, over `work_items w LEFT JOIN states st ON st.id = w.state_id`: the
/// columns of the item, the compat columns, the state's group, and the people and labels as arrays. A person counts only while they
/// are an active member of the workspace and in the project (the rows stay when they leave).
const WIR_COLS: &str = "w.id, w.workspace_id, w.project_id, w.sequence_id, w.sort_order, w.name, w.description, \
    w.external_source, w.external_id, w.created_by, w.created_at, w.archived_at, w.is_draft, w.deleted_at, \
    w.state_id, w.priority, w.start_date, w.target_date, w.completed_at, w.updated_at, w.updated_by, w.description_html, \
    w.parent_id, w.point, st.state_group AS st_group, \
    ARRAY(SELECT a.user_id FROM work_item_assignees a \
            JOIN project_members pm ON pm.project_id = a.project_id AND pm.workspace_id = a.workspace_id AND pm.user_id = a.user_id \
            JOIN workspace_members wm ON wm.workspace_id = a.workspace_id AND wm.user_id = a.user_id AND wm.is_active \
          WHERE a.work_item_id = w.id ORDER BY a.user_id) AS assignee_ids, \
    ARRAY(SELECT l.label_id FROM work_item_labels l WHERE l.work_item_id = w.id ORDER BY l.label_id) AS label_ids";

fn work_item_row(r: &Row) -> Result<WorkItemRow, DbError> {
    let priority: String = r.get("priority");
    let group: Option<String> = r.get("st_group");
    Ok(WorkItemRow {
        item: work_item(r),
        attrs: WorkItemAttrs {
            state_id: r.get("state_id"),
            priority: Priority::parse(&priority)
                .ok_or_else(|| DbError::Backend(format!("priority holds '{priority}', which is no priority")))?,
            start_date: r.get("start_date"),
            target_date: r.get("target_date"),
            completed_at: r.get("completed_at"),
            updated_at: r.get("updated_at"),
            updated_by: r.get::<_, Option<Uuid>>("updated_by").map(UserId),
            description_html: r.get("description_html"),
            assignee_ids: r.get::<_, Vec<Uuid>>("assignee_ids").into_iter().map(UserId).collect(),
            label_ids: r.get("label_ids"),
            parent_id: r.get("parent_id"),
            point: r.get("point"),
        },
        state_group: group
            .map(|g| {
                StateGroup::parse(&g)
                    .ok_or_else(|| DbError::Backend(format!("a state's group holds '{g}', which is no group")))
            })
            .transpose()?,
    })
}

/// The `ORDER BY` of a page, as [`WorkItemOrder`] states it (every piece is a fixed string: nothing a request sent is in the SQL).
/// Names sort as bytes (`COLLATE "C"`), as the memory adapter's strings do.
fn work_item_order_sql(o: WorkItemOrder) -> String {
    let dir = if o.descending { "DESC" } else { "ASC" };
    let newest = "w.created_at DESC, w.sequence_id DESC";
    match o.field {
        WorkItemOrderField::CreatedAt => format!("w.created_at {dir}, w.sequence_id {dir}"),
        WorkItemOrderField::UpdatedAt => format!("w.updated_at {dir}, w.sequence_id {dir}"),
        WorkItemOrderField::SequenceId => format!("w.sequence_id {dir}"),
        WorkItemOrderField::SortOrder => format!("w.sort_order {dir}, {newest}"),
        WorkItemOrderField::TargetDate => format!("w.target_date {dir} NULLS LAST, {newest}"),
        WorkItemOrderField::StartDate => format!("w.start_date {dir} NULLS LAST, {newest}"),
        WorkItemOrderField::CompletedAt => format!("w.completed_at {dir} NULLS LAST, {newest}"),
        WorkItemOrderField::ArchivedAt => format!("w.archived_at {dir} NULLS LAST, {newest}"),
        // the pin sorts on the inverse of the rank it computed: ascending runs from `none` to `urgent`
        WorkItemOrderField::Priority => format!(
            "CASE w.priority WHEN 'urgent' THEN 0 WHEN 'high' THEN 1 WHEN 'medium' THEN 2 WHEN 'low' THEN 3 ELSE 4 END {}, {newest}",
            if o.descending { "ASC" } else { "DESC" }
        ),
        WorkItemOrderField::StateName => format!("st.name COLLATE \"C\" {dir} NULLS LAST, {newest}"),
        // however it is asked, as the pin's own code has it
        WorkItemOrderField::StateGroup => format!(
            "CASE st.state_group WHEN 'backlog' THEN 0 WHEN 'unstarted' THEN 1 WHEN 'started' THEN 2 WHEN 'completed' THEN 3 WHEN 'cancelled' THEN 4 ELSE 5 END ASC, {newest}"
        ),
    }
}

/// `%query%` for `ILIKE ... ESCAPE '\'`, with the query's own `\`, `%` and `_` escaped so they match themselves.
fn like_pattern(query: &str) -> String {
    let mut p = String::with_capacity(query.len() + 2);
    p.push('%');
    for c in query.chars() {
        if matches!(c, '\\' | '%' | '_') {
            p.push('\\');
        }
        p.push(c);
    }
    p.push('%');
    p
}

const USER_COLS: &str = "id, email, password_hash, is_active";

fn user_record(r: &Row) -> UserRecord {
    UserRecord {
        id: UserId(r.get("id")),
        email: r.get("email"),
        password_hash: r.get("password_hash"),
        is_active: r.get("is_active"),
    }
}

const PROFILE_COLS: &str =
    "user_id, id, username, first_name, last_name, display_name, avatar, cover_image, timezone, \
    date_joined, last_login_at, is_onboarded, is_tour_completed, last_workspace_id, onboarding_step, preferences, \
    created_at, updated_at";

fn user_profile(r: &Row) -> UserProfile {
    UserProfile {
        id: r.get("id"),
        user_id: UserId(r.get("user_id")),
        username: r.get("username"),
        first_name: r.get("first_name"),
        last_name: r.get("last_name"),
        display_name: r.get("display_name"),
        avatar: r.get("avatar"),
        cover_image: r.get("cover_image"),
        timezone: r.get("timezone"),
        date_joined: r.get("date_joined"),
        last_login_at: r.get("last_login_at"),
        is_onboarded: r.get("is_onboarded"),
        is_tour_completed: r.get("is_tour_completed"),
        last_workspace_id: r.get::<_, Option<Uuid>>("last_workspace_id").map(WorkspaceId),
        onboarding_step: r.get("onboarding_step"),
        preferences: r.get("preferences"),
        created_at: r.get("created_at"),
        updated_at: r.get("updated_at"),
    }
}

/// A role as the `role` columns hold it. The CHECK constraints keep them to the three names, so anything else is a bug.
fn role_of(r: &Row, column: &str) -> Result<Role, DbError> {
    let s: String = r.get(column);
    Role::parse(&s).ok_or_else(|| DbError::Backend(format!("{column} holds '{s}', which is no role")))
}

/// The columns of `workspaces` the port's `Workspace` is made of, as `workspace_of` reads them (aliased `w`).
const WORKSPACE_COLS: &str = "w.id AS w_id, w.slug AS w_slug, w.name AS w_name, w.owner_id AS w_owner_id, \
    w.organization_size AS w_organization_size, w.timezone AS w_timezone, w.background_color AS w_background_color, \
    w.created_at AS w_created_at, w.updated_at AS w_updated_at";

fn workspace_of(r: &Row) -> Workspace {
    Workspace {
        id: WorkspaceId(r.get("w_id")),
        slug: WorkspaceSlug::new(r.get::<_, String>("w_slug")),
        name: r.get("w_name"),
        owner_id: r.get::<_, Option<Uuid>>("w_owner_id").map(UserId),
        organization_size: r.get("w_organization_size"),
        timezone: r.get("w_timezone"),
        background_color: r.get("w_background_color"),
        created_at: r.get("w_created_at"),
        updated_at: r.get("w_updated_at"),
    }
}

/// The columns of `workspace_members` the port's `WorkspaceMember` is made of (aliased `wm`).
const MEMBER_COLS: &str = "wm.id AS m_id, wm.workspace_id AS m_workspace_id, wm.user_id AS m_user_id, wm.role AS m_role, \
    wm.company_role AS m_company_role, wm.is_active AS m_is_active, wm.props AS m_props, wm.created_at AS m_created_at, \
    wm.updated_at AS m_updated_at";

fn member_of(r: &Row) -> Result<WorkspaceMember, DbError> {
    Ok(WorkspaceMember {
        id: r.get("m_id"),
        workspace_id: WorkspaceId(r.get("m_workspace_id")),
        user_id: UserId(r.get("m_user_id")),
        role: role_of(r, "m_role")?,
        company_role: r.get("m_company_role"),
        is_active: r.get("m_is_active"),
        props: r.get("m_props"),
        created_at: r.get("m_created_at"),
        updated_at: r.get("m_updated_at"),
    })
}

/// The columns of `projects` the port's `ProjectRow` is made of (aliased `p`).
const PROJECT_COLS: &str = "p.id AS p_id, p.workspace_id AS p_workspace_id, p.name AS p_name, p.identifier AS p_identifier, \
    p.network AS p_network, p.logo_props AS p_logo_props, p.archived_at AS p_archived_at, p.cycle_view AS p_cycle_view, \
    p.issue_views_view AS p_issue_views_view, p.module_view AS p_module_view, p.page_view AS p_page_view, \
    p.intake_view AS p_intake_view, p.guest_view_all_features AS p_guest_view_all_features, \
    p.project_lead AS p_project_lead, p.created_by AS p_created_by, p.updated_by AS p_updated_by, \
    p.created_at AS p_created_at, p.updated_at AS p_updated_at, p.description AS p_description, \
    p.description_text AS p_description_text, p.description_html AS p_description_html, p.emoji AS p_emoji, \
    p.icon_prop AS p_icon_prop, p.is_time_tracking_enabled AS p_is_time_tracking_enabled, \
    p.is_issue_type_enabled AS p_is_issue_type_enabled, p.cover_image AS p_cover_image, \
    p.cover_image_asset AS p_cover_image_asset, p.estimate AS p_estimate, p.archive_in AS p_archive_in, \
    p.close_in AS p_close_in, p.default_state AS p_default_state, p.timezone AS p_timezone, \
    p.external_source AS p_external_source, p.external_id AS p_external_id, p.default_assignee AS p_default_assignee";

fn project_of(r: &Row) -> Result<ProjectRow, DbError> {
    let network: String = r.get("p_network");
    Ok(ProjectRow {
        id: ProjectId(r.get("p_id")),
        workspace_id: WorkspaceId(r.get("p_workspace_id")),
        name: r.get("p_name"),
        identifier: r.get("p_identifier"),
        network: ProjectNetwork::parse(&network)
            .ok_or_else(|| DbError::Backend(format!("p_network holds '{network}', which is no network")))?,
        logo_props: r.get("p_logo_props"),
        archived_at: r.get("p_archived_at"),
        cycle_view: r.get("p_cycle_view"),
        issue_views_view: r.get("p_issue_views_view"),
        module_view: r.get("p_module_view"),
        page_view: r.get("p_page_view"),
        intake_view: r.get("p_intake_view"),
        guest_view_all_features: r.get("p_guest_view_all_features"),
        project_lead: r.get::<_, Option<Uuid>>("p_project_lead").map(UserId),
        created_by: r.get::<_, Option<Uuid>>("p_created_by").map(UserId),
        updated_by: r.get::<_, Option<Uuid>>("p_updated_by").map(UserId),
        created_at: r.get("p_created_at"),
        updated_at: r.get("p_updated_at"),
        description: r.get("p_description"),
        description_text: r.get("p_description_text"),
        description_html: r.get("p_description_html"),
        emoji: r.get("p_emoji"),
        icon_prop: r.get("p_icon_prop"),
        is_time_tracking_enabled: r.get("p_is_time_tracking_enabled"),
        is_issue_type_enabled: r.get("p_is_issue_type_enabled"),
        cover_image: r.get("p_cover_image"),
        cover_image_asset: r.get("p_cover_image_asset"),
        estimate: r.get("p_estimate"),
        archive_in: r.get("p_archive_in"),
        close_in: r.get("p_close_in"),
        default_state: r.get("p_default_state"),
        timezone: r.get("p_timezone"),
        external_source: r.get("p_external_source"),
        external_id: r.get("p_external_id"),
        default_assignee: r.get::<_, Option<Uuid>>("p_default_assignee").map(UserId),
    })
}

/// What a statement that writes a project keeps alive for the length of the call: the values of its columns that are not bound as
/// they are stored.
struct ProjectTemps {
    network: &'static str,
    project_lead: Option<Uuid>,
    created_by: Option<Uuid>,
    updated_by: Option<Uuid>,
    default_assignee: Option<Uuid>,
}

impl ProjectTemps {
    fn of(p: &ProjectRow) -> Self {
        Self {
            network: p.network.as_str(),
            project_lead: p.project_lead.map(|u| u.0),
            created_by: p.created_by.map(|u| u.0),
            updated_by: p.updated_by.map(|u| u.0),
            default_assignee: p.default_assignee.map(|u| u.0),
        }
    }
}

/// Every column of `projects` with its value. A creation writes all of them; an update writes all but the first four (the id, the
/// workspace and the creation).
fn project_columns<'a>(p: &'a ProjectRow, t: &'a ProjectTemps) -> Vec<(&'static str, &'a dyn PgParam)> {
    vec![
        ("id", &p.id.0),
        ("workspace_id", &p.workspace_id.0),
        ("created_at", &p.created_at),
        ("created_by", &t.created_by),
        ("name", &p.name),
        ("identifier", &p.identifier),
        ("network", &t.network),
        ("logo_props", &p.logo_props),
        ("archived_at", &p.archived_at),
        ("cycle_view", &p.cycle_view),
        ("issue_views_view", &p.issue_views_view),
        ("module_view", &p.module_view),
        ("page_view", &p.page_view),
        ("intake_view", &p.intake_view),
        ("guest_view_all_features", &p.guest_view_all_features),
        ("project_lead", &t.project_lead),
        ("updated_by", &t.updated_by),
        ("updated_at", &p.updated_at),
        ("description", &p.description),
        ("description_text", &p.description_text),
        ("description_html", &p.description_html),
        ("emoji", &p.emoji),
        ("icon_prop", &p.icon_prop),
        ("is_time_tracking_enabled", &p.is_time_tracking_enabled),
        ("is_issue_type_enabled", &p.is_issue_type_enabled),
        ("cover_image", &p.cover_image),
        ("cover_image_asset", &p.cover_image_asset),
        ("estimate", &p.estimate),
        ("archive_in", &p.archive_in),
        ("close_in", &p.close_in),
        ("default_state", &p.default_state),
        ("timezone", &p.timezone),
        ("external_source", &p.external_source),
        ("external_id", &p.external_id),
        ("default_assignee", &t.default_assignee),
    ]
}

/// The columns of `project_members` the port's `ProjectMemberRow` is made of (aliased `pm`).
const PMEMBER_COLS: &str = "pm.id AS pm_id, pm.project_id AS pm_project_id, pm.workspace_id AS pm_workspace_id, \
    pm.user_id AS pm_user_id, pm.role AS pm_role, pm.comment AS pm_comment, pm.view_props AS pm_view_props, \
    pm.default_props AS pm_default_props, pm.preferences AS pm_preferences, pm.sort_order AS pm_sort_order, \
    pm.created_by AS pm_created_by, pm.created_at AS pm_created_at, pm.updated_at AS pm_updated_at";

fn pmember_of(r: &Row) -> Result<ProjectMemberRow, DbError> {
    Ok(ProjectMemberRow {
        id: r.get("pm_id"),
        project_id: ProjectId(r.get("pm_project_id")),
        workspace_id: WorkspaceId(r.get("pm_workspace_id")),
        user_id: UserId(r.get("pm_user_id")),
        role: role_of(r, "pm_role")?,
        comment: r.get("pm_comment"),
        view_props: r.get("pm_view_props"),
        default_props: r.get("pm_default_props"),
        preferences: r.get("pm_preferences"),
        sort_order: r.get("pm_sort_order"),
        created_by: r.get::<_, Option<Uuid>>("pm_created_by").map(UserId),
        created_at: r.get("pm_created_at"),
        updated_at: r.get("pm_updated_at"),
    })
}

/// The columns of `states` the port's `StateRow` is made of (aliased `st`).
const STATE_COLS: &str = "st.id AS st_id, st.workspace_id AS st_workspace_id, st.project_id AS st_project_id, st.name AS st_name, \
    st.description AS st_description, st.color AS st_color, st.sequence AS st_sequence, st.state_group AS st_state_group, \
    st.is_default AS st_is_default, st.created_by AS st_created_by, st.created_at AS st_created_at, st.updated_at AS st_updated_at, \
    st.external_source AS st_external_source, st.external_id AS st_external_id, st.updated_by AS st_updated_by";

fn state_of(r: &Row) -> Result<StateRow, DbError> {
    let group: String = r.get("st_state_group");
    Ok(StateRow {
        id: r.get("st_id"),
        workspace_id: WorkspaceId(r.get("st_workspace_id")),
        project_id: ProjectId(r.get("st_project_id")),
        name: r.get("st_name"),
        description: r.get("st_description"),
        color: r.get("st_color"),
        sequence: r.get("st_sequence"),
        group: StateGroup::parse(&group)
            .ok_or_else(|| DbError::Backend(format!("st_state_group holds '{group}', which is no state group")))?,
        is_default: r.get("st_is_default"),
        created_by: r.get::<_, Option<Uuid>>("st_created_by").map(UserId),
        created_at: r.get("st_created_at"),
        updated_at: r.get("st_updated_at"),
        external_source: r.get("st_external_source"),
        external_id: r.get("st_external_id"),
        updated_by: r.get::<_, Option<Uuid>>("st_updated_by").map(UserId),
    })
}

/// The columns of `labels` the port's `LabelRow` is made of (unaliased).
const LABEL_COLS: &str =
    "id, workspace_id, project_id, parent_id, name, description, color, sort_order, created_by, created_at, \
    updated_at, external_source, external_id, updated_by";

fn label_of(r: &Row) -> LabelRow {
    LabelRow {
        id: r.get("id"),
        workspace_id: WorkspaceId(r.get("workspace_id")),
        project_id: ProjectId(r.get("project_id")),
        parent_id: r.get("parent_id"),
        name: r.get("name"),
        description: r.get("description"),
        color: r.get("color"),
        sort_order: r.get("sort_order"),
        created_by: r.get::<_, Option<Uuid>>("created_by").map(UserId),
        created_at: r.get("created_at"),
        updated_at: r.get("updated_at"),
        external_source: r.get("external_source"),
        external_id: r.get("external_id"),
        updated_by: r.get::<_, Option<Uuid>>("updated_by").map(UserId),
    }
}

/// The columns of `project_user_properties` the port's `ProjectUserProperties` is made of (unaliased).
const PUP_COLS: &str =
    "id, project_id, workspace_id, user_id, filters, display_filters, display_properties, rich_filters, \
    preferences, sort_order, created_at, updated_at";

fn project_user_properties_of(r: &Row) -> ProjectUserProperties {
    ProjectUserProperties {
        id: r.get("id"),
        project_id: ProjectId(r.get("project_id")),
        workspace_id: WorkspaceId(r.get("workspace_id")),
        user_id: UserId(r.get("user_id")),
        filters: r.get("filters"),
        display_filters: r.get("display_filters"),
        display_properties: r.get("display_properties"),
        rich_filters: r.get("rich_filters"),
        preferences: r.get("preferences"),
        sort_order: r.get("sort_order"),
        created_at: r.get("created_at"),
        updated_at: r.get("updated_at"),
    }
}

fn user_properties_of(r: &Row) -> UserProperties {
    UserProperties {
        id: r.get("id"),
        filters: r.get("filters"),
        display_filters: r.get("display_filters"),
        display_properties: r.get("display_properties"),
        rich_filters: r.get("rich_filters"),
        navigation_project_limit: r.get("navigation_project_limit"),
        navigation_control_preference: r.get("navigation_control_preference"),
        created_at: r.get("created_at"),
        updated_at: r.get("updated_at"),
    }
}

fn activity(r: &Row) -> Activity {
    Activity {
        id: ActivityId(r.get("id")),
        event_id: EventId(r.get("event_id")),
        workspace_id: WorkspaceId(r.get("workspace_id")),
        project_id: ProjectId(r.get("project_id")),
        entity_id: r.get("entity_id"),
        verb: r.get("verb"),
        actor_id: r.get::<_, Option<Uuid>>("actor_id").map(UserId),
        created_at: r.get("created_at"),
    }
}

/// The columns of a row of a work item's history (`work_item_activities`, migration 0014), `seq` (the order the rows were written in)
/// left out: it is only ever sorted on.
const WIA_COLS: &str =
    "id, event_id, ordinal, workspace_id, project_id, work_item_id, actor_id, verb, field, old_value, new_value, \
    comment, old_identifier, new_identifier, epoch, created_at";

fn work_item_activity(r: &Row) -> WorkItemActivity {
    WorkItemActivity {
        id: ActivityId(r.get("id")),
        event_id: EventId(r.get("event_id")),
        ordinal: r.get("ordinal"),
        workspace_id: WorkspaceId(r.get("workspace_id")),
        project_id: ProjectId(r.get("project_id")),
        work_item_id: WorkItemId(r.get("work_item_id")),
        actor_id: r.get::<_, Option<Uuid>>("actor_id").map(UserId),
        verb: r.get("verb"),
        field: r.get("field"),
        old_value: r.get("old_value"),
        new_value: r.get("new_value"),
        comment: r.get("comment"),
        old_identifier: r.get("old_identifier"),
        new_identifier: r.get("new_identifier"),
        epoch: r.get("epoch"),
        created_at: r.get("created_at"),
    }
}

/// Take the default from the project's other live states ($1 the project, $2 the state that becomes the default).
const CLEAR_OTHER_DEFAULTS: &str =
    "UPDATE states SET is_default = false WHERE project_id = $1 AND id <> $2 AND is_default AND deleted_at IS NULL";

/// A project row of a listing query (`PROJECT_COLS`, the caller's `m_role` and their `up_sort_order`).
fn listing_of(r: &Row) -> Result<ProjectListing, DbError> {
    let role: Option<String> = r.get("m_role");
    Ok(ProjectListing {
        project: project_of(r)?,
        role: role
            .map(|s| Role::parse(&s).ok_or_else(|| DbError::Backend(format!("m_role holds '{s}', which is no role"))))
            .transpose()?,
        sort_order: r.get("up_sort_order"),
        member_sort_order: r.get("m_sort_order"),
    })
}

#[async_trait]
impl Database for PgDatabase {
    async fn membership(&self, slug: &WorkspaceSlug, project: ProjectId, user: UserId) -> Result<Membership, DbError> {
        let c = self.conn().await?;
        // One query: the slug, the caller's workspace membership (inner join), the project in that
        // workspace (inner join) and the caller's project row (left join, same workspace). No row
        // means the caller may not know the workspace or the project exists.
        let row = c
            .pg()
            .query_opt(
                "SELECT w.id AS workspace_id, m.role AS role FROM workspaces w \
                 JOIN workspace_members wm ON wm.workspace_id = w.id AND wm.user_id = $3 AND wm.is_active \
                 JOIN projects p ON p.workspace_id = w.id AND p.id = $2 AND p.deleted_at IS NULL \
                 LEFT JOIN project_members m \
                   ON m.project_id = p.id AND m.workspace_id = p.workspace_id AND m.user_id = $3 \
                 WHERE w.slug = $1",
                &[&slug.as_str(), &project.0, &user.0],
            )
            .await
            .map_err(db_err)?;
        Ok(match row {
            None => Membership::NoSuchProject,
            Some(r) => match r.get::<_, Option<String>>("role").as_deref().and_then(Role::parse) {
                Some(role) => Membership::Member { workspace_id: WorkspaceId(r.get("workspace_id")), role },
                None => Membership::NotAMember,
            },
        })
    }

    async fn session(&self, hash: &TokenHash, now: Timestamp) -> Result<Option<LiveSession>, DbError> {
        let c = self.conn().await?;
        let row = c
            .pg()
            .query_opt(
                "SELECT s.user_id, s.last_seen_at FROM sessions s JOIN users u ON u.id = s.user_id \
                 WHERE s.token_hash = $1 AND s.revoked_at IS NULL AND s.expires_at > $2 AND u.is_active",
                &[&&hash.0[..], &now],
            )
            .await
            .map_err(db_err)?;
        Ok(row.map(|r| LiveSession { user_id: UserId(r.get("user_id")), last_seen_at: r.get("last_seen_at") }))
    }

    async fn touch_session(&self, hash: &TokenHash, at: Timestamp) -> Result<(), DbError> {
        let c = self.conn().await?;
        c.pg()
            .execute("UPDATE sessions SET last_seen_at = $2 WHERE token_hash = $1", &[&&hash.0[..], &at])
            .await
            .map_err(db_err)?;
        Ok(())
    }

    async fn revoke_session(&self, hash: &TokenHash, at: Timestamp) -> Result<(), DbError> {
        let c = self.conn().await?;
        c.pg()
            .execute(
                "UPDATE sessions SET revoked_at = $2 WHERE token_hash = $1 AND revoked_at IS NULL",
                &[&&hash.0[..], &at],
            )
            .await
            .map_err(db_err)?;
        Ok(())
    }

    async fn insert_session(&self, s: &NewSession) -> Result<(), DbError> {
        let c = self.conn().await?;
        c.pg()
            .execute(
                "INSERT INTO sessions (token_hash, user_id, created_at, expires_at, last_seen_at) \
                 VALUES ($1, $2, $3, $4, $3) ON CONFLICT (token_hash) DO NOTHING",
                &[&&s.token_hash.0[..], &s.user_id.0, &s.created_at, &s.expires_at],
            )
            .await
            .map_err(db_err)?;
        Ok(())
    }

    async fn revoke_user_sessions(&self, user: UserId, at: Timestamp) -> Result<u64, DbError> {
        let c = self.conn().await?;
        c.pg()
            .execute("UPDATE sessions SET revoked_at = $2 WHERE user_id = $1 AND revoked_at IS NULL", &[&user.0, &at])
            .await
            .map_err(db_err)
    }

    async fn user_by_email(&self, email: &str) -> Result<Option<UserRecord>, DbError> {
        let c = self.conn().await?;
        let row = c
            .pg()
            .query_opt(&format!("SELECT {USER_COLS} FROM users WHERE email = $1"), &[&email])
            .await
            .map_err(db_err)?;
        Ok(row.as_ref().map(user_record))
    }

    async fn insert_workspace(&self, w: &NewWorkspace) -> Result<(), DbError> {
        // The workspace and its first member go in together or not at all: `Tx` only exposes slice operations, so this goes
        // through a connection of its own, as `seed` does. A taken slug is a unique violation, which `db_err` makes a `Conflict`.
        let c = self.connector.checkout(true).await?;
        let cl = c.pg();
        cl.batch_execute("BEGIN").await.map_err(db_err)?;
        let res: Result<(), DbError> = async {
            let ws = &w.workspace;
            cl.execute(
                "INSERT INTO workspaces (id, slug, name, owner_id, organization_size, timezone, background_color, created_at, updated_at) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
                &[
                    &ws.id.0,
                    &ws.slug.as_str(),
                    &ws.name,
                    &ws.owner_id.map(|u| u.0),
                    &ws.organization_size,
                    &ws.timezone,
                    &ws.background_color,
                    &ws.created_at,
                    &ws.updated_at,
                ],
            )
            .await
            .map_err(db_err)?;
            let m = &w.owner;
            cl.execute(
                "INSERT INTO workspace_members (id, workspace_id, user_id, role, company_role, is_active, props, created_at, updated_at) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
                &[
                    &m.id,
                    &m.workspace_id.0,
                    &m.user_id.0,
                    &m.role.as_str(),
                    &m.company_role,
                    &m.is_active,
                    &m.props,
                    &m.created_at,
                    &m.updated_at,
                ],
            )
            .await
            .map_err(db_err)?;
            Ok(())
        }
        .await;
        match res {
            Ok(()) => cl.batch_execute("COMMIT").await.map_err(db_err),
            Err(e) => {
                let _ = cl.batch_execute("ROLLBACK").await;
                Err(e)
            }
        }
    }

    async fn workspace_slug_taken(&self, slug: &str) -> Result<bool, DbError> {
        let c = self.conn().await?;
        let row = c
            .pg()
            .query_one("SELECT EXISTS (SELECT 1 FROM workspaces WHERE slug = $1) AS taken", &[&slug])
            .await
            .map_err(db_err)?;
        Ok(row.get("taken"))
    }

    async fn any_workspace(&self) -> Result<bool, DbError> {
        let c = self.conn().await?;
        let row = c.pg().query_one("SELECT EXISTS (SELECT 1 FROM workspaces) AS any", &[]).await.map_err(db_err)?;
        Ok(row.get("any"))
    }

    async fn workspaces_of(&self, user: UserId) -> Result<Vec<WorkspaceListing>, DbError> {
        let c = self.conn().await?;
        let rows = c.pg()
            .query(
                &format!(
                    "SELECT {WORKSPACE_COLS}, wm.role AS m_role, \
                       (SELECT count(*) FROM workspace_members n WHERE n.workspace_id = w.id AND n.is_active) AS total_members \
                     FROM workspaces w JOIN workspace_members wm ON wm.workspace_id = w.id AND wm.user_id = $1 AND wm.is_active \
                     ORDER BY w.created_at DESC, w.id DESC"
                ),
                &[&user.0],
            )
            .await
            .map_err(db_err)?;
        rows.iter()
            .map(|r| {
                Ok(WorkspaceListing {
                    workspace: workspace_of(r),
                    role: role_of(r, "m_role")?,
                    total_members: u64::try_from(r.get::<_, i64>("total_members")).unwrap_or(0),
                })
            })
            .collect()
    }

    async fn workspace_access(&self, slug: &WorkspaceSlug, user: UserId) -> Result<Option<WorkspaceAccess>, DbError> {
        let c = self.conn().await?;
        let row = c
            .pg()
            .query_opt(
                "SELECT w.id AS workspace_id, wm.role AS role FROM workspaces w \
                 JOIN workspace_members wm ON wm.workspace_id = w.id AND wm.user_id = $2 AND wm.is_active \
                 WHERE w.slug = $1",
                &[&slug.as_str(), &user.0],
            )
            .await
            .map_err(db_err)?;
        row.map(|r| {
            Ok(WorkspaceAccess { workspace_id: WorkspaceId(r.get("workspace_id")), role: role_of(&r, "role")? })
        })
        .transpose()
    }

    async fn is_workspace_member(&self, scope: &WorkspaceScope, user: UserId) -> Result<bool, DbError> {
        let c = self.conn().await?;
        let row = c.pg()
            .query_one(
                "SELECT EXISTS (SELECT 1 FROM workspace_members WHERE workspace_id = $1 AND user_id = $2 AND is_active) AS is_member",
                &[&scope.workspace_id().0, &user.0],
            )
            .await
            .map_err(db_err)?;
        Ok(row.get("is_member"))
    }

    async fn set_workspace_member_active(
        &self,
        _sys: &SystemScope,
        workspace: WorkspaceId,
        user: UserId,
        active: bool,
    ) -> Result<bool, DbError> {
        let c = self.conn().await?;
        let n = c
            .pg()
            .execute(
                "UPDATE workspace_members SET is_active = $3 WHERE workspace_id = $1 AND user_id = $2",
                &[&workspace.0, &user.0, &active],
            )
            .await
            .map_err(db_err)?;
        Ok(n == 1)
    }

    async fn workspace_member(&self, scope: &WorkspaceScope) -> Result<Option<WorkspaceMember>, DbError> {
        let c = self.conn().await?;
        let row = c
            .pg()
            .query_opt(
                &format!(
                    "SELECT {MEMBER_COLS} FROM workspace_members wm WHERE wm.workspace_id = $1 AND wm.user_id = $2"
                ),
                &[&scope.workspace_id().0, &scope.user_id().0],
            )
            .await
            .map_err(db_err)?;
        row.as_ref().map(member_of).transpose()
    }

    async fn workspace_members(&self, scope: &WorkspaceScope) -> Result<Vec<MemberWithUser>, DbError> {
        let c = self.conn().await?;
        let rows = c
            .pg()
            .query(
                &format!(
                    "SELECT {MEMBER_COLS}, u.email AS u_email, COALESCE(p.first_name, '') AS u_first_name, \
                       COALESCE(p.last_name, '') AS u_last_name, COALESCE(p.display_name, '') AS u_display_name, \
                       COALESCE(p.avatar, '') AS u_avatar \
                     FROM workspace_members wm LEFT JOIN users u ON u.id = wm.user_id \
                       LEFT JOIN user_profiles p ON p.user_id = wm.user_id \
                     WHERE wm.workspace_id = $1 ORDER BY wm.created_at DESC, wm.id DESC"
                ),
                &[&scope.workspace_id().0],
            )
            .await
            .map_err(db_err)?;
        rows.iter()
            .map(|r| {
                let member = member_of(r)?;
                let user = MemberUser {
                    id: member.user_id,
                    email: r.get("u_email"),
                    first_name: r.get("u_first_name"),
                    last_name: r.get("u_last_name"),
                    display_name: r.get("u_display_name"),
                    avatar: r.get("u_avatar"),
                };
                Ok(MemberWithUser { member, user })
            })
            .collect()
    }

    async fn project_roles(&self, scope: &WorkspaceScope) -> Result<Vec<(ProjectId, Role)>, DbError> {
        let c = self.conn().await?;
        let rows = c
            .pg()
            .query(
                // the project must really be in the workspace the member row names: a row that says otherwise is not a role
                "SELECT m.project_id AS project_id, m.role AS role FROM project_members m \
                 JOIN projects p ON p.id = m.project_id AND p.workspace_id = m.workspace_id AND p.deleted_at IS NULL \
                 WHERE m.workspace_id = $1 AND m.user_id = $2 ORDER BY m.project_id",
                &[&scope.workspace_id().0, &scope.user_id().0],
            )
            .await
            .map_err(db_err)?;
        rows.iter().map(|r| Ok((ProjectId(r.get("project_id")), role_of(r, "role")?))).collect()
    }

    async fn workspace_projects(&self, scope: &WorkspaceScope) -> Result<Vec<ProjectListing>, DbError> {
        // Who sees what is decided by two booleans from the caller's workspace role, so the whole list is one statement: an
        // admin sees every project, a member the ones they are in and the public ones, a guest only the ones they are in.
        let (all, public) = (scope.role() == Role::Admin, scope.role() == Role::Member);
        let c = self.conn().await?;
        let rows = c
            .pg()
            .query(
                &format!(
                    "SELECT {PROJECT_COLS}, m.role AS m_role, m.sort_order AS m_sort_order, up.sort_order AS up_sort_order FROM projects p \
                     LEFT JOIN project_members m \
                       ON m.project_id = p.id AND m.workspace_id = p.workspace_id AND m.user_id = $2 \
                     LEFT JOIN project_user_properties up ON up.project_id = p.id AND up.user_id = $2 \
                     WHERE p.workspace_id = $1 AND p.deleted_at IS NULL \
                       AND ($3 OR m.role IS NOT NULL OR ($4 AND p.network = 'public')) \
                     ORDER BY p.created_at DESC, p.id DESC"
                ),
                &[&scope.workspace_id().0, &scope.user_id().0, &all, &public],
            )
            .await
            .map_err(db_err)?;
        rows.iter().map(listing_of).collect()
    }

    async fn visible_projects(&self, scope: &WorkspaceScope) -> Result<Vec<ProjectListing>, DbError> {
        // Plane's v1 rule, whatever the caller's role in the workspace: the projects they are in, and the public ones (a guest sees a
        // public project they are not in, an admin does not see a secret one they are not in).
        let c = self.conn().await?;
        let rows = c.pg()
            .query(
                &format!(
                    "SELECT {PROJECT_COLS}, m.role AS m_role, m.sort_order AS m_sort_order, up.sort_order AS up_sort_order FROM projects p \
                     LEFT JOIN project_members m \
                       ON m.project_id = p.id AND m.workspace_id = p.workspace_id AND m.user_id = $2 \
                     LEFT JOIN project_user_properties up ON up.project_id = p.id AND up.user_id = $2 \
                     WHERE p.workspace_id = $1 AND p.deleted_at IS NULL AND (m.role IS NOT NULL OR p.network = 'public') \
                     ORDER BY p.created_at, p.id"
                ),
                &[&scope.workspace_id().0, &scope.user_id().0],
            )
            .await
            .map_err(db_err)?;
        rows.iter().map(listing_of).collect()
    }

    async fn delete_project(&self, scope: &WorkspaceScope, id: ProjectId, at: Timestamp) -> Result<bool, DbError> {
        let c = self.conn().await?;
        let n = c
            .pg()
            .execute(
                "UPDATE projects SET deleted_at = $3 WHERE id = $1 AND workspace_id = $2 AND deleted_at IS NULL",
                &[&id.0, &scope.workspace_id().0, &at],
            )
            .await
            .map_err(db_err)?;
        Ok(n == 1)
    }

    async fn ensure_sidebar_preferences(
        &self,
        scope: &WorkspaceScope,
        defaults: &[SidebarPreference],
        at: Timestamp,
    ) -> Result<Vec<SidebarPreference>, DbError> {
        let keys: Vec<&str> = defaults.iter().map(|d| d.key.as_str()).collect();
        let pinned: Vec<bool> = defaults.iter().map(|d| d.is_pinned).collect();
        let orders: Vec<f64> = defaults.iter().map(|d| d.sort_order).collect();
        let c = self.conn().await?;
        let cl = c.pg();
        // INSERT first, then SELECT: a first visit racing another waits on the other's row at the conflict, so its SELECT
        // (a new snapshot) sees every row
        cl.execute(
            "INSERT INTO workspace_user_preferences (workspace_id, user_id, key, is_pinned, sort_order, created_at, updated_at) \
             SELECT $1, $2, d.key, d.is_pinned, d.sort_order, $6, $6 \
             FROM unnest($3::text[], $4::bool[], $5::float8[]) AS d(key, is_pinned, sort_order) \
             ON CONFLICT (workspace_id, user_id, key) DO NOTHING",
            &[&scope.workspace_id().0, &scope.user_id().0, &keys, &pinned, &orders, &at],
        )
        .await
        .map_err(db_err)?;
        let rows = cl
            .query(
                "SELECT key, is_pinned, sort_order FROM workspace_user_preferences WHERE workspace_id = $1 AND user_id = $2 \
                 ORDER BY sort_order, key",
                &[&scope.workspace_id().0, &scope.user_id().0],
            )
            .await
            .map_err(db_err)?;
        Ok(rows
            .iter()
            .map(|r| SidebarPreference {
                key: r.get("key"),
                is_pinned: r.get("is_pinned"),
                sort_order: r.get("sort_order"),
            })
            .collect())
    }

    async fn ensure_home_preferences(
        &self,
        scope: &WorkspaceScope,
        defaults: &[HomePreference],
    ) -> Result<Vec<HomePreference>, DbError> {
        let keys: Vec<&str> = defaults.iter().map(|d| d.key.as_str()).collect();
        let enabled: Vec<bool> = defaults.iter().map(|d| d.is_enabled).collect();
        let configs: Vec<serde_json::Value> = defaults.iter().map(|d| d.config.clone()).collect();
        let orders: Vec<f64> = defaults.iter().map(|d| d.sort_order).collect();
        let made: Vec<Timestamp> = defaults.iter().map(|d| d.created_at).collect();
        let c = self.conn().await?;
        let cl = c.pg();
        cl.execute(
            "INSERT INTO workspace_home_preferences (workspace_id, user_id, key, is_enabled, config, sort_order, created_at, updated_at) \
             SELECT $1, $2, d.key, d.is_enabled, d.config, d.sort_order, d.created_at, d.created_at \
             FROM unnest($3::text[], $4::bool[], $5::jsonb[], $6::float8[], $7::timestamptz[]) \
               AS d(key, is_enabled, config, sort_order, created_at) \
             ON CONFLICT (workspace_id, user_id, key) DO NOTHING",
            &[&scope.workspace_id().0, &scope.user_id().0, &keys, &enabled, &configs, &orders, &made],
        )
        .await
        .map_err(db_err)?;
        let rows = cl
            .query(
                "SELECT key, is_enabled, config, sort_order, created_at FROM workspace_home_preferences \
                 WHERE workspace_id = $1 AND user_id = $2 ORDER BY key",
                &[&scope.workspace_id().0, &scope.user_id().0],
            )
            .await
            .map_err(db_err)?;
        Ok(rows
            .iter()
            .map(|r| HomePreference {
                key: r.get("key"),
                is_enabled: r.get("is_enabled"),
                config: r.get("config"),
                sort_order: r.get("sort_order"),
                created_at: r.get("created_at"),
            })
            .collect())
    }

    async fn ensure_user_properties(
        &self,
        scope: &WorkspaceScope,
        new: &UserProperties,
    ) -> Result<UserProperties, DbError> {
        let c = self.conn().await?;
        let cl = c.pg();
        cl.execute(
            "INSERT INTO workspace_user_properties (id, workspace_id, user_id, filters, display_filters, display_properties, \
               rich_filters, navigation_project_limit, navigation_control_preference, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11) ON CONFLICT (workspace_id, user_id) DO NOTHING",
            &[
                &new.id,
                &scope.workspace_id().0,
                &scope.user_id().0,
                &new.filters,
                &new.display_filters,
                &new.display_properties,
                &new.rich_filters,
                &new.navigation_project_limit,
                &new.navigation_control_preference,
                &new.created_at,
                &new.updated_at,
            ],
        )
        .await
        .map_err(db_err)?;
        let row = cl
            .query_one(
                "SELECT id, filters, display_filters, display_properties, rich_filters, navigation_project_limit, \
                   navigation_control_preference, created_at, updated_at \
                 FROM workspace_user_properties WHERE workspace_id = $1 AND user_id = $2",
                &[&scope.workspace_id().0, &scope.user_id().0],
            )
            .await
            .map_err(db_err)?;
        Ok(user_properties_of(&row))
    }

    async fn insert_project(&self, scope: &WorkspaceScope, p: &NewProject) -> Result<(), DbError> {
        if p.project.workspace_id != scope.workspace_id() {
            return Err(DbError::Backend("project is not in this workspace".into()));
        }
        // The project, its members with their settings rows and its states go in together or not at all: `Tx` only exposes slice
        // operations, so this goes through a connection of its own, as `insert_workspace` does. A taken name or key is a unique
        // violation, which `db_err` makes a `Conflict`.
        let c = self.connector.checkout(true).await?;
        let cl = c.pg();
        cl.batch_execute("BEGIN").await.map_err(db_err)?;
        let res: Result<(), DbError> = async {
            let temps = ProjectTemps::of(&p.project);
            let columns = project_columns(&p.project, &temps);
            let names: Vec<&str> = columns.iter().map(|c| c.0).collect();
            let marks: Vec<String> = (1..=columns.len()).map(|i| format!("${i}")).collect();
            let params: Vec<&dyn PgParam> = columns.iter().map(|c| c.1).collect();
            cl.execute(&format!("INSERT INTO projects ({}) VALUES ({})", names.join(", "), marks.join(", ")), &params)
                .await
                .map_err(db_err)?;
            for m in &p.members {
                let (row, props) = (&m.member, &m.properties);
                let (role, created_by) = (row.role.as_str(), row.created_by.map(|u| u.0));
                cl.execute(
                    "INSERT INTO project_members (id, project_id, workspace_id, user_id, role, comment, view_props, default_props, \
                       preferences, sort_order, created_by, created_at, updated_at) \
                     VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)",
                    &[
                        &row.id,
                        &row.project_id.0,
                        &row.workspace_id.0,
                        &row.user_id.0,
                        &role,
                        &row.comment,
                        &row.view_props,
                        &row.default_props,
                        &row.preferences,
                        &row.sort_order,
                        &created_by,
                        &row.created_at,
                        &row.updated_at,
                    ],
                )
                .await
                .map_err(db_err)?;
                // the place the pin gives a project its member has just joined: 10000 less than the lowest they hold in the workspace
                cl.execute(
                    "INSERT INTO project_user_properties (id, project_id, workspace_id, user_id, filters, display_filters, \
                       display_properties, rich_filters, preferences, sort_order, created_at, updated_at) \
                     SELECT $1, $2, $3, $4, $5, $6, $7, $8, $9, \
                       COALESCE((SELECT min(u.sort_order) FROM project_user_properties u WHERE u.workspace_id = $3 AND u.user_id = $4) - 10000, 65535), \
                       $10, $11",
                    &[
                        &props.id,
                        &props.project_id.0,
                        &props.workspace_id.0,
                        &props.user_id.0,
                        &props.filters,
                        &props.display_filters,
                        &props.display_properties,
                        &props.rich_filters,
                        &props.preferences,
                        &props.created_at,
                        &props.updated_at,
                    ],
                )
                .await
                .map_err(db_err)?;
            }
            for st in &p.states {
                let (group, created_by) = (st.group.as_str(), st.created_by.map(|u| u.0));
                cl.execute(
                    "INSERT INTO states (id, workspace_id, project_id, name, description, color, sequence, state_group, is_default, \
                       created_by, created_at, updated_at) \
                     VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
                    &[
                        &st.id,
                        &st.workspace_id.0,
                        &st.project_id.0,
                        &st.name,
                        &st.description,
                        &st.color,
                        &st.sequence,
                        &group,
                        &st.is_default,
                        &created_by,
                        &st.created_at,
                        &st.updated_at,
                    ],
                )
                .await
                .map_err(db_err)?;
            }
            Ok(())
        }
        .await;
        match res {
            Ok(()) => cl.batch_execute("COMMIT").await.map_err(db_err),
            Err(e) => {
                let _ = cl.batch_execute("ROLLBACK").await;
                Err(e)
            }
        }
    }

    async fn project_clashes(
        &self,
        scope: &WorkspaceScope,
        name: Option<&str>,
        identifier: Option<&str>,
        except: Option<ProjectId>,
    ) -> Result<ProjectClashes, DbError> {
        let except = except.map(|p| p.0);
        let c = self.conn().await?;
        let row = c.pg()
            .query_one(
                "SELECT \
                   EXISTS (SELECT 1 FROM projects WHERE workspace_id = $1 AND name = $2 AND deleted_at IS NULL AND ($4::uuid IS NULL OR id <> $4)) AS name_taken, \
                   EXISTS (SELECT 1 FROM projects WHERE workspace_id = $1 AND identifier = $3 AND deleted_at IS NULL AND ($4::uuid IS NULL OR id <> $4)) AS identifier_taken",
                &[&scope.workspace_id().0, &name, &identifier, &except],
            )
            .await
            .map_err(db_err)?;
        Ok(ProjectClashes { name: row.get("name_taken"), identifier: row.get("identifier_taken") })
    }

    async fn update_project(&self, scope: &WorkspaceScope, row: &ProjectRow) -> Result<bool, DbError> {
        let temps = ProjectTemps::of(row);
        let columns = project_columns(row, &temps);
        // everything but the first four columns (the id, the workspace and the creation) is replaced
        let replaced = columns.get(4..).unwrap_or_default();
        let set: Vec<String> = replaced.iter().enumerate().map(|(i, c)| format!("{} = ${}", c.0, i + 3)).collect();
        let workspace = scope.workspace_id().0;
        let mut params: Vec<&dyn PgParam> = vec![&row.id.0, &workspace];
        params.extend(replaced.iter().map(|c| c.1));
        let c = self.conn().await?;
        let n = c
            .pg()
            .execute(
                &format!(
                    "UPDATE projects SET {} WHERE id = $1 AND workspace_id = $2 AND deleted_at IS NULL",
                    set.join(", ")
                ),
                &params,
            )
            .await
            .map_err(db_err)?;
        Ok(n == 1)
    }

    async fn project_detail(&self, scope: &WorkspaceScope, id: ProjectId) -> Result<Option<ProjectDetail>, DbError> {
        let c = self.conn().await?;
        let row = c.pg()
            .query_opt(
                &format!(
                    "SELECT {PROJECT_COLS}, m.role AS m_role, m.sort_order AS m_sort_order, up.sort_order AS up_sort_order, \
                       COALESCE((SELECT s.last_sequence_id FROM project_sequences s WHERE s.workspace_id = p.workspace_id AND s.project_id = p.id), 0) + 1 AS next_sequence, \
                       ARRAY(SELECT pm.user_id FROM project_members pm WHERE pm.project_id = p.id AND pm.workspace_id = p.workspace_id \
                             ORDER BY pm.created_at DESC, pm.id DESC) AS member_ids \
                     FROM projects p \
                     LEFT JOIN project_members m ON m.project_id = p.id AND m.workspace_id = p.workspace_id AND m.user_id = $2 \
                     LEFT JOIN project_user_properties up ON up.project_id = p.id AND up.user_id = $2 \
                     WHERE p.id = $3 AND p.workspace_id = $1 AND p.deleted_at IS NULL"
                ),
                &[&scope.workspace_id().0, &scope.user_id().0, &id.0],
            )
            .await
            .map_err(db_err)?;
        row.map(|r| {
            let role: Option<String> = r.get("m_role");
            Ok(ProjectDetail {
                listing: ProjectListing {
                    project: project_of(&r)?,
                    role: role
                        .map(|s| {
                            Role::parse(&s)
                                .ok_or_else(|| DbError::Backend(format!("m_role holds '{s}', which is no role")))
                        })
                        .transpose()?,
                    sort_order: r.get("up_sort_order"),
                    member_sort_order: r.get("m_sort_order"),
                },
                members: r.get::<_, Vec<Uuid>>("member_ids").into_iter().map(UserId).collect(),
                next_sequence: r.get("next_sequence"),
            })
        })
        .transpose()
    }

    async fn project_member(
        &self,
        scope: &WorkspaceScope,
        project: ProjectId,
    ) -> Result<Option<ProjectMemberWithUser>, DbError> {
        let c = self.conn().await?;
        let row = c.pg()
            .query_opt(
                &format!(
                    "SELECT {PMEMBER_COLS}, {PROJECT_COLS}, {WORKSPACE_COLS}, u.email AS u_email, \
                       COALESCE(pr.first_name, '') AS u_first_name, COALESCE(pr.last_name, '') AS u_last_name, \
                       COALESCE(pr.display_name, '') AS u_display_name, COALESCE(pr.avatar, '') AS u_avatar \
                     FROM project_members pm \
                       JOIN projects p ON p.id = pm.project_id AND p.workspace_id = pm.workspace_id AND p.deleted_at IS NULL \
                       JOIN workspaces w ON w.id = p.workspace_id \
                       LEFT JOIN users u ON u.id = pm.user_id LEFT JOIN user_profiles pr ON pr.user_id = pm.user_id \
                     WHERE pm.workspace_id = $1 AND pm.project_id = $2 AND pm.user_id = $3"
                ),
                &[&scope.workspace_id().0, &project.0, &scope.user_id().0],
            )
            .await
            .map_err(db_err)?;
        row.map(|r| {
            let member = pmember_of(&r)?;
            let user = MemberUser {
                id: member.user_id,
                email: r.get("u_email"),
                first_name: r.get("u_first_name"),
                last_name: r.get("u_last_name"),
                display_name: r.get("u_display_name"),
                avatar: r.get("u_avatar"),
            };
            Ok(ProjectMemberWithUser { member, user, project: project_of(&r)?, workspace: workspace_of(&r) })
        })
        .transpose()
    }

    async fn project_members(&self, scope: &Scope) -> Result<Vec<ProjectMemberRow>, DbError> {
        let c = self.conn().await?;
        let rows = c.pg()
            .query(
                &format!(
                    "SELECT {PMEMBER_COLS} FROM project_members pm \
                       JOIN projects p ON p.id = pm.project_id AND p.workspace_id = pm.workspace_id AND p.deleted_at IS NULL \
                       JOIN workspace_members wm ON wm.workspace_id = pm.workspace_id AND wm.user_id = pm.user_id AND wm.is_active \
                     WHERE pm.workspace_id = $1 AND pm.project_id = $2 ORDER BY pm.created_at DESC, pm.id DESC"
                ),
                &[&scope.workspace_id().0, &scope.project_id().0],
            )
            .await
            .map_err(db_err)?;
        rows.iter().map(pmember_of).collect()
    }

    async fn ensure_project_user_properties(
        &self,
        scope: &Scope,
        new: &ProjectUserProperties,
    ) -> Result<ProjectUserProperties, DbError> {
        let c = self.conn().await?;
        let cl = c.pg();
        // from `projects`, so a project that is not in the scope's workspace makes no row (and the SELECT below finds none)
        cl.execute(
            "INSERT INTO project_user_properties (id, project_id, workspace_id, user_id, filters, display_filters, display_properties, \
               rich_filters, preferences, sort_order, created_at, updated_at) \
             SELECT $1, p.id, p.workspace_id, $4, $5, $6, $7, $8, $9, $10, $11, $12 FROM projects p WHERE p.id = $2 AND p.workspace_id = $3 AND p.deleted_at IS NULL \
             ON CONFLICT (project_id, user_id) DO NOTHING",
            &[
                &new.id,
                &scope.project_id().0,
                &scope.workspace_id().0,
                &scope.user_id().0,
                &new.filters,
                &new.display_filters,
                &new.display_properties,
                &new.rich_filters,
                &new.preferences,
                &new.sort_order,
                &new.created_at,
                &new.updated_at,
            ],
        )
        .await
        .map_err(db_err)?;
        let row = cl
            .query_one(
                &format!("SELECT {PUP_COLS} FROM project_user_properties WHERE project_id = $1 AND user_id = $2 AND workspace_id = $3"),
                &[&scope.project_id().0, &scope.user_id().0, &scope.workspace_id().0],
            )
            .await
            .map_err(db_err)?;
        Ok(project_user_properties_of(&row))
    }

    async fn project_labels(&self, scope: &Scope) -> Result<Vec<LabelRow>, DbError> {
        let c = self.conn().await?;
        let rows = c
            .pg()
            .query(
                &format!(
                    "SELECT {LABEL_COLS} FROM labels \
                     WHERE workspace_id = $1 AND project_id = $2 AND deleted_at IS NULL ORDER BY sort_order, id"
                ),
                &[&scope.workspace_id().0, &scope.project_id().0],
            )
            .await
            .map_err(db_err)?;
        Ok(rows.iter().map(label_of).collect())
    }

    async fn insert_label(&self, scope: &Scope, l: &LabelRow) -> Result<LabelRow, DbError> {
        if l.workspace_id != scope.workspace_id() || l.project_id != scope.project_id() {
            return Err(DbError::Backend("label is not in this project".into()));
        }
        // The row goes in with a place of Plane's making, in one statement. A taken name is a unique violation, which `db_err` makes a
        // `Conflict`.
        let c = self.conn().await?;
        let (created_by, updated_by) = (l.created_by.map(|u| u.0), l.updated_by.map(|u| u.0));
        let row = c.pg()
            .query_one(
                &format!(
                    "INSERT INTO labels (id, workspace_id, project_id, parent_id, name, description, color, sort_order, \
                       external_source, external_id, created_by, updated_by, created_at, updated_at) \
                     VALUES ($1, $2, $3, $4, $5, $6, $7, \
                       COALESCE((SELECT max(o.sort_order) FROM labels o WHERE o.project_id = $3 AND o.deleted_at IS NULL) + 10000, $8), \
                       $9, $10, $11, $12, $13, $14) \
                     RETURNING {LABEL_COLS}"
                ),
                &[
                    &l.id,
                    &l.workspace_id.0,
                    &l.project_id.0,
                    &l.parent_id,
                    &l.name,
                    &l.description,
                    &l.color,
                    &l.sort_order,
                    &l.external_source,
                    &l.external_id,
                    &created_by,
                    &updated_by,
                    &l.created_at,
                    &l.updated_at,
                ],
            )
            .await
            .map_err(db_err)?;
        Ok(label_of(&row))
    }

    async fn update_label(&self, scope: &Scope, l: &LabelRow) -> Result<bool, DbError> {
        let c = self.conn().await?;
        let updated_by = l.updated_by.map(|u| u.0);
        let n = c
            .pg()
            .execute(
                "UPDATE labels SET parent_id = $4, name = $5, description = $6, color = $7, sort_order = $8, \
                   external_source = $9, external_id = $10, updated_by = $11, updated_at = $12 \
                 WHERE id = $1 AND project_id = $2 AND workspace_id = $3 AND deleted_at IS NULL",
                &[
                    &l.id,
                    &scope.project_id().0,
                    &scope.workspace_id().0,
                    &l.parent_id,
                    &l.name,
                    &l.description,
                    &l.color,
                    &l.sort_order,
                    &l.external_source,
                    &l.external_id,
                    &updated_by,
                    &l.updated_at,
                ],
            )
            .await
            .map_err(db_err)?;
        Ok(n == 1)
    }

    async fn delete_label(&self, scope: &Scope, id: Uuid, at: Timestamp) -> Result<bool, DbError> {
        // One statement, so a label, the live labels under it and the work items' hold on every one of them go together or not at all.
        // The walk is a UNION (not UNION ALL), which drops a row it has met: the pin lets a label be its own parent, and a loop of
        // labels must end the walk, not feed it.
        let c = self.conn().await?;
        let row = c.pg()
            .query_one(
                "WITH RECURSIVE tree AS ( \
                     SELECT id FROM labels WHERE id = $1 AND project_id = $2 AND workspace_id = $3 AND deleted_at IS NULL \
                   UNION \
                     SELECT c.id FROM labels c JOIN tree t ON c.parent_id = t.id \
                     WHERE c.workspace_id = $3 AND c.deleted_at IS NULL \
                 ), \
                 unassigned AS (DELETE FROM work_item_labels WHERE label_id IN (SELECT id FROM tree)), \
                 gone AS (UPDATE labels SET deleted_at = $4 WHERE id IN (SELECT id FROM tree) RETURNING id) \
                 SELECT EXISTS (SELECT 1 FROM gone WHERE id = $1) AS deleted",
                &[&id, &scope.project_id().0, &scope.workspace_id().0, &at],
            )
            .await
            .map_err(db_err)?;
        Ok(row.get("deleted"))
    }

    async fn project_states(&self, scope: &Scope) -> Result<Vec<StateRow>, DbError> {
        let c = self.conn().await?;
        let rows = c.pg()
            .query(
                &format!(
                    "SELECT {STATE_COLS} FROM states st WHERE st.workspace_id = $1 AND st.project_id = $2 AND st.state_group <> 'triage' \
                       AND st.deleted_at IS NULL ORDER BY st.sequence, st.id"
                ),
                &[&scope.workspace_id().0, &scope.project_id().0],
            )
            .await
            .map_err(db_err)?;
        rows.iter().map(state_of).collect()
    }

    async fn workspace_states(&self, scope: &WorkspaceScope) -> Result<Vec<StateRow>, DbError> {
        let c = self.conn().await?;
        let rows = c.pg()
            .query(
                &format!(
                    "SELECT {STATE_COLS} FROM states st \
                       JOIN projects p ON p.id = st.project_id AND p.workspace_id = st.workspace_id AND p.archived_at IS NULL AND p.deleted_at IS NULL \
                       JOIN project_members m ON m.project_id = p.id AND m.workspace_id = p.workspace_id AND m.user_id = $2 \
                     WHERE st.workspace_id = $1 AND st.state_group <> 'triage' AND st.deleted_at IS NULL \
                     ORDER BY st.sequence, st.project_id, st.id"
                ),
                &[&scope.workspace_id().0, &scope.user_id().0],
            )
            .await
            .map_err(db_err)?;
        rows.iter().map(state_of).collect()
    }

    async fn triage_state(&self, scope: &Scope) -> Result<Option<StateRow>, DbError> {
        let c = self.conn().await?;
        let row = c.pg()
            .query_opt(
                &format!(
                    "SELECT {STATE_COLS} FROM states st WHERE st.workspace_id = $1 AND st.project_id = $2 AND st.state_group = 'triage' \
                       AND st.deleted_at IS NULL ORDER BY st.sequence, st.id LIMIT 1"
                ),
                &[&scope.workspace_id().0, &scope.project_id().0],
            )
            .await
            .map_err(db_err)?;
        row.as_ref().map(state_of).transpose()
    }

    async fn project_is_archived(&self, scope: &Scope) -> Result<bool, DbError> {
        let c = self.conn().await?;
        let row = c
            .pg()
            .query_opt(
                "SELECT archived_at IS NOT NULL AS archived FROM projects \
                 WHERE id = $1 AND workspace_id = $2 AND deleted_at IS NULL",
                &[&scope.project_id().0, &scope.workspace_id().0],
            )
            .await
            .map_err(db_err)?;
        Ok(row.is_some_and(|r| r.get::<_, bool>("archived")))
    }

    async fn insert_state(&self, scope: &Scope, st: &StateRow) -> Result<StateRow, DbError> {
        if st.workspace_id != scope.workspace_id() || st.project_id != scope.project_id() {
            return Err(DbError::Backend("state is not in this project".into()));
        }
        // A new default state takes the default from the project's other states, and the row goes in with a place of Plane's
        // making, in one transaction: the one-default index would refuse the row while another still held the flag. A taken name is a
        // unique violation, which `db_err` makes a `Conflict`.
        let c = self.connector.checkout(true).await?;
        let cl = c.pg();
        cl.batch_execute("BEGIN").await.map_err(db_err)?;
        let res: Result<StateRow, DbError> = async {
            if st.is_default {
                cl.execute(CLEAR_OTHER_DEFAULTS, &[&st.project_id.0, &st.id]).await.map_err(db_err)?;
            }
            let (group, created_by, updated_by) =
                (st.group.as_str(), st.created_by.map(|u| u.0), st.updated_by.map(|u| u.0));
            let row = cl
                .query_one(
                    &format!(
                        "INSERT INTO states AS st (id, workspace_id, project_id, name, description, color, sequence, state_group, \
                           is_default, external_source, external_id, created_by, updated_by, created_at, updated_at) \
                         VALUES ($1, $2, $3, $4, $5, $6, \
                           COALESCE((SELECT max(o.sequence) FROM states o WHERE o.project_id = $3 AND o.state_group <> 'triage' \
                                       AND o.deleted_at IS NULL) + 15000, $7), \
                           $8, $9, $10, $11, $12, $13, $14, $15) \
                         RETURNING {STATE_COLS}"
                    ),
                    &[
                        &st.id,
                        &st.workspace_id.0,
                        &st.project_id.0,
                        &st.name,
                        &st.description,
                        &st.color,
                        &st.sequence,
                        &group,
                        &st.is_default,
                        &st.external_source,
                        &st.external_id,
                        &created_by,
                        &updated_by,
                        &st.created_at,
                        &st.updated_at,
                    ],
                )
                .await
                .map_err(db_err)?;
            state_of(&row)
        }
        .await;
        match res {
            Ok(row) => cl.batch_execute("COMMIT").await.map_err(db_err).map(|()| row),
            Err(e) => {
                let _ = cl.batch_execute("ROLLBACK").await;
                Err(e)
            }
        }
    }

    async fn update_state(&self, scope: &Scope, st: &StateRow) -> Result<bool, DbError> {
        let c = self.connector.checkout(true).await?;
        let cl = c.pg();
        cl.batch_execute("BEGIN").await.map_err(db_err)?;
        let res: Result<bool, DbError> = async {
            if st.is_default {
                cl.execute(CLEAR_OTHER_DEFAULTS, &[&scope.project_id().0, &st.id]).await.map_err(db_err)?;
            }
            let (group, updated_by) = (st.group.as_str(), st.updated_by.map(|u| u.0));
            let n = cl
                .execute(
                    "UPDATE states SET name = $4, description = $5, color = $6, sequence = $7, state_group = $8, is_default = $9, \
                       external_source = $10, external_id = $11, updated_by = $12, updated_at = $13 \
                     WHERE id = $1 AND project_id = $2 AND workspace_id = $3 AND deleted_at IS NULL",
                    &[
                        &st.id,
                        &scope.project_id().0,
                        &scope.workspace_id().0,
                        &st.name,
                        &st.description,
                        &st.color,
                        &st.sequence,
                        &group,
                        &st.is_default,
                        &st.external_source,
                        &st.external_id,
                        &updated_by,
                        &st.updated_at,
                    ],
                )
                .await
                .map_err(db_err)?;
            Ok(n == 1)
        }
        .await;
        match res {
            Ok(true) => cl.batch_execute("COMMIT").await.map_err(db_err).map(|()| true),
            // no such state: nothing changed, and the default that was taken from the others is given back
            Ok(false) => cl.batch_execute("ROLLBACK").await.map_err(db_err).map(|()| false),
            Err(e) => {
                let _ = cl.batch_execute("ROLLBACK").await;
                Err(e)
            }
        }
    }

    async fn delete_state(&self, scope: &Scope, id: Uuid, at: Timestamp) -> Result<bool, DbError> {
        let c = self.conn().await?;
        let n = c
            .pg()
            .execute(
                "UPDATE states SET deleted_at = $4 \
                 WHERE id = $1 AND project_id = $2 AND workspace_id = $3 AND deleted_at IS NULL",
                &[&id, &scope.project_id().0, &scope.workspace_id().0, &at],
            )
            .await
            .map_err(db_err)?;
        Ok(n == 1)
    }

    async fn state_has_work_items(&self, scope: &Scope, id: Uuid) -> Result<bool, DbError> {
        // The pin's `Issue.objects`, which leaves out what is archived, a draft or deleted, and everything in an archived project.
        let c = self.conn().await?;
        let row = c
            .pg()
            .query_one(
                "SELECT EXISTS (SELECT 1 FROM work_items w \
                   JOIN projects p ON p.id = w.project_id AND p.workspace_id = w.workspace_id \
                     AND p.archived_at IS NULL AND p.deleted_at IS NULL \
                   WHERE w.state_id = $1 AND w.workspace_id = $2 AND w.project_id = $3 \
                     AND w.deleted_at IS NULL AND w.archived_at IS NULL AND NOT w.is_draft) AS present",
                &[&id, &scope.workspace_id().0, &scope.project_id().0],
            )
            .await
            .map_err(db_err)?;
        Ok(row.get("present"))
    }

    async fn project_stats(
        &self,
        scope: &WorkspaceScope,
        only: Option<&[ProjectId]>,
    ) -> Result<Vec<ProjectStat>, DbError> {
        let only: Option<Vec<Uuid>> = only.map(|ids| ids.iter().map(|p| p.0).collect());
        let c = self.conn().await?;
        let rows = c.pg()
            .query(
                "SELECT p.id AS id, \
                   CASE WHEN p.archived_at IS NULL THEN \
                     (SELECT count(*) FROM work_items w WHERE w.project_id = p.id AND w.workspace_id = p.workspace_id \
                        AND w.deleted_at IS NULL AND w.archived_at IS NULL AND NOT w.is_draft) \
                   ELSE 0 END AS total_issues, \
                   (SELECT count(*) FROM project_members m WHERE m.project_id = p.id AND m.workspace_id = p.workspace_id) AS total_members \
                 FROM projects p WHERE p.workspace_id = $1 AND p.deleted_at IS NULL AND ($2::uuid[] IS NULL OR p.id = ANY($2)) \
                 ORDER BY p.created_at DESC, p.id DESC",
                &[&scope.workspace_id().0, &only],
            )
            .await
            .map_err(db_err)?;
        Ok(rows
            .iter()
            .map(|r| ProjectStat {
                id: ProjectId(r.get("id")),
                total_issues: r.get("total_issues"),
                total_members: r.get("total_members"),
            })
            .collect())
    }

    async fn user_by_id(&self, id: UserId) -> Result<Option<UserRecord>, DbError> {
        let c = self.conn().await?;
        let row = c
            .pg()
            .query_opt(&format!("SELECT {USER_COLS} FROM users WHERE id = $1"), &[&id.0])
            .await
            .map_err(db_err)?;
        Ok(row.as_ref().map(user_record))
    }

    async fn insert_profile(&self, p: &UserProfile) -> Result<bool, DbError> {
        // ON CONFLICT on the user only: a taken `id` or `username` is a unique violation, which `db_err` makes a `Conflict`
        let c = self.conn().await?;
        let n = c
            .pg()
            .execute(
                &format!(
                    "INSERT INTO user_profiles ({PROFILE_COLS}) VALUES \
                     ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18) \
                     ON CONFLICT (user_id) DO NOTHING"
                ),
                &[
                    &p.user_id.0,
                    &p.id,
                    &p.username,
                    &p.first_name,
                    &p.last_name,
                    &p.display_name,
                    &p.avatar,
                    &p.cover_image,
                    &p.timezone,
                    &p.date_joined,
                    &p.last_login_at,
                    &p.is_onboarded,
                    &p.is_tour_completed,
                    &p.last_workspace_id.map(|w| w.0),
                    &p.onboarding_step,
                    &p.preferences,
                    &p.created_at,
                    &p.updated_at,
                ],
            )
            .await
            .map_err(db_err)?;
        Ok(n == 1)
    }

    async fn profile(&self, user: UserId) -> Result<Option<UserProfile>, DbError> {
        let c = self.conn().await?;
        let row = c
            .pg()
            .query_opt(&format!("SELECT {PROFILE_COLS} FROM user_profiles WHERE user_id = $1"), &[&user.0])
            .await
            .map_err(db_err)?;
        Ok(row.as_ref().map(user_profile))
    }

    async fn update_profile(
        &self,
        user: UserId,
        patch: &ProfilePatch,
        at: Timestamp,
    ) -> Result<Option<UserProfile>, DbError> {
        // ONE statement: every column the patch does not name keeps its value, and the preferences are merged by jsonb `||`
        // inside the row lock, so two requests that change different fields cannot overwrite each other.
        let set_cover = patch.cover_image.is_some();
        let cover: Option<&str> = patch.cover_image.as_ref().and_then(|c| c.as_deref());
        let set_workspace = patch.last_workspace_id.is_some();
        let workspace: Option<Uuid> = patch.last_workspace_id.and_then(|w| w.map(|w| w.0));
        let preferences = serde_json::Value::Object(patch.preferences.clone());
        let c = self.conn().await?;
        let row = c.pg()
            .query_opt(
                &format!(
                    "UPDATE user_profiles SET \
                       first_name = COALESCE($2, first_name), last_name = COALESCE($3, last_name), \
                       display_name = COALESCE($4, display_name), avatar = COALESCE($5, avatar), \
                       cover_image = CASE WHEN $6 THEN $7 ELSE cover_image END, \
                       timezone = COALESCE($8, timezone), last_login_at = COALESCE($9, last_login_at), \
                       is_onboarded = COALESCE($10, is_onboarded), is_tour_completed = COALESCE($11, is_tour_completed), \
                       last_workspace_id = CASE WHEN $12 THEN $13 ELSE last_workspace_id END, \
                       onboarding_step = COALESCE($14, onboarding_step), preferences = preferences || $15, \
                       updated_at = $16 \
                     WHERE user_id = $1 RETURNING {PROFILE_COLS}"
                ),
                &[
                    &user.0,
                    &patch.first_name.as_deref(),
                    &patch.last_name.as_deref(),
                    &patch.display_name.as_deref(),
                    &patch.avatar.as_deref(),
                    &set_cover,
                    &cover,
                    &patch.timezone.as_deref(),
                    &patch.last_login_at,
                    &patch.is_onboarded,
                    &patch.is_tour_completed,
                    &set_workspace,
                    &workspace,
                    &patch.onboarding_step,
                    &preferences,
                    &at,
                ],
            )
            .await
            .map_err(db_err)?;
        Ok(row.as_ref().map(user_profile))
    }

    async fn insert_user(&self, user: &UserRecord) -> Result<(), DbError> {
        // No ON CONFLICT: a taken id or email must fail, and `db_err` turns the unique violation into `Conflict`.
        let c = self.conn().await?;
        c.pg()
            .execute(
                "INSERT INTO users (id, email, password_hash, is_active) VALUES ($1, $2, $3, $4)",
                &[&user.id.0, &user.email.as_deref(), &user.password_hash.as_deref(), &user.is_active],
            )
            .await
            .map_err(db_err)?;
        Ok(())
    }

    async fn user_for_account(&self, provider: &str, provider_account_id: &str) -> Result<Option<UserRecord>, DbError> {
        let c = self.conn().await?;
        let row = c
            .pg()
            .query_opt(
                "SELECT u.id, u.email, u.password_hash, u.is_active FROM accounts a JOIN users u ON u.id = a.user_id \
                 WHERE a.provider = $1 AND a.provider_account_id = $2",
                &[&provider, &provider_account_id],
            )
            .await
            .map_err(db_err)?;
        Ok(row.as_ref().map(user_record))
    }

    async fn link_account(&self, a: &NewAccount) -> Result<(), DbError> {
        let c = self.conn().await?;
        c.pg()
            .execute(
                "INSERT INTO accounts (provider, provider_account_id, user_id, email, created_at) \
                 VALUES ($1, $2, $3, $4, $5) ON CONFLICT (provider, provider_account_id) DO NOTHING",
                &[
                    &a.provider.as_str(),
                    &a.provider_account_id.as_str(),
                    &a.user_id.0,
                    &a.email.as_deref(),
                    &a.created_at,
                ],
            )
            .await
            .map_err(db_err)?;
        Ok(())
    }

    async fn insert_api_key(&self, k: &NewApiKey) -> Result<(), DbError> {
        let c = self.conn().await?;
        c.pg()
            .execute(
                "INSERT INTO api_keys (id, user_id, token_hash, hash_version, prefix, label, created_at, expires_at) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
                &[
                    &k.id.0,
                    &k.user_id.0,
                    &&k.token_hash.0[..],
                    &k.hash_version,
                    &k.prefix.as_str(),
                    &k.label.as_str(),
                    &k.created_at,
                    &k.expires_at,
                ],
            )
            .await
            .map_err(db_err)?;
        Ok(())
    }

    async fn api_key(&self, hash: &TokenHash, now: Timestamp) -> Result<Option<LiveApiKey>, DbError> {
        let c = self.conn().await?;
        let row = c.pg()
            .query_opt(
                "SELECT k.id, k.user_id, k.last_used_at, k.hash_version FROM api_keys k JOIN users u ON u.id = k.user_id \
                 WHERE k.token_hash = $1 AND k.revoked_at IS NULL AND (k.expires_at IS NULL OR k.expires_at > $2) \
                 AND u.is_active",
                &[&&hash.0[..], &now],
            )
            .await
            .map_err(db_err)?;
        Ok(row.map(|r| LiveApiKey {
            id: ApiKeyId(r.get("id")),
            user_id: UserId(r.get("user_id")),
            last_used_at: r.get("last_used_at"),
            hash_version: r.get("hash_version"),
        }))
    }

    async fn touch_api_key(&self, id: ApiKeyId, at: Timestamp) -> Result<(), DbError> {
        let c = self.conn().await?;
        c.pg().execute("UPDATE api_keys SET last_used_at = $2 WHERE id = $1", &[&id.0, &at]).await.map_err(db_err)?;
        Ok(())
    }

    async fn rehash_api_key(
        &self,
        id: ApiKeyId,
        from_version: i16,
        new_hash: &TokenHash,
        new_version: i16,
    ) -> Result<bool, DbError> {
        let c = self.conn().await?;
        // The compare-and-set is the WHERE clause: a row that is not (or no longer) at `from_version` is not touched,
        // and neither of its other constraints is looked at. Owner, expiry, revocation and last use stay.
        let n = c
            .pg()
            .execute(
                "UPDATE api_keys SET token_hash = $3, hash_version = $4 WHERE id = $1 AND hash_version = $2",
                &[&id.0, &from_version, &&new_hash.0[..], &new_version],
            )
            .await
            .map_err(db_err)?;
        Ok(n == 1)
    }

    async fn list_api_keys(&self, user: UserId) -> Result<Vec<ApiKeyInfo>, DbError> {
        let c = self.conn().await?;
        let rows = c
            .pg()
            .query(
                "SELECT id, prefix, label, created_at, expires_at, revoked_at, last_used_at FROM api_keys \
                 WHERE user_id = $1 ORDER BY created_at, id",
                &[&user.0],
            )
            .await
            .map_err(db_err)?;
        Ok(rows
            .iter()
            .map(|r| ApiKeyInfo {
                id: ApiKeyId(r.get("id")),
                prefix: r.get("prefix"),
                label: r.get("label"),
                created_at: r.get("created_at"),
                expires_at: r.get("expires_at"),
                revoked_at: r.get("revoked_at"),
                last_used_at: r.get("last_used_at"),
            })
            .collect())
    }

    async fn revoke_api_key(&self, user: UserId, id: ApiKeyId, at: Timestamp) -> Result<bool, DbError> {
        let c = self.conn().await?;
        // COALESCE keeps the first revocation time, so a second revoke changes nothing but still says "yours".
        let n = c
            .pg()
            .execute(
                "UPDATE api_keys SET revoked_at = COALESCE(revoked_at, $3) WHERE id = $1 AND user_id = $2",
                &[&id.0, &user.0, &at],
            )
            .await
            .map_err(db_err)?;
        Ok(n == 1)
    }

    async fn insert_oidc_login(&self, l: &NewOidcLogin) -> Result<(), DbError> {
        let c = self.conn().await?;
        let pg = c.pg();
        pg.execute("DELETE FROM oidc_logins WHERE expires_at <= $1", &[&l.created_at]).await.map_err(db_err)?;
        pg.execute(
            "INSERT INTO oidc_logins (state_hash, nonce, code_verifier, created_at, expires_at) \
             VALUES ($1, $2, $3, $4, $5)",
            &[&&l.state_hash.0[..], &l.nonce.as_str(), &l.code_verifier.as_str(), &l.created_at, &l.expires_at],
        )
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn take_oidc_login(&self, state: &TokenHash) -> Result<Option<OidcLogin>, DbError> {
        let c = self.conn().await?;
        // One statement: the row is returned to exactly one caller, and gone for the next.
        let row = c
            .pg()
            .query_opt(
                "DELETE FROM oidc_logins WHERE state_hash = $1 RETURNING nonce, code_verifier, expires_at",
                &[&&state.0[..]],
            )
            .await
            .map_err(db_err)?;
        Ok(row.map(|r| OidcLogin {
            nonce: r.get("nonce"),
            code_verifier: r.get("code_verifier"),
            expires_at: r.get("expires_at"),
        }))
    }

    async fn insert_asset(&self, scope: &WorkspaceScope, a: &AssetRow) -> Result<(), DbError> {
        if a.workspace_id != scope.workspace_id() || a.created_by != scope.user_id() {
            return Err(DbError::Backend("asset is not the scope's".into()));
        }
        let Some(grant) = a.upload.filter(|_| a.state == AssetState::Pending && a.uploaded_at.is_none()) else {
            return Err(DbError::Backend("a new asset is pending, with its grant".into()));
        };
        let c = self.conn().await?;
        c.pg()
            .execute(
                "INSERT INTO assets (id, workspace_id, created_by, entity_type, project_id, name, content_type, size, object_key, \
                   state, upload_token_hash, upload_expires_at, created_at) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, 'pending', $10, $11, $12)",
                &[
                    &a.id.0,
                    &a.workspace_id.0,
                    &a.created_by.0,
                    &a.entity.as_str(),
                    &a.project_id.map(|p| p.0),
                    &a.name.as_str(),
                    &a.content_type.as_str(),
                    &a.size,
                    &a.object_key.as_str(),
                    &&grant.token_hash.0[..],
                    &grant.expires_at,
                    &a.created_at,
                ],
            )
            .await
            .map_err(db_err)?;
        Ok(())
    }

    async fn asset(&self, scope: &WorkspaceScope, id: AssetId) -> Result<Option<AssetRow>, DbError> {
        let c = self.conn().await?;
        let row = c
            .pg()
            .query_opt(
                &format!("SELECT {ASSET_COLS} FROM assets WHERE id = $1 AND workspace_id = $2 AND deleted_at IS NULL"),
                &[&id.0, &scope.workspace_id().0],
            )
            .await
            .map_err(db_err)?;
        row.as_ref().map(asset_row).transpose()
    }

    async fn mark_asset_uploaded(&self, scope: &WorkspaceScope, id: AssetId, at: Timestamp) -> Result<bool, DbError> {
        let c = self.conn().await?;
        // an asset that is uploaded already keeps its time (and has no grant to lose), so this is idempotent
        let n = c
            .pg()
            .execute(
                "UPDATE assets SET state = 'uploaded', uploaded_at = COALESCE(uploaded_at, $3), \
                   upload_token_hash = NULL, upload_expires_at = NULL \
                 WHERE id = $1 AND workspace_id = $2 AND deleted_at IS NULL",
                &[&id.0, &scope.workspace_id().0, &at],
            )
            .await
            .map_err(db_err)?;
        Ok(n == 1)
    }

    async fn attach_project_cover(
        &self,
        scope: &Scope,
        ids: &[AssetId],
        at: Timestamp,
    ) -> Result<Option<AssetId>, DbError> {
        let asked: Vec<Uuid> = ids.iter().map(|i| i.0).collect();
        let c = self.conn().await?;
        // One statement, so one transaction: the assets that qualify get the project, and the oldest of them becomes its cover (the
        // pin saves each in turn, newest first), or nothing is written when none qualifies.
        let row = c
            .pg()
            .query_opt(
                "WITH matched AS ( \
                   UPDATE assets SET project_id = $2 \
                   WHERE id = ANY($3) AND workspace_id = $1 AND created_by = $4 AND deleted_at IS NULL \
                     AND (project_id IS NULL OR project_id = $2) AND entity_type = 'PROJECT_COVER' \
                   RETURNING id, created_at \
                 ), cover AS ( \
                   SELECT id FROM matched ORDER BY created_at, id LIMIT 1 \
                 ), changed AS ( \
                   UPDATE projects SET cover_image_asset = (SELECT id FROM cover), updated_by = $4, updated_at = $5 \
                   WHERE id = $2 AND workspace_id = $1 AND deleted_at IS NULL AND EXISTS (SELECT 1 FROM cover) \
                   RETURNING id \
                 ) \
                 SELECT id FROM cover",
                &[&scope.workspace_id().0, &scope.project_id().0, &asked, &scope.user_id().0, &at],
            )
            .await
            .map_err(db_err)?;
        Ok(row.map(|r| AssetId(r.get("id"))))
    }

    async fn take_upload_grant(
        &self,
        id: AssetId,
        token: &TokenHash,
        now: Timestamp,
    ) -> Result<Option<AssetRow>, DbError> {
        let c = self.conn().await?;
        // One statement: the grant is spent by exactly one caller, and the next finds none
        let row = c
            .pg()
            .query_opt(
                &format!(
                    "UPDATE assets SET upload_token_hash = NULL, upload_expires_at = NULL \
                     WHERE id = $1 AND deleted_at IS NULL AND state = 'pending' AND upload_token_hash = $2 AND upload_expires_at > $3 \
                     RETURNING {ASSET_COLS}"
                ),
                &[&id.0, &&token.0[..], &now],
            )
            .await
            .map_err(db_err)?;
        row.as_ref().map(asset_row).transpose()
    }

    async fn uploaded_asset(&self, id: AssetId) -> Result<Option<AssetRow>, DbError> {
        let c = self.conn().await?;
        let row = c
            .pg()
            .query_opt(
                &format!("SELECT {ASSET_COLS} FROM assets WHERE id = $1 AND state = 'uploaded' AND deleted_at IS NULL"),
                &[&id.0],
            )
            .await
            .map_err(db_err)?;
        row.as_ref().map(asset_row).transpose()
    }

    async fn set_user_active(&self, _sys: &SystemScope, user: UserId, active: bool) -> Result<bool, DbError> {
        let c = self.conn().await?;
        let n = c
            .pg()
            .execute("UPDATE users SET is_active = $2 WHERE id = $1", &[&user.0, &active])
            .await
            .map_err(db_err)?;
        Ok(n == 1)
    }

    async fn set_password_hash(&self, _sys: &SystemScope, user: UserId, password_hash: &str) -> Result<bool, DbError> {
        let c = self.conn().await?;
        let n = c
            .pg()
            .execute("UPDATE users SET password_hash = $2 WHERE id = $1", &[&user.0, &password_hash])
            .await
            .map_err(db_err)?;
        Ok(n == 1)
    }

    async fn seed_users(&self, _sys: &SystemScope, users: &[UserRecord]) -> Result<(), DbError> {
        let c = self.connector.checkout(true).await?;
        let cl = c.pg();
        cl.batch_execute("BEGIN").await.map_err(db_err)?;
        let res: Result<(), DbError> = async {
            for u in users {
                cl.execute(
                    "INSERT INTO users (id, email, password_hash, is_active) VALUES ($1, $2, $3, $4) \
                     ON CONFLICT (id) DO UPDATE SET email = EXCLUDED.email, password_hash = EXCLUDED.password_hash, \
                       is_active = EXCLUDED.is_active",
                    &[&u.id.0, &u.email.as_deref(), &u.password_hash.as_deref(), &u.is_active],
                )
                .await
                .map_err(db_err)?;
            }
            Ok(())
        }
        .await;
        match res {
            Ok(()) => cl.batch_execute("COMMIT").await.map_err(db_err),
            Err(e) => {
                let _ = cl.batch_execute("ROLLBACK").await;
                Err(e)
            }
        }
    }

    async fn begin(&self, _scope: &Scope) -> Result<Box<dyn Tx>, DbError> {
        // The connection is the transaction's own, but nothing is open on it yet: `BEGIN` waits for the first write
        // (FR-22, see the module docs), so `allocate` can run before it.
        let conn = self.connector.checkout(true).await?;
        Ok(Box::new(PgTx { conn: Some(conn), in_tx: false, connector: self.connector.clone() }))
    }

    async fn get_work_item(&self, scope: &Scope, id: WorkItemId, vis: Visibility) -> Result<Option<WorkItem>, DbError> {
        let (ws, project) = (scope.workspace_id(), scope.project_id());
        let hidden = match vis {
            Visibility::Visible => " AND deleted_at IS NULL AND archived_at IS NULL AND is_draft = false",
            Visibility::WithDeleted => "",
        };
        let sql =
            format!("SELECT {WI_COLS} FROM work_items WHERE workspace_id = $1 AND project_id = $2 AND id = $3{hidden}");
        let c = self.conn().await?;
        let row = c.pg().query_opt(&sql, &[&ws.0, &project.0, &id.0]).await.map_err(db_err)?;
        Ok(row.as_ref().map(work_item))
    }

    async fn work_item_page(&self, scope: &Scope, q: &WorkItemQuery) -> Result<WorkItemPage, DbError> {
        let (ws, project) = (scope.workspace_id(), scope.project_id());
        let limit = i64::from(q.limit);
        let offset = i64::from(q.page).saturating_mul(limit);
        let limited_guest = q.limited_guest.map(|u| u.0);
        // A limited guest sees what they made, unless the project shows its guests everything. The project's flag is read here,
        // in the statement, and not by a statement before it: that was two more for a guest (issue #39). It is one condition for
        // both statements below, because the second gives the total when the first finds no row, and they must agree on what
        // the guest may see. A project that is gone (`deleted_at`) shows nothing to a guest, as its detail read answers none.
        const GUEST_MAY_SEE: &str = "($4::uuid IS NULL OR w.created_by = $4 OR (SELECT p.guest_view_all_features \
             FROM projects p WHERE p.id = $2 AND p.workspace_id = $1 AND p.deleted_at IS NULL))";
        let c = self.conn().await?;
        // one statement: the rows of the page and one more (is there a next page), and how many rows match, in each of them
        let rows = c.pg()
            .query(
                &format!(
                    "SELECT {WIR_COLS}, count(*) OVER () AS total \
                     FROM work_items w LEFT JOIN states st ON st.id = w.state_id \
                     WHERE w.workspace_id = $1 AND w.project_id = $2 AND w.deleted_at IS NULL AND w.archived_at IS NULL AND NOT w.is_draft \
                       AND ($3::timestamptz IS NULL OR w.updated_at > $3) AND {GUEST_MAY_SEE} \
                     ORDER BY {} LIMIT $5 OFFSET $6",
                    work_item_order_sql(q.order)
                ),
                &[&ws.0, &project.0, &q.updated_after, &limited_guest, &(limit + 1), &offset],
            )
            .await
            .map_err(db_err)?;
        // MUTANT (control run only, feature `mutant_extra_stmt_per_row`): one more statement for every row the page returns,
        // the shape of Django's N+1. The budget test must notice (issue #39). Nothing but `scripts/mutants.sh` builds this.
        #[cfg(feature = "mutant_extra_stmt_per_row")]
        for _ in &rows {
            c.pg().query_one("SELECT 1", &[]).await.map_err(db_err)?;
        }
        let total: i64 = match rows.first() {
            Some(r) => r.get("total"),
            // a page past the end has no row to carry the count
            None => c.pg()
                .query_one(
                    &format!(
                        "SELECT count(*) FROM work_items w \
                         WHERE w.workspace_id = $1 AND w.project_id = $2 AND w.deleted_at IS NULL AND w.archived_at IS NULL AND NOT w.is_draft \
                           AND ($3::timestamptz IS NULL OR w.updated_at > $3) AND {GUEST_MAY_SEE}"
                    ),
                    &[&ws.0, &project.0, &q.updated_after, &limited_guest],
                )
                .await
                .map_err(db_err)?
                .get(0),
        };
        let has_next = rows.len() as i64 > limit;
        let rows = rows.iter().take(q.limit as usize).map(work_item_row).collect::<Result<Vec<_>, _>>()?;
        Ok(WorkItemPage { rows, has_next, total: total as u64 })
    }

    async fn work_item_row(&self, scope: &Scope, id: WorkItemId) -> Result<Option<WorkItemRow>, DbError> {
        let c = self.conn().await?;
        let row = c
            .pg()
            .query_opt(
                &format!(
                    "SELECT {WIR_COLS} FROM work_items w LEFT JOIN states st ON st.id = w.state_id \
                     WHERE w.workspace_id = $1 AND w.project_id = $2 AND w.id = $3 \
                       AND w.deleted_at IS NULL AND w.archived_at IS NULL AND NOT w.is_draft"
                ),
                &[&scope.workspace_id().0, &scope.project_id().0, &id.0],
            )
            .await
            .map_err(db_err)?;
        row.as_ref().map(work_item_row).transpose()
    }

    async fn work_item_by_sequence(&self, scope: &Scope, sequence_id: i64) -> Result<Option<WorkItemRow>, DbError> {
        let c = self.conn().await?;
        let row = c
            .pg()
            .query_opt(
                &format!(
                    "SELECT {WIR_COLS} FROM work_items w LEFT JOIN states st ON st.id = w.state_id \
                     WHERE w.workspace_id = $1 AND w.project_id = $2 AND w.sequence_id = $3 \
                       AND w.deleted_at IS NULL AND w.archived_at IS NULL AND NOT w.is_draft"
                ),
                &[&scope.workspace_id().0, &scope.project_id().0, &sequence_id],
            )
            .await
            .map_err(db_err)?;
        row.as_ref().map(work_item_row).transpose()
    }

    async fn work_item_by_external(
        &self,
        scope: &Scope,
        source: &str,
        id: &str,
    ) -> Result<Option<WorkItemRow>, DbError> {
        let c = self.conn().await?;
        let row = c
            .pg()
            .query_opt(
                &format!(
                    "SELECT {WIR_COLS} FROM work_items w LEFT JOIN states st ON st.id = w.state_id \
                     WHERE w.workspace_id = $1 AND w.project_id = $2 AND w.external_source = $3 AND w.external_id = $4 \
                       AND w.deleted_at IS NULL AND w.archived_at IS NULL AND NOT w.is_draft"
                ),
                &[&scope.workspace_id().0, &scope.project_id().0, &source, &id],
            )
            .await
            .map_err(db_err)?;
        row.as_ref().map(work_item_row).transpose()
    }

    async fn search_workspace_work_items(
        &self,
        scope: &WorkspaceScope,
        q: &WorkspaceSearch,
    ) -> Result<Vec<WorkItemHit>, DbError> {
        // `%`, `_` and `\` are literal in the text, so they are escaped for LIKE. The caller's own projects only (an active member
        // row that names the workspace), none archived or deleted; what counts is what the project's lists show.
        let pattern = like_pattern(&q.text);
        let project: Option<Uuid> = q.project.map(|p| p.0);
        let c = self.conn().await?;
        let rows = c.pg()
            .query(
                "SELECT w.id, w.name, w.sequence_id, w.project_id, p.identifier, ws.slug \
                 FROM work_items w \
                   JOIN projects p ON p.id = w.project_id AND p.workspace_id = w.workspace_id \
                     AND p.archived_at IS NULL AND p.deleted_at IS NULL \
                   JOIN workspaces ws ON ws.id = w.workspace_id \
                   JOIN project_members pm ON pm.project_id = w.project_id AND pm.workspace_id = w.workspace_id AND pm.user_id = $2 \
                 WHERE w.workspace_id = $1 AND w.deleted_at IS NULL AND w.archived_at IS NULL AND NOT w.is_draft \
                   AND ($3::uuid IS NULL OR w.project_id = $3) \
                   AND (w.name ILIKE $4 ESCAPE '\\' OR p.identifier ILIKE $4 ESCAPE '\\' OR w.sequence_id = ANY($5::bigint[])) \
                 ORDER BY w.created_at DESC, w.sequence_id DESC, w.id LIMIT $6",
                &[
                    &scope.workspace_id().0,
                    &scope.user_id().0,
                    &project,
                    &pattern,
                    &q.numbers,
                    &i64::from(q.limit),
                ],
            )
            .await
            .map_err(db_err)?;
        Ok(rows
            .iter()
            .map(|r| WorkItemHit {
                id: WorkItemId(r.get("id")),
                name: r.get("name"),
                sequence_id: r.get("sequence_id"),
                project_id: ProjectId(r.get("project_id")),
                project_identifier: r.get("identifier"),
                workspace_slug: WorkspaceSlug::new(r.get::<_, String>("slug")),
            })
            .collect())
    }

    async fn max_work_item_sort_order(&self, scope: &Scope, state: Option<Uuid>) -> Result<Option<i64>, DbError> {
        let c = self.conn().await?;
        let row = c.pg()
            .query_one(
                "SELECT max(sort_order) FROM work_items \
                 WHERE workspace_id = $1 AND project_id = $2 AND deleted_at IS NULL AND state_id IS NOT DISTINCT FROM $3",
                &[&scope.workspace_id().0, &scope.project_id().0, &state],
            )
            .await
            .map_err(db_err)?;
        Ok(row.get(0))
    }

    async fn seed_work_items(
        &self,
        _sys: &SystemScope,
        workspace: WorkspaceId,
        project: ProjectId,
        items: &[SeedWorkItem],
    ) -> Result<i64, DbError> {
        // One transaction on a connection of its own: the counter advance and the rows commit together. The rows go in as
        // columns (`unnest`), so 5,000 of them are one round trip, which is what makes 100k seedable through Hyperdrive.
        let n = items.len() as i64;
        let c = self.connector.checkout(true).await?;
        let cl = c.pg();
        cl.batch_execute("BEGIN").await.map_err(db_err)?;
        let res: Result<i64, DbError> = async {
            let r = cl
                .query_one(
                    "INSERT INTO project_sequences (workspace_id, project_id, last_sequence_id, last_sort_order) \
                     VALUES ($1, $2, $3, $4) \
                     ON CONFLICT (workspace_id, project_id) DO UPDATE SET \
                       last_sequence_id = project_sequences.last_sequence_id + $3, \
                       last_sort_order  = project_sequences.last_sort_order + $4 \
                     RETURNING last_sequence_id, last_sort_order",
                    &[&workspace.0, &project.0, &n, &(n * 10_000)],
                )
                .await
                .map_err(db_err)?;
            let (last_sequence, last_sort): (i64, i64) = (r.get(0), r.get(1));
            let (base_sequence, base_sort) = (last_sequence - n, last_sort - n * 10_000);
            let ids: Vec<Uuid> = items.iter().map(|i| i.id.0).collect();
            let names: Vec<String> = items.iter().map(|i| i.name.clone()).collect();
            let descriptions: Vec<Option<String>> = items.iter().map(|i| i.description.clone()).collect();
            let (created_by, created_at) = items.first().map(|i| (i.created_by.0, i.created_at)).ok_or_else(|| DbError::Backend("no items".into()))?;
            cl.execute(
                "INSERT INTO work_items (id, workspace_id, project_id, sequence_id, sort_order, name, description, created_by, created_at, is_draft, updated_at) \
                 SELECT t.id, $1, $2, $3 + t.ord, $4 + t.ord * 10000, t.name, t.description, $5, $6, false, $6 \
                 FROM unnest($7, $8, $9) WITH ORDINALITY AS t(id, name, description, ord)",
                &[&workspace.0, &project.0, &base_sequence, &base_sort, &created_by, &created_at, &ids, &names, &descriptions],
            )
            .await
            .map_err(db_err)?;
            Ok(base_sequence + 1)
        }
        .await;
        match res {
            Ok(first) => cl.batch_execute("COMMIT").await.map(|()| first).map_err(db_err),
            Err(e) => {
                let _ = cl.batch_execute("ROLLBACK").await;
                Err(e)
            }
        }
    }

    async fn insert_activity(&self, _sys: &SystemScope, a: &Activity) -> Result<bool, DbError> {
        let c = self.conn().await?;
        let n = c
            .pg()
            .execute(
                "INSERT INTO activity (id, event_id, workspace_id, project_id, entity_id, verb, actor_id, created_at) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8) ON CONFLICT (event_id) DO NOTHING",
                &[
                    &a.id.0,
                    &a.event_id.0,
                    &a.workspace_id.0,
                    &a.project_id.0,
                    &a.entity_id,
                    &a.verb,
                    &a.actor_id.map(|u| u.0),
                    &a.created_at,
                ],
            )
            .await
            .map_err(db_err)?;
        Ok(n == 1)
    }

    async fn list_activity(&self, scope: &Scope, entity: Option<WorkItemId>) -> Result<Vec<Activity>, DbError> {
        let (ws, project) = (scope.workspace_id(), scope.project_id());
        let c = self.conn().await?;
        let entity: Option<Uuid> = entity.map(|e| e.0);
        let rows = c
            .pg()
            .query(
                "SELECT id, event_id, workspace_id, project_id, entity_id, verb, actor_id, created_at \
                 FROM activity WHERE workspace_id = $1 AND project_id = $2 \
                 AND ($3::uuid IS NULL OR entity_id = $3::uuid) ORDER BY created_at, id",
                &[&ws.0, &project.0, &entity],
            )
            .await
            .map_err(db_err)?;
        Ok(rows.iter().map(activity).collect())
    }

    /// Oldest first by time, then in the order the rows were written (`seq`, which `touch_work_item_activity` never moves). The row
    /// carries the workspace and project it was written for, so another scope's read finds none.
    async fn work_item_history(
        &self,
        scope: &Scope,
        id: WorkItemId,
        after: Option<Timestamp>,
    ) -> Result<Vec<WorkItemActivity>, DbError> {
        let c = self.conn().await?;
        let rows = c.pg()
            .query(
                &format!(
                    "SELECT {WIA_COLS} FROM work_item_activities \
                     WHERE workspace_id = $1 AND project_id = $2 AND work_item_id = $3 AND ($4::timestamptz IS NULL OR created_at > $4) \
                     ORDER BY created_at, seq"
                ),
                &[&scope.workspace_id().0, &scope.project_id().0, &id.0, &after],
            )
            .await
            .map_err(db_err)?;
        Ok(rows.iter().map(work_item_activity).collect())
    }

    async fn latest_work_item_activity(
        &self,
        _sys: &SystemScope,
        workspace: WorkspaceId,
        project: ProjectId,
        id: WorkItemId,
    ) -> Result<Option<WorkItemActivity>, DbError> {
        let c = self.conn().await?;
        let row = c.pg()
            .query_opt(
                &format!(
                    "SELECT {WIA_COLS} FROM work_item_activities \
                     WHERE workspace_id = $1 AND project_id = $2 AND work_item_id = $3 ORDER BY created_at DESC, seq DESC LIMIT 1"
                ),
                &[&workspace.0, &project.0, &id.0],
            )
            .await
            .map_err(db_err)?;
        Ok(row.as_ref().map(work_item_activity))
    }

    async fn touch_work_item_activity(
        &self,
        _sys: &SystemScope,
        id: ActivityId,
        at: Timestamp,
    ) -> Result<bool, DbError> {
        let c = self.conn().await?;
        let n = c
            .pg()
            .execute("UPDATE work_item_activities SET created_at = $2 WHERE id = $1", &[&id.0, &at])
            .await
            .map_err(db_err)?;
        Ok(n == 1)
    }

    /// ONE statement, so all of the rows or none: the rows go in as columns (`unnest`, in the order given, which is the order of
    /// `seq`), a row whose key (`event_id`, `ordinal`) is there is skipped (`ON CONFLICT DO NOTHING`, which also skips the second of two
    /// rows of one batch with one key), and the statement answers how many rows it refused and how many it wrote. A row is refused when
    /// its work item is not in the workspace and project it names, or its actor is no user; the first the foreign keys cannot say (the
    /// item's key is its id alone) and the second they would not see on a row that is skipped, so both are asked of every row before
    /// any is written, as the memory adapter asks them.
    async fn insert_work_item_activities(&self, _sys: &SystemScope, rows: &[WorkItemActivity]) -> Result<u64, DbError> {
        if rows.is_empty() {
            return Ok(0);
        }
        let ids: Vec<Uuid> = rows.iter().map(|r| r.id.0).collect();
        let events: Vec<Uuid> = rows.iter().map(|r| r.event_id.0).collect();
        let ordinals: Vec<i32> = rows.iter().map(|r| r.ordinal).collect();
        let workspaces: Vec<Uuid> = rows.iter().map(|r| r.workspace_id.0).collect();
        let projects: Vec<Uuid> = rows.iter().map(|r| r.project_id.0).collect();
        let items: Vec<Uuid> = rows.iter().map(|r| r.work_item_id.0).collect();
        let actors: Vec<Option<Uuid>> = rows.iter().map(|r| r.actor_id.map(|u| u.0)).collect();
        let verbs: Vec<String> = rows.iter().map(|r| r.verb.clone()).collect();
        let fields: Vec<Option<String>> = rows.iter().map(|r| r.field.clone()).collect();
        let olds: Vec<Option<String>> = rows.iter().map(|r| r.old_value.clone()).collect();
        let news: Vec<Option<String>> = rows.iter().map(|r| r.new_value.clone()).collect();
        let comments: Vec<String> = rows.iter().map(|r| r.comment.clone()).collect();
        let old_ids: Vec<Option<Uuid>> = rows.iter().map(|r| r.old_identifier).collect();
        let new_ids: Vec<Option<Uuid>> = rows.iter().map(|r| r.new_identifier).collect();
        let epochs: Vec<i64> = rows.iter().map(|r| r.epoch).collect();
        let times: Vec<Timestamp> = rows.iter().map(|r| r.created_at).collect();
        let c = self.conn().await?;
        let r = c.pg()
            .query_one(
                "WITH input AS ( \
                   SELECT * FROM unnest($1::uuid[], $2::uuid[], $3::int4[], $4::uuid[], $5::uuid[], $6::uuid[], $7::uuid[], $8::text[], $9::text[], \
                                        $10::text[], $11::text[], $12::text[], $13::uuid[], $14::uuid[], $15::int8[], $16::timestamptz[]) \
                     WITH ORDINALITY AS t(id, event_id, ordinal, workspace_id, project_id, work_item_id, actor_id, verb, field, old_value, \
                                          new_value, comment, old_identifier, new_identifier, epoch, created_at, place) \
                 ), refused AS ( \
                   SELECT count(*) FILTER (WHERE NOT EXISTS ( \
                            SELECT 1 FROM work_items w WHERE w.id = i.work_item_id AND w.workspace_id = i.workspace_id AND w.project_id = i.project_id)) AS strays, \
                          count(*) FILTER (WHERE i.actor_id IS NOT NULL AND NOT EXISTS (SELECT 1 FROM users u WHERE u.id = i.actor_id)) AS unknown_actors \
                   FROM input i \
                 ), written AS ( \
                   INSERT INTO work_item_activities (id, event_id, ordinal, workspace_id, project_id, work_item_id, actor_id, verb, field, old_value, \
                                                     new_value, comment, old_identifier, new_identifier, epoch, created_at) \
                   SELECT i.id, i.event_id, i.ordinal, i.workspace_id, i.project_id, i.work_item_id, i.actor_id, i.verb, i.field, i.old_value, \
                          i.new_value, i.comment, i.old_identifier, i.new_identifier, i.epoch, i.created_at \
                   FROM input i WHERE (SELECT strays + unknown_actors FROM refused) = 0 ORDER BY i.place \
                   ON CONFLICT (event_id, ordinal) DO NOTHING \
                   RETURNING 1 \
                 ) \
                 SELECT (SELECT strays FROM refused) AS strays, (SELECT unknown_actors FROM refused) AS unknown_actors, (SELECT count(*) FROM written) AS written",
                &[
                    &ids, &events, &ordinals, &workspaces, &projects, &items, &actors, &verbs, &fields, &olds, &news, &comments, &old_ids,
                    &new_ids, &epochs, &times,
                ],
            )
            .await
            .map_err(db_err)?;
        let (strays, unknown_actors, written): (i64, i64, i64) =
            (r.get("strays"), r.get("unknown_actors"), r.get("written"));
        if strays > 0 {
            return Err(DbError::Backend(format!("{strays} history row(s) name a work item that is not in the workspace and project they name (work_item_activities_work_item_fkey)")));
        }
        if unknown_actors > 0 {
            return Err(DbError::Backend(format!(
                "{unknown_actors} history row(s) name an actor who is no user (work_item_activities_actor_id_fkey)"
            )));
        }
        Ok(written as u64)
    }

    async fn undispatched_outbox(&self, _sys: &SystemScope, limit: u32) -> Result<Vec<OutboxRow>, DbError> {
        let c = self.conn().await?;
        let rows = c
            .pg()
            .query(
                "SELECT envelope, created_at, attempts FROM outbox WHERE dispatched_at IS NULL \
                 ORDER BY created_at, event_id LIMIT $1",
                &[&(limit as i64)],
            )
            .await
            .map_err(db_err)?;
        rows.iter()
            .map(|r| {
                let v: serde_json::Value = r.get("envelope");
                let envelope: Envelope =
                    serde_json::from_value(v).map_err(|e| DbError::Backend(format!("bad envelope: {e}")))?;
                Ok(OutboxRow {
                    envelope,
                    created_at: r.get("created_at"),
                    dispatched_at: None,
                    attempts: r.get("attempts"),
                })
            })
            .collect()
    }

    async fn mark_outbox_dispatched(&self, _sys: &SystemScope, event: EventId, at: Timestamp) -> Result<(), DbError> {
        let c = self.conn().await?;
        c.pg()
            .execute(
                "UPDATE outbox SET dispatched_at = $2, attempts = attempts + 1 WHERE event_id = $1",
                &[&event.0, &at],
            )
            .await
            .map_err(db_err)?;
        Ok(())
    }

    async fn outbox_depth(&self, _sys: &SystemScope) -> Result<u64, DbError> {
        let c = self.conn().await?;
        let r = c
            .pg()
            .query_one("SELECT count(*) AS n FROM outbox WHERE dispatched_at IS NULL", &[])
            .await
            .map_err(db_err)?;
        Ok(r.get::<_, i64>("n") as u64)
    }

    async fn seed(&self, _sys: &SystemScope, f: &SeedFixture) -> Result<(), DbError> {
        // `Tx` only exposes slice operations, so the fixture goes through a connection directly.
        let c = self.connector.checkout(true).await?;
        let cl = c.pg();
        cl.batch_execute("BEGIN").await.map_err(db_err)?;
        let res: Result<(), DbError> = async {
            // users that exist by id only; a row that is there already (with an email, say) is left as it is
            for u in &f.users {
                cl.execute("INSERT INTO users (id) VALUES ($1) ON CONFLICT (id) DO NOTHING", &[&u.0])
                    .await
                    .map_err(db_err)?;
            }
            for w in &f.workspaces {
                cl.execute(
                    // a seeded workspace is named after its slug and has no owner
                    "INSERT INTO workspaces (id, slug, name) VALUES ($1, $2, $2) \
                     ON CONFLICT (id) DO UPDATE SET slug = EXCLUDED.slug",
                    &[&w.id.0, &w.slug.as_str()],
                )
                .await
                .map_err(db_err)?;
            }
            for m in &f.workspace_members {
                cl.execute(
                    "INSERT INTO workspace_members (workspace_id, user_id) VALUES ($1, $2) ON CONFLICT DO NOTHING",
                    &[&m.workspace_id.0, &m.user_id.0],
                )
                .await
                .map_err(db_err)?;
            }
            // the role of a member the seed names, made when the seed did not have them
            for r in &f.workspace_roles {
                cl.execute(
                    "INSERT INTO workspace_members (workspace_id, user_id, role) VALUES ($1, $2, $3) \
                     ON CONFLICT (workspace_id, user_id) DO UPDATE SET role = EXCLUDED.role",
                    &[&r.workspace_id.0, &r.user_id.0, &r.role.as_str()],
                )
                .await
                .map_err(db_err)?;
            }
            for p in &f.projects {
                // a project with no `project_details` entry is what a project made with no options is; with no time given it
                // is made at `now()`, and an archived one was archived when it was made
                let details = f.project_details.iter().find(|d| d.project_id == p.id).cloned().unwrap_or_else(|| SeedProjectDetails::of(p.id));
                let identifier = details.identifier.unwrap_or_else(|| default_project_identifier(p.id));
                cl.execute(
                    "INSERT INTO projects (id, workspace_id, name, identifier, network, archived_at, project_lead, created_by, \
                       created_at, updated_at, description, default_state) \
                     VALUES ($1, $2, $3, $4, $5, CASE WHEN $6 THEN COALESCE($9, now()) END, $7, $8, COALESCE($9, now()), \
                       COALESCE($9, now()), $10, $11) \
                     ON CONFLICT (id) DO NOTHING",
                    &[
                        &p.id.0,
                        &p.workspace_id.0,
                        &p.name,
                        &identifier,
                        &details.network.as_str(),
                        &details.archived,
                        &details.project_lead.map(|u| u.0),
                        &details.created_by.map(|u| u.0),
                        &details.created_at,
                        &details.description,
                        &details.default_state,
                    ],
                )
                .await
                .map_err(db_err)?;
            }
            for m in &f.memberships {
                // a seeded member row has an id made from the project and the user (as the memory adapter makes it), the epoch for
                // its time, and the settings the pin starts with unset (`{}`)
                let id = Uuid::from_u128(m.project_id.0.as_u128() ^ m.user_id.0.as_u128().rotate_left(64));
                cl.execute(
                    "INSERT INTO project_members (id, project_id, workspace_id, user_id, role, created_at, updated_at) \
                     VALUES ($1, $2, $3, $4, $5, to_timestamp(0), to_timestamp(0)) \
                     ON CONFLICT (project_id, user_id) DO UPDATE SET role = EXCLUDED.role",
                    &[&id, &m.project_id.0, &m.workspace_id.0, &m.user_id.0, &m.role.as_str()],
                )
                .await
                .map_err(db_err)?;
            }
            for o in &f.project_sort_orders {
                let id = Uuid::from_u128(o.project_id.0.as_u128().rotate_left(32) ^ o.user_id.0.as_u128());
                let n = cl
                    .execute(
                        "INSERT INTO project_user_properties (id, project_id, workspace_id, user_id, sort_order, created_at, updated_at) \
                         SELECT $1, p.id, p.workspace_id, $3, $4, to_timestamp(0), to_timestamp(0) FROM projects p WHERE p.id = $2 \
                         ON CONFLICT (project_id, user_id) DO UPDATE SET sort_order = EXCLUDED.sort_order",
                        &[&id, &o.project_id.0, &o.user_id.0, &o.sort_order],
                    )
                    .await
                    .map_err(db_err)?;
                if n == 0 {
                    return Err(DbError::Backend("project_user_properties_project_id_fkey".into()));
                }
            }
            for st in &f.states {
                cl.execute(
                    "INSERT INTO states (id, workspace_id, project_id, name, color, sequence, state_group, is_default, created_at, updated_at) \
                     VALUES ($1, $2, $3, $4, $5, $6, $7, $8, to_timestamp(0), to_timestamp(0)) ON CONFLICT (id) DO NOTHING",
                    &[&st.id, &st.workspace_id.0, &st.project_id.0, &st.name, &st.color, &st.sequence, &st.group.as_str(), &st.is_default],
                )
                .await
                .map_err(db_err)?;
            }
            for l in &f.labels {
                cl.execute(
                    "INSERT INTO labels (id, workspace_id, project_id, parent_id, name, color, sort_order, created_at, updated_at) \
                     VALUES ($1, $2, $3, $4, $5, $6, $7, to_timestamp(0), to_timestamp(0)) ON CONFLICT (id) DO NOTHING",
                    &[&l.id, &l.workspace_id.0, &l.project_id.0, &l.parent_id, &l.name, &l.color, &l.sort_order],
                )
                .await
                .map_err(db_err)?;
            }
            // Work items, in the order the fixture has them (a parent comes before its children: the foreign key is checked as
            // each row goes in). The row, what a request would have given it and the project's counters are written together,
            // so the number and the place a project hands out next come after the largest the fixture gave.
            for w in &f.work_items {
                let priority = w.priority.as_str();
                cl.execute(
                    "INSERT INTO work_items (id, workspace_id, project_id, sequence_id, sort_order, name, description, created_by, \
                       created_at, updated_at, is_draft, state_id, priority, start_date, target_date, completed_at, \
                       description_html, parent_id) \
                     VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $9, false, $10, $11, $12, $13, $14, $15, $16)",
                    &[
                        &w.id.0,
                        &w.workspace_id.0,
                        &w.project_id.0,
                        &w.sequence_id,
                        &w.sort_order,
                        &w.name,
                        &w.description,
                        &w.created_by.map(|u| u.0),
                        &w.created_at,
                        &w.state_id,
                        &priority,
                        &w.start_date,
                        &w.target_date,
                        &w.completed_at,
                        &w.description_html,
                        &w.parent_id,
                    ],
                )
                .await
                .map_err(db_err)?;
                let people: Vec<Uuid> = w.assignee_ids.iter().map(|u| u.0).collect();
                cl.execute(
                    "INSERT INTO work_item_assignees (work_item_id, workspace_id, project_id, user_id, created_at) \
                     SELECT $1, $2, $3, t.user_id, $5 FROM unnest($4::uuid[]) AS t(user_id) ON CONFLICT (work_item_id, user_id) DO NOTHING",
                    &[&w.id.0, &w.workspace_id.0, &w.project_id.0, &people, &w.created_at],
                )
                .await
                .map_err(db_err)?;
                cl.execute(
                    "INSERT INTO work_item_labels (work_item_id, workspace_id, project_id, label_id, created_at) \
                     SELECT $1, $2, $3, t.label_id, $5 FROM unnest($4::uuid[]) AS t(label_id) ON CONFLICT (work_item_id, label_id) DO NOTHING",
                    &[&w.id.0, &w.workspace_id.0, &w.project_id.0, &w.label_ids, &w.created_at],
                )
                .await
                .map_err(db_err)?;
                cl.execute(
                    "INSERT INTO project_sequences (workspace_id, project_id, last_sequence_id, last_sort_order) \
                     VALUES ($1, $2, $3, $4) \
                     ON CONFLICT (workspace_id, project_id) DO UPDATE SET \
                       last_sequence_id = GREATEST(project_sequences.last_sequence_id, EXCLUDED.last_sequence_id), \
                       last_sort_order  = GREATEST(project_sequences.last_sort_order, EXCLUDED.last_sort_order)",
                    &[&w.workspace_id.0, &w.project_id.0, &w.sequence_id, &w.sort_order],
                )
                .await
                .map_err(db_err)?;
            }
            Ok(())
        }
        .await;
        match res {
            Ok(()) => cl.batch_execute("COMMIT").await.map_err(db_err),
            Err(e) => {
                let _ = cl.batch_execute("ROLLBACK").await;
                Err(e)
            }
        }
    }

    async fn reset(&self, _sys: &SystemScope) -> Result<(), DbError> {
        // One statement, so one transaction: either every table is empty or none is. The tables are looked up, not listed, so
        // that a table a later migration adds is emptied without anyone remembering this function. The migrations are plain
        // scripts that run again on every start and leave no bookkeeping table, and what they insert is a backfill of rows that
        // are there already, so nothing here is something the schema needs to keep.
        const RESET: &str = "DO $$ \
            DECLARE tables text; \
            BEGIN \
              SELECT string_agg(format('%I.%I', schemaname, tablename), ', ') INTO tables FROM pg_tables WHERE schemaname = 'public'; \
              IF tables IS NOT NULL THEN \
                EXECUTE 'TRUNCATE TABLE ' || tables || ' RESTART IDENTITY CASCADE'; \
              END IF; \
            END $$";
        let c = self.connector.checkout(true).await?;
        c.pg().batch_execute(RESET).await.map_err(db_err)
    }
}

pub struct PgTx {
    conn: Option<Box<dyn ClientHandle>>,
    /// `BEGIN` has been sent on `conn`: the first write opens the transaction.
    in_tx: bool,
    connector: Arc<dyn Connector>,
}

/// What a call on a transaction that has lost its connection answers. `ensure_tx` drops the connection when `BEGIN` fails (its state
/// is unknown, so it is not reused); a further call on that transaction must fail, and never panic.
const TX_HAS_NO_CONNECTION: &str = "transaction already finished: its connection was discarded";

impl PgTx {
    fn client(&self) -> Result<Pg<'_>, DbError> {
        match &self.conn {
            Some(c) => Ok(c.pg()),
            None => Err(DbError::Backend(TX_HAS_NO_CONNECTION.into())),
        }
    }

    /// Open the transaction on first use. If `BEGIN` fails the connection's state is unknown, so it is not reused.
    async fn ensure_tx(&mut self) -> Result<(), DbError> {
        if self.in_tx {
            return Ok(());
        }
        match self.client()?.batch_execute("BEGIN").await {
            Ok(()) => {
                self.in_tx = true;
                Ok(())
            }
            Err(e) => {
                if let Some(c) = self.conn.take() {
                    c.discard();
                }
                Err(db_err(e))
            }
        }
    }
}

impl Drop for PgTx {
    fn drop(&mut self) {
        // Dropped without commit/rollback (e.g. the request future was cancelled): never return a connection that is
        // still inside a transaction to the pool. One that never opened a transaction is clean and goes back as it is.
        if let Some(c) = self.conn.take() {
            if self.in_tx {
                c.discard();
            }
        }
    }
}

#[async_trait]
impl Tx for PgTx {
    /// A scope whose project is outside its workspace violates `project_sequences_tenant_fkey`
    /// (migration 0002): `Backend`, and nothing is taken. `authorize` never mints one; a test can.
    ///
    /// FR-22: the upsert is a transaction of its own (one statement, autocommit), so the counter row is locked for one
    /// round trip. Before the first write that is this transaction's own connection; if a write has opened the
    /// transaction already (a second `allocate` in one create), another connection takes the number, because on this
    /// one the upsert would join the transaction and keep the lock until it ends.
    async fn allocate(&mut self, scope: &Scope) -> Result<Allocation, DbError> {
        const UPSERT: &str =
            "INSERT INTO project_sequences (workspace_id, project_id, last_sequence_id, last_sort_order) \
             VALUES ($1, $2, 1, 10000) \
             ON CONFLICT (workspace_id, project_id) DO UPDATE SET \
               last_sequence_id = project_sequences.last_sequence_id + 1, \
               last_sort_order  = project_sequences.last_sort_order + 10000 \
             RETURNING last_sequence_id, last_sort_order";
        let (ws, project) = (scope.workspace_id(), scope.project_id());
        let r = if self.in_tx {
            let other = self.connector.checkout(true).await?;
            let r = other.pg().query_one(UPSERT, &[&ws.0, &project.0]).await;
            drop(other); // autocommit: nothing is open on it, it is clean
            r
        } else {
            self.client()?.query_one(UPSERT, &[&ws.0, &project.0]).await
        }
        .map_err(db_err)?;
        Ok(Allocation { sequence_id: r.get(0), sort_order: r.get(1) })
    }

    async fn insert_work_item(&mut self, scope: &Scope, w: &WorkItem) -> Result<(), DbError> {
        check_tenant(scope, w.workspace_id, Some(w.project_id))?;
        self.ensure_tx().await?;
        self.client()?
            .execute(
                &format!(
                    // a work item has been changed when it was made: `updated_at` is the creation time until a change moves it
                    "INSERT INTO work_items ({WI_COLS}, updated_at) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$11)"
                ),
                &[
                    &w.id.0,
                    &w.workspace_id.0,
                    &w.project_id.0,
                    &w.sequence_id,
                    &w.sort_order,
                    &w.name,
                    &w.description,
                    &w.external_source,
                    &w.external_id,
                    &w.created_by.map(|u| u.0),
                    &w.created_at,
                    &w.archived_at,
                    &w.is_draft,
                    &w.deleted_at,
                ],
            )
            .await
            .map_err(db_err)?;
        Ok(())
    }

    async fn insert_work_item_attrs(
        &mut self,
        scope: &Scope,
        id: WorkItemId,
        attrs: &WorkItemAttrs,
    ) -> Result<(), DbError> {
        let (ws, project) = (scope.workspace_id(), scope.project_id());
        self.ensure_tx().await?;
        let priority = attrs.priority.as_str();
        let n = self
            .client()?
            .execute(
                "UPDATE work_items SET state_id = $4, priority = $5, start_date = $6, target_date = $7, completed_at = $8, \
                   updated_at = $9, updated_by = $10, description_html = $11, parent_id = $12, point = $13 \
                 WHERE workspace_id = $1 AND project_id = $2 AND id = $3",
                &[
                    &ws.0,
                    &project.0,
                    &id.0,
                    &attrs.state_id,
                    &priority,
                    &attrs.start_date,
                    &attrs.target_date,
                    &attrs.completed_at,
                    &attrs.updated_at,
                    &attrs.updated_by.map(|u| u.0),
                    &attrs.description_html,
                    &attrs.parent_id,
                    &attrs.point,
                ],
            )
            .await
            .map_err(db_err)?;
        if n != 1 {
            return Err(DbError::Backend("work item attributes for a work item that is not there".into()));
        }
        let people: Vec<Uuid> = attrs.assignee_ids.iter().map(|u| u.0).collect();
        self.client()?
            .execute(
                "INSERT INTO work_item_assignees (work_item_id, workspace_id, project_id, user_id, created_at) \
                 SELECT $1, $2, $3, t.user_id, $5 FROM unnest($4::uuid[]) AS t(user_id) ON CONFLICT (work_item_id, user_id) DO NOTHING",
                &[&id.0, &ws.0, &project.0, &people, &attrs.updated_at],
            )
            .await
            .map_err(db_err)?;
        self.client()?
            .execute(
                "INSERT INTO work_item_labels (work_item_id, workspace_id, project_id, label_id, created_at) \
                 SELECT $1, $2, $3, t.label_id, $5 FROM unnest($4::uuid[]) AS t(label_id) ON CONFLICT (work_item_id, label_id) DO NOTHING",
                &[&id.0, &ws.0, &project.0, &attrs.label_ids, &attrs.updated_at],
            )
            .await
            .map_err(db_err)?;
        Ok(())
    }

    /// The row is the one a read finds (`work_item_row`: not deleted, archived or a draft), and none is `Ok(false)` with nothing else
    /// written: the update of the columns is the first statement and the people and labels follow only when it changed a row. What
    /// the constraints refuse (a state, an editor, a person or a label that is not there; a start after a target) is a `Backend`
    /// error, as for [`Tx::insert_work_item_attrs`], and Postgres then holds the whole transaction aborted until it is rolled back.
    /// The people and the labels, when named, are replaced: every row of the item goes, those of people who have left the project
    /// too, and the named ones are written once each.
    async fn update_work_item(
        &mut self,
        scope: &Scope,
        id: WorkItemId,
        update: &WorkItemUpdate,
    ) -> Result<bool, DbError> {
        let (ws, project) = (scope.workspace_id(), scope.project_id());
        self.ensure_tx().await?;
        let priority = update.priority.as_str();
        let n = self
            .client()?
            .execute(
                "UPDATE work_items SET name = $4, state_id = $5, priority = $6, start_date = $7, target_date = $8, completed_at = $9, \
                   updated_at = $10, updated_by = $11, description_html = $12, parent_id = $13, point = $14, \
                   external_source = $15, external_id = $16 \
                 WHERE workspace_id = $1 AND project_id = $2 AND id = $3 AND deleted_at IS NULL AND archived_at IS NULL AND is_draft = false",
                &[
                    &ws.0,
                    &project.0,
                    &id.0,
                    &update.name,
                    &update.state_id,
                    &priority,
                    &update.start_date,
                    &update.target_date,
                    &update.completed_at,
                    &update.updated_at,
                    &update.updated_by.0,
                    &update.description_html,
                    &update.parent_id,
                    &update.point,
                    &update.external_source,
                    &update.external_id,
                ],
            )
            .await
            .map_err(db_err)?;
        if n != 1 {
            return Ok(false);
        }
        if let Some(people) = &update.assignee_ids {
            let people: Vec<Uuid> = people.iter().map(|u| u.0).collect();
            self.client()?
                .execute("DELETE FROM work_item_assignees WHERE work_item_id = $1", &[&id.0])
                .await
                .map_err(db_err)?;
            self.client()?
                .execute(
                    "INSERT INTO work_item_assignees (work_item_id, workspace_id, project_id, user_id, created_at) \
                     SELECT $1, $2, $3, t.user_id, $5 FROM unnest($4::uuid[]) AS t(user_id) ON CONFLICT (work_item_id, user_id) DO NOTHING",
                    &[&id.0, &ws.0, &project.0, &people, &update.updated_at],
                )
                .await
                .map_err(db_err)?;
        }
        if let Some(labels) = &update.label_ids {
            self.client()?
                .execute("DELETE FROM work_item_labels WHERE work_item_id = $1", &[&id.0])
                .await
                .map_err(db_err)?;
            self.client()?
                .execute(
                    "INSERT INTO work_item_labels (work_item_id, workspace_id, project_id, label_id, created_at) \
                     SELECT $1, $2, $3, t.label_id, $5 FROM unnest($4::uuid[]) AS t(label_id) ON CONFLICT (work_item_id, label_id) DO NOTHING",
                    &[&id.0, &ws.0, &project.0, labels, &update.updated_at],
                )
                .await
                .map_err(db_err)?;
        }
        Ok(true)
    }

    async fn soft_delete_work_item(&mut self, scope: &Scope, id: WorkItemId, at: Timestamp) -> Result<bool, DbError> {
        let (ws, project) = (scope.workspace_id(), scope.project_id());
        self.ensure_tx().await?;
        // The item and, in one statement, every live item under it. The walk is a UNION (not UNION ALL), which drops a row it has
        // met: the pin lets an item be its own parent, and a loop must end the walk, not feed it.
        let row = self
            .client()?
            .query_one(
                "WITH RECURSIVE tree AS ( \
                     SELECT id FROM work_items WHERE id = $3 AND workspace_id = $1 AND project_id = $2 AND deleted_at IS NULL \
                   UNION \
                     SELECT c.id FROM work_items c JOIN tree t ON c.parent_id = t.id \
                     WHERE c.workspace_id = $1 AND c.project_id = $2 AND c.deleted_at IS NULL \
                 ), \
                 gone AS (UPDATE work_items SET deleted_at = $4 WHERE id IN (SELECT id FROM tree) RETURNING id) \
                 SELECT EXISTS (SELECT 1 FROM gone WHERE id = $3) AS deleted",
                &[&ws.0, &project.0, &id.0, &at],
            )
            .await
            .map_err(db_err)?;
        Ok(row.get("deleted"))
    }

    async fn insert_outbox(&mut self, scope: &Scope, row: &OutboxRow) -> Result<(), DbError> {
        check_tenant(scope, row.envelope.workspace_id, row.envelope.project_id())?;
        let envelope = serde_json::to_value(&row.envelope).map_err(|e| DbError::Backend(e.to_string()))?;
        let params: Params = &[&row.envelope.event_id.0, &row.envelope.workspace_id.0, &envelope, &row.created_at];
        self.ensure_tx().await?;
        self.client()?
            .execute(
                "INSERT INTO outbox (event_id, workspace_id, envelope, created_at, dispatched_at, attempts) \
                 VALUES ($1, $2, $3, $4, NULL, 0)",
                params,
            )
            .await
            .map_err(db_err)?;
        Ok(())
    }

    async fn commit(self: Box<Self>) -> Result<(), DbError> {
        let mut this = *self;
        let in_tx = this.in_tx;
        let Some(conn) = this.conn.take() else {
            // The connection was discarded (a failed `BEGIN`): nothing can be committed, and the caller is not told it was.
            return Err(DbError::Backend(TX_HAS_NO_CONNECTION.into()));
        };
        if !in_tx {
            // Nothing was written, so no transaction was opened: the connection is clean and goes back as it is.
            return Ok(());
        }
        match conn.pg().batch_execute("COMMIT").await {
            Ok(()) => Ok(()),
            Err(e) => {
                conn.discard();
                Err(db_err(e))
            }
        }
    }

    async fn rollback(self: Box<Self>) -> Result<(), DbError> {
        let mut this = *self;
        let in_tx = this.in_tx;
        let Some(conn) = this.conn.take() else {
            // The connection was discarded (a failed `BEGIN`), which closed it and so rolled back whatever it held: nothing to do.
            return Ok(());
        };
        if !in_tx {
            return Ok(());
        }
        match conn.pg().batch_execute("ROLLBACK").await {
            Ok(()) => Ok(()),
            Err(e) => {
                conn.discard();
                Err(db_err(e))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::task::{Context, Poll, Waker};

    use super::*;

    /// A `Connector` that is never asked: the transactions below have no connection, which is the point.
    struct NoConnector;

    #[async_trait]
    impl Connector for NoConnector {
        async fn checkout(&self, _exclusive: bool) -> Result<Box<dyn ClientHandle>, DbError> {
            Err(DbError::Backend("no connector in this test".into()))
        }
    }

    /// Runs a future that has nothing to wait for.
    fn ready<T>(f: impl Future<Output = T>) -> T {
        let mut f = std::pin::pin!(f);
        match f.as_mut().poll(&mut Context::from_waker(Waker::noop())) {
            Poll::Ready(v) => v,
            Poll::Pending => panic!("the future was not ready"),
        }
    }

    /// A transaction whose connection has been discarded: what `ensure_tx` leaves behind when `BEGIN` fails.
    fn discarded() -> PgTx {
        PgTx { conn: None, in_tx: false, connector: Arc::new(NoConnector) }
    }

    // Pinned regression (issue #40, ADR-0006): each of these was `expect("transaction already finished")`, a panic on request-driven
    // control flow (a first write whose `BEGIN` failed, then any further call on the same transaction).
    #[test]
    fn a_call_on_a_transaction_that_lost_its_connection_is_an_error() {
        let tx = discarded();
        let r = tx.client();
        assert!(matches!(&r, Err(DbError::Backend(m)) if m == TX_HAS_NO_CONNECTION));
    }

    #[test]
    fn commit_on_a_transaction_that_lost_its_connection_is_an_error() {
        let r = ready(Box::new(discarded()).commit());
        assert!(matches!(&r, Err(DbError::Backend(m)) if m == TX_HAS_NO_CONNECTION), "{r:?}");
    }

    #[test]
    fn rollback_on_a_transaction_that_lost_its_connection_is_a_no_op() {
        let r = ready(Box::new(discarded()).rollback());
        assert!(r.is_ok(), "{r:?}");
    }
}
