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
    client.batch_execute(MIGRATION_0012).await
}

pub trait ClientHandle: Send {
    fn client(&self) -> &Client;
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

fn db_err(e: tokio_postgres::Error) -> DbError {
    if e.code() == Some(&SqlState::UNIQUE_VIOLATION) {
        let c = e.as_db_error().and_then(|d| d.constraint()).unwrap_or("?");
        return DbError::Conflict(format!("unique constraint '{c}' violated"));
    }
    // `Display` of a database error is just "db error": the server's own words say which statement and constraint
    DbError::Backend(match e.as_db_error() {
        Some(d) => format!("{} ({}{})", d.message(), d.code().code(), d.constraint().map(|c| format!(", constraint {c}")).unwrap_or_default()),
        None => e.to_string(),
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
    serde_json::Value => Type::JSONB,
    f64 => Type::FLOAT8,
);
/// Whole columns at once, for the bulk seed's `unnest`.
impl PgType for Vec<Uuid> {
    const TY: Type = Type::UUID_ARRAY;
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

/// A connected client whose statements all go through the single-pipeline typed calls.
#[derive(Clone, Copy)]
pub struct Pg<'a>(pub &'a Client);

impl Pg<'_> {
    pub async fn query(&self, sql: &str, p: Params<'_>) -> Result<Vec<Row>, tokio_postgres::Error> {
        self.0.query_typed(sql, &typed(p)).await
    }
    pub async fn query_opt(&self, sql: &str, p: Params<'_>) -> Result<Option<Row>, tokio_postgres::Error> {
        self.0.query_typed_opt(sql, &typed(p)).await
    }
    pub async fn query_one(&self, sql: &str, p: Params<'_>) -> Result<Row, tokio_postgres::Error> {
        self.0.query_typed_one(sql, &typed(p)).await
    }
    pub async fn execute(&self, sql: &str, p: Params<'_>) -> Result<u64, tokio_postgres::Error> {
        self.0.execute_typed(sql, &typed(p)).await
    }
    /// Simple query protocol; no parameters (BEGIN / COMMIT / ROLLBACK).
    pub async fn batch_execute(&self, sql: &str) -> Result<(), tokio_postgres::Error> {
        self.0.batch_execute(sql).await
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
        created_by: UserId(r.get("created_by")),
        created_at: r.get("created_at"),
        archived_at: r.get("archived_at"),
        is_draft: r.get("is_draft"),
        deleted_at: r.get("deleted_at"),
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

const PROFILE_COLS: &str = "user_id, id, username, first_name, last_name, display_name, avatar, cover_image, timezone, \
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
        network: ProjectNetwork::parse(&network).ok_or_else(|| DbError::Backend(format!("p_network holds '{network}', which is no network")))?,
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
    st.is_default AS st_is_default, st.created_by AS st_created_by, st.created_at AS st_created_at, st.updated_at AS st_updated_at";

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
        group: StateGroup::parse(&group).ok_or_else(|| DbError::Backend(format!("st_state_group holds '{group}', which is no state group")))?,
        is_default: r.get("st_is_default"),
        created_by: r.get::<_, Option<Uuid>>("st_created_by").map(UserId),
        created_at: r.get("st_created_at"),
        updated_at: r.get("st_updated_at"),
    })
}

/// The columns of `project_user_properties` the port's `ProjectUserProperties` is made of (unaliased).
const PUP_COLS: &str = "id, project_id, workspace_id, user_id, filters, display_filters, display_properties, rich_filters, \
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

#[async_trait]
impl Database for PgDatabase {
    async fn membership(
        &self,
        slug: &WorkspaceSlug,
        project: ProjectId,
        user: UserId,
    ) -> Result<Membership, DbError> {
        let c = self.conn().await?;
        // One query: the slug, the caller's workspace membership (inner join), the project in that
        // workspace (inner join) and the caller's project row (left join, same workspace). No row
        // means the caller may not know the workspace or the project exists.
        let row = Pg(c.client())
            .query_opt(
                "SELECT w.id AS workspace_id, m.role AS role FROM workspaces w \
                 JOIN workspace_members wm ON wm.workspace_id = w.id AND wm.user_id = $3 AND wm.is_active \
                 JOIN projects p ON p.workspace_id = w.id AND p.id = $2 \
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
        let row = Pg(c.client())
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
        Pg(c.client())
            .execute("UPDATE sessions SET last_seen_at = $2 WHERE token_hash = $1", &[&&hash.0[..], &at])
            .await
            .map_err(db_err)?;
        Ok(())
    }

    async fn revoke_session(&self, hash: &TokenHash, at: Timestamp) -> Result<(), DbError> {
        let c = self.conn().await?;
        Pg(c.client())
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
        Pg(c.client())
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
        Pg(c.client())
            .execute(
                "UPDATE sessions SET revoked_at = $2 WHERE user_id = $1 AND revoked_at IS NULL",
                &[&user.0, &at],
            )
            .await
            .map_err(db_err)
    }

    async fn user_by_email(&self, email: &str) -> Result<Option<UserRecord>, DbError> {
        let c = self.conn().await?;
        let row = Pg(c.client())
            .query_opt(&format!("SELECT {USER_COLS} FROM users WHERE email = $1"), &[&email])
            .await
            .map_err(db_err)?;
        Ok(row.as_ref().map(user_record))
    }

    async fn insert_workspace(&self, w: &NewWorkspace) -> Result<(), DbError> {
        // The workspace and its first member go in together or not at all: `Tx` only exposes slice operations, so this goes
        // through a connection of its own, as `seed` does. A taken slug is a unique violation, which `db_err` makes a `Conflict`.
        let c = self.connector.checkout(true).await?;
        let cl = Pg(c.client());
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
        let row = Pg(c.client())
            .query_one("SELECT EXISTS (SELECT 1 FROM workspaces WHERE slug = $1) AS taken", &[&slug])
            .await
            .map_err(db_err)?;
        Ok(row.get("taken"))
    }

    async fn any_workspace(&self) -> Result<bool, DbError> {
        let c = self.conn().await?;
        let row = Pg(c.client()).query_one("SELECT EXISTS (SELECT 1 FROM workspaces) AS any", &[]).await.map_err(db_err)?;
        Ok(row.get("any"))
    }

    async fn workspaces_of(&self, user: UserId) -> Result<Vec<WorkspaceListing>, DbError> {
        let c = self.conn().await?;
        let rows = Pg(c.client())
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
        let row = Pg(c.client())
            .query_opt(
                "SELECT w.id AS workspace_id, wm.role AS role FROM workspaces w \
                 JOIN workspace_members wm ON wm.workspace_id = w.id AND wm.user_id = $2 AND wm.is_active \
                 WHERE w.slug = $1",
                &[&slug.as_str(), &user.0],
            )
            .await
            .map_err(db_err)?;
        row.map(|r| Ok(WorkspaceAccess { workspace_id: WorkspaceId(r.get("workspace_id")), role: role_of(&r, "role")? })).transpose()
    }

    async fn is_workspace_member(&self, scope: &WorkspaceScope, user: UserId) -> Result<bool, DbError> {
        let c = self.conn().await?;
        let row = Pg(c.client())
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
        let n = Pg(c.client())
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
        let row = Pg(c.client())
            .query_opt(
                &format!("SELECT {MEMBER_COLS} FROM workspace_members wm WHERE wm.workspace_id = $1 AND wm.user_id = $2"),
                &[&scope.workspace_id().0, &scope.user_id().0],
            )
            .await
            .map_err(db_err)?;
        row.as_ref().map(member_of).transpose()
    }

    async fn workspace_members(&self, scope: &WorkspaceScope) -> Result<Vec<MemberWithUser>, DbError> {
        let c = self.conn().await?;
        let rows = Pg(c.client())
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
        let rows = Pg(c.client())
            .query(
                // the project must really be in the workspace the member row names: a row that says otherwise is not a role
                "SELECT m.project_id AS project_id, m.role AS role FROM project_members m \
                 JOIN projects p ON p.id = m.project_id AND p.workspace_id = m.workspace_id \
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
        let rows = Pg(c.client())
            .query(
                &format!(
                    "SELECT {PROJECT_COLS}, m.role AS m_role, up.sort_order AS up_sort_order FROM projects p \
                     LEFT JOIN project_members m \
                       ON m.project_id = p.id AND m.workspace_id = p.workspace_id AND m.user_id = $2 \
                     LEFT JOIN project_user_properties up ON up.project_id = p.id AND up.user_id = $2 \
                     WHERE p.workspace_id = $1 AND ($3 OR m.role IS NOT NULL OR ($4 AND p.network = 'public')) \
                     ORDER BY p.created_at DESC, p.id DESC"
                ),
                &[&scope.workspace_id().0, &scope.user_id().0, &all, &public],
            )
            .await
            .map_err(db_err)?;
        rows.iter()
            .map(|r| {
                let role: Option<String> = r.get("m_role");
                Ok(ProjectListing {
                    project: project_of(r)?,
                    role: role
                        .map(|s| Role::parse(&s).ok_or_else(|| DbError::Backend(format!("m_role holds '{s}', which is no role"))))
                        .transpose()?,
                    sort_order: r.get("up_sort_order"),
                })
            })
            .collect()
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
        let cl = Pg(c.client());
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
            .map(|r| SidebarPreference { key: r.get("key"), is_pinned: r.get("is_pinned"), sort_order: r.get("sort_order") })
            .collect())
    }

    async fn ensure_home_preferences(&self, scope: &WorkspaceScope, defaults: &[HomePreference]) -> Result<Vec<HomePreference>, DbError> {
        let keys: Vec<&str> = defaults.iter().map(|d| d.key.as_str()).collect();
        let enabled: Vec<bool> = defaults.iter().map(|d| d.is_enabled).collect();
        let configs: Vec<serde_json::Value> = defaults.iter().map(|d| d.config.clone()).collect();
        let orders: Vec<f64> = defaults.iter().map(|d| d.sort_order).collect();
        let made: Vec<Timestamp> = defaults.iter().map(|d| d.created_at).collect();
        let c = self.conn().await?;
        let cl = Pg(c.client());
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

    async fn ensure_user_properties(&self, scope: &WorkspaceScope, new: &UserProperties) -> Result<UserProperties, DbError> {
        let c = self.conn().await?;
        let cl = Pg(c.client());
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
        let cl = Pg(c.client());
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
        let row = Pg(c.client())
            .query_one(
                "SELECT \
                   EXISTS (SELECT 1 FROM projects WHERE workspace_id = $1 AND name = $2 AND ($4::uuid IS NULL OR id <> $4)) AS name_taken, \
                   EXISTS (SELECT 1 FROM projects WHERE workspace_id = $1 AND identifier = $3 AND ($4::uuid IS NULL OR id <> $4)) AS identifier_taken",
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
        let set: Vec<String> = columns[4..].iter().enumerate().map(|(i, c)| format!("{} = ${}", c.0, i + 3)).collect();
        let workspace = scope.workspace_id().0;
        let mut params: Vec<&dyn PgParam> = vec![&row.id.0, &workspace];
        params.extend(columns[4..].iter().map(|c| c.1));
        let c = self.conn().await?;
        let n = Pg(c.client())
            .execute(&format!("UPDATE projects SET {} WHERE id = $1 AND workspace_id = $2", set.join(", ")), &params)
            .await
            .map_err(db_err)?;
        Ok(n == 1)
    }

    async fn project_detail(&self, scope: &WorkspaceScope, id: ProjectId) -> Result<Option<ProjectDetail>, DbError> {
        let c = self.conn().await?;
        let row = Pg(c.client())
            .query_opt(
                &format!(
                    "SELECT {PROJECT_COLS}, m.role AS m_role, up.sort_order AS up_sort_order, \
                       COALESCE((SELECT s.last_sequence_id FROM project_sequences s WHERE s.workspace_id = p.workspace_id AND s.project_id = p.id), 0) + 1 AS next_sequence, \
                       ARRAY(SELECT pm.user_id FROM project_members pm WHERE pm.project_id = p.id AND pm.workspace_id = p.workspace_id \
                             ORDER BY pm.created_at DESC, pm.id DESC) AS member_ids \
                     FROM projects p \
                     LEFT JOIN project_members m ON m.project_id = p.id AND m.workspace_id = p.workspace_id AND m.user_id = $2 \
                     LEFT JOIN project_user_properties up ON up.project_id = p.id AND up.user_id = $2 \
                     WHERE p.id = $3 AND p.workspace_id = $1"
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
                    role: role.map(|s| Role::parse(&s).ok_or_else(|| DbError::Backend(format!("m_role holds '{s}', which is no role")))).transpose()?,
                    sort_order: r.get("up_sort_order"),
                },
                members: r.get::<_, Vec<Uuid>>("member_ids").into_iter().map(UserId).collect(),
                next_sequence: r.get("next_sequence"),
            })
        })
        .transpose()
    }

    async fn project_member(&self, scope: &WorkspaceScope, project: ProjectId) -> Result<Option<ProjectMemberWithUser>, DbError> {
        let c = self.conn().await?;
        let row = Pg(c.client())
            .query_opt(
                &format!(
                    "SELECT {PMEMBER_COLS}, {PROJECT_COLS}, {WORKSPACE_COLS}, u.email AS u_email, \
                       COALESCE(pr.first_name, '') AS u_first_name, COALESCE(pr.last_name, '') AS u_last_name, \
                       COALESCE(pr.display_name, '') AS u_display_name, COALESCE(pr.avatar, '') AS u_avatar \
                     FROM project_members pm \
                       JOIN projects p ON p.id = pm.project_id AND p.workspace_id = pm.workspace_id \
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
        let rows = Pg(c.client())
            .query(
                &format!(
                    "SELECT {PMEMBER_COLS} FROM project_members pm \
                       JOIN projects p ON p.id = pm.project_id AND p.workspace_id = pm.workspace_id \
                       JOIN workspace_members wm ON wm.workspace_id = pm.workspace_id AND wm.user_id = pm.user_id AND wm.is_active \
                     WHERE pm.workspace_id = $1 AND pm.project_id = $2 ORDER BY pm.created_at DESC, pm.id DESC"
                ),
                &[&scope.workspace_id().0, &scope.project_id().0],
            )
            .await
            .map_err(db_err)?;
        rows.iter().map(pmember_of).collect()
    }

    async fn ensure_project_user_properties(&self, scope: &Scope, new: &ProjectUserProperties) -> Result<ProjectUserProperties, DbError> {
        let c = self.conn().await?;
        let cl = Pg(c.client());
        // from `projects`, so a project that is not in the scope's workspace makes no row (and the SELECT below finds none)
        cl.execute(
            "INSERT INTO project_user_properties (id, project_id, workspace_id, user_id, filters, display_filters, display_properties, \
               rich_filters, preferences, sort_order, created_at, updated_at) \
             SELECT $1, p.id, p.workspace_id, $4, $5, $6, $7, $8, $9, $10, $11, $12 FROM projects p WHERE p.id = $2 AND p.workspace_id = $3 \
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
        let rows = Pg(c.client())
            .query(
                "SELECT id, workspace_id, project_id, parent_id, name, description, color, sort_order, created_by, created_at, updated_at \
                 FROM labels WHERE workspace_id = $1 AND project_id = $2 ORDER BY sort_order, id",
                &[&scope.workspace_id().0, &scope.project_id().0],
            )
            .await
            .map_err(db_err)?;
        Ok(rows
            .iter()
            .map(|r| LabelRow {
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
            })
            .collect())
    }

    async fn project_states(&self, scope: &Scope) -> Result<Vec<StateRow>, DbError> {
        let c = self.conn().await?;
        let rows = Pg(c.client())
            .query(
                &format!(
                    "SELECT {STATE_COLS} FROM states st WHERE st.workspace_id = $1 AND st.project_id = $2 AND st.state_group <> 'triage' \
                     ORDER BY st.sequence, st.id"
                ),
                &[&scope.workspace_id().0, &scope.project_id().0],
            )
            .await
            .map_err(db_err)?;
        rows.iter().map(state_of).collect()
    }

    async fn workspace_states(&self, scope: &WorkspaceScope) -> Result<Vec<StateRow>, DbError> {
        let c = self.conn().await?;
        let rows = Pg(c.client())
            .query(
                &format!(
                    "SELECT {STATE_COLS} FROM states st \
                       JOIN projects p ON p.id = st.project_id AND p.workspace_id = st.workspace_id AND p.archived_at IS NULL \
                       JOIN project_members m ON m.project_id = p.id AND m.workspace_id = p.workspace_id AND m.user_id = $2 \
                     WHERE st.workspace_id = $1 AND st.state_group <> 'triage' ORDER BY st.sequence, st.project_id, st.id"
                ),
                &[&scope.workspace_id().0, &scope.user_id().0],
            )
            .await
            .map_err(db_err)?;
        rows.iter().map(state_of).collect()
    }

    async fn triage_state(&self, scope: &Scope) -> Result<Option<StateRow>, DbError> {
        let c = self.conn().await?;
        let row = Pg(c.client())
            .query_opt(
                &format!(
                    "SELECT {STATE_COLS} FROM states st WHERE st.workspace_id = $1 AND st.project_id = $2 AND st.state_group = 'triage' \
                     ORDER BY st.sequence, st.id LIMIT 1"
                ),
                &[&scope.workspace_id().0, &scope.project_id().0],
            )
            .await
            .map_err(db_err)?;
        row.as_ref().map(state_of).transpose()
    }

    async fn project_stats(&self, scope: &WorkspaceScope, only: Option<&[ProjectId]>) -> Result<Vec<ProjectStat>, DbError> {
        let only: Option<Vec<Uuid>> = only.map(|ids| ids.iter().map(|p| p.0).collect());
        let c = self.conn().await?;
        let rows = Pg(c.client())
            .query(
                "SELECT p.id AS id, \
                   CASE WHEN p.archived_at IS NULL THEN \
                     (SELECT count(*) FROM work_items w WHERE w.project_id = p.id AND w.workspace_id = p.workspace_id \
                        AND w.deleted_at IS NULL AND w.archived_at IS NULL AND NOT w.is_draft) \
                   ELSE 0 END AS total_issues, \
                   (SELECT count(*) FROM project_members m WHERE m.project_id = p.id AND m.workspace_id = p.workspace_id) AS total_members \
                 FROM projects p WHERE p.workspace_id = $1 AND ($2::uuid[] IS NULL OR p.id = ANY($2)) \
                 ORDER BY p.created_at DESC, p.id DESC",
                &[&scope.workspace_id().0, &only],
            )
            .await
            .map_err(db_err)?;
        Ok(rows
            .iter()
            .map(|r| ProjectStat { id: ProjectId(r.get("id")), total_issues: r.get("total_issues"), total_members: r.get("total_members") })
            .collect())
    }

    async fn user_by_id(&self, id: UserId) -> Result<Option<UserRecord>, DbError> {
        let c = self.conn().await?;
        let row = Pg(c.client())
            .query_opt(&format!("SELECT {USER_COLS} FROM users WHERE id = $1"), &[&id.0])
            .await
            .map_err(db_err)?;
        Ok(row.as_ref().map(user_record))
    }

    async fn insert_profile(&self, p: &UserProfile) -> Result<bool, DbError> {
        // ON CONFLICT on the user only: a taken `id` or `username` is a unique violation, which `db_err` makes a `Conflict`
        let c = self.conn().await?;
        let n = Pg(c.client())
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
        let row = Pg(c.client())
            .query_opt(&format!("SELECT {PROFILE_COLS} FROM user_profiles WHERE user_id = $1"), &[&user.0])
            .await
            .map_err(db_err)?;
        Ok(row.as_ref().map(user_profile))
    }

    async fn update_profile(&self, user: UserId, patch: &ProfilePatch, at: Timestamp) -> Result<Option<UserProfile>, DbError> {
        // ONE statement: every column the patch does not name keeps its value, and the preferences are merged by jsonb `||`
        // inside the row lock, so two requests that change different fields cannot overwrite each other.
        let set_cover = patch.cover_image.is_some();
        let cover: Option<&str> = patch.cover_image.as_ref().and_then(|c| c.as_deref());
        let set_workspace = patch.last_workspace_id.is_some();
        let workspace: Option<Uuid> = patch.last_workspace_id.and_then(|w| w.map(|w| w.0));
        let preferences = serde_json::Value::Object(patch.preferences.clone());
        let c = self.conn().await?;
        let row = Pg(c.client())
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
        Pg(c.client())
            .execute(
                "INSERT INTO users (id, email, password_hash, is_active) VALUES ($1, $2, $3, $4)",
                &[&user.id.0, &user.email.as_deref(), &user.password_hash.as_deref(), &user.is_active],
            )
            .await
            .map_err(db_err)?;
        Ok(())
    }

    async fn user_for_account(
        &self,
        provider: &str,
        provider_account_id: &str,
    ) -> Result<Option<UserRecord>, DbError> {
        let c = self.conn().await?;
        let row = Pg(c.client())
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
        Pg(c.client())
            .execute(
                "INSERT INTO accounts (provider, provider_account_id, user_id, email, created_at) \
                 VALUES ($1, $2, $3, $4, $5) ON CONFLICT (provider, provider_account_id) DO NOTHING",
                &[&a.provider.as_str(), &a.provider_account_id.as_str(), &a.user_id.0, &a.email.as_deref(), &a.created_at],
            )
            .await
            .map_err(db_err)?;
        Ok(())
    }

    async fn insert_api_key(&self, k: &NewApiKey) -> Result<(), DbError> {
        let c = self.conn().await?;
        Pg(c.client())
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
        let row = Pg(c.client())
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
        Pg(c.client())
            .execute("UPDATE api_keys SET last_used_at = $2 WHERE id = $1", &[&id.0, &at])
            .await
            .map_err(db_err)?;
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
        let n = Pg(c.client())
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
        let rows = Pg(c.client())
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
        let n = Pg(c.client())
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
        let pg = Pg(c.client());
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
        let row = Pg(c.client())
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

    async fn set_user_active(&self, _sys: &SystemScope, user: UserId, active: bool) -> Result<bool, DbError> {
        let c = self.conn().await?;
        let n = Pg(c.client())
            .execute("UPDATE users SET is_active = $2 WHERE id = $1", &[&user.0, &active])
            .await
            .map_err(db_err)?;
        Ok(n == 1)
    }

    async fn seed_users(&self, _sys: &SystemScope, users: &[UserRecord]) -> Result<(), DbError> {
        let c = self.connector.checkout(true).await?;
        let cl = Pg(c.client());
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

    async fn list_work_items(&self, scope: &Scope, vis: Visibility) -> Result<Vec<WorkItem>, DbError> {
        let (ws, project) = (scope.workspace_id(), scope.project_id());
        let sql = match vis {
            Visibility::Visible => format!(
                "SELECT {WI_COLS} FROM work_items WHERE workspace_id = $1 AND project_id = $2 \
                 AND deleted_at IS NULL AND archived_at IS NULL AND is_draft = false ORDER BY sequence_id"
            ),
            Visibility::WithDeleted => format!(
                "SELECT {WI_COLS} FROM work_items WHERE workspace_id = $1 AND project_id = $2 ORDER BY sequence_id"
            ),
        };
        let c = self.conn().await?;
        let rows = Pg(c.client()).query(&sql, &[&ws.0, &project.0]).await.map_err(db_err)?;
        Ok(rows.iter().map(work_item).collect())
    }

    async fn get_work_item(
        &self,
        scope: &Scope,
        id: WorkItemId,
        vis: Visibility,
    ) -> Result<Option<WorkItem>, DbError> {
        let (ws, project) = (scope.workspace_id(), scope.project_id());
        let hidden = match vis {
            Visibility::Visible => " AND deleted_at IS NULL AND archived_at IS NULL AND is_draft = false",
            Visibility::WithDeleted => "",
        };
        let sql = format!(
            "SELECT {WI_COLS} FROM work_items WHERE workspace_id = $1 AND project_id = $2 AND id = $3{hidden}"
        );
        let c = self.conn().await?;
        let row = Pg(c.client()).query_opt(&sql, &[&ws.0, &project.0, &id.0]).await.map_err(db_err)?;
        Ok(row.as_ref().map(work_item))
    }

    async fn search_work_items(&self, scope: &Scope, query: &str, limit: u32) -> Result<Vec<WorkItem>, DbError> {
        let (ws, project) = (scope.workspace_id(), scope.project_id());
        // `%`, `_` and `\` are literal in a query, so they are escaped for LIKE (the trigram GIN index serves `ILIKE`).
        let pattern = like_pattern(query);
        let c = self.conn().await?;
        let rows = Pg(c.client())
            .query(
                &format!(
                    "SELECT {WI_COLS} FROM work_items WHERE workspace_id = $1 AND project_id = $2 \
                     AND deleted_at IS NULL AND archived_at IS NULL AND is_draft = false \
                     AND (name ILIKE $3 ESCAPE '\\' OR description ILIKE $3 ESCAPE '\\') ORDER BY sequence_id LIMIT $4"
                ),
                &[&ws.0, &project.0, &pattern, &i64::from(limit)],
            )
            .await
            .map_err(db_err)?;
        Ok(rows.iter().map(work_item).collect())
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
        let cl = Pg(c.client());
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
                "INSERT INTO work_items (id, workspace_id, project_id, sequence_id, sort_order, name, description, created_by, created_at, is_draft) \
                 SELECT t.id, $1, $2, $3 + t.ord, $4 + t.ord * 10000, t.name, t.description, $5, $6, false \
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
        let n = Pg(c.client())
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
        let rows = Pg(c.client())
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

    async fn undispatched_outbox(&self, _sys: &SystemScope, limit: u32) -> Result<Vec<OutboxRow>, DbError> {
        let c = self.conn().await?;
        let rows = Pg(c.client())
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
        Pg(c.client())
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
        let r = Pg(c.client())
            .query_one("SELECT count(*) AS n FROM outbox WHERE dispatched_at IS NULL", &[])
            .await
            .map_err(db_err)?;
        Ok(r.get::<_, i64>("n") as u64)
    }

    async fn seed(&self, _sys: &SystemScope, f: &SeedFixture) -> Result<(), DbError> {
        // `Tx` only exposes slice operations, so the fixture goes through a connection directly.
        let c = self.connector.checkout(true).await?;
        let cl = Pg(c.client());
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
                       created_at, updated_at) \
                     VALUES ($1, $2, $3, $4, $5, CASE WHEN $6 THEN COALESCE($9, now()) END, $7, $8, COALESCE($9, now()), \
                       COALESCE($9, now())) \
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
}

pub struct PgTx {
    conn: Option<Box<dyn ClientHandle>>,
    /// `BEGIN` has been sent on `conn`: the first write opens the transaction.
    in_tx: bool,
    connector: Arc<dyn Connector>,
}

impl PgTx {
    fn client(&self) -> Pg<'_> {
        Pg(self.conn.as_ref().expect("transaction already finished").client())
    }

    /// Open the transaction on first use. If `BEGIN` fails the connection's state is unknown, so it is not reused.
    async fn ensure_tx(&mut self) -> Result<(), DbError> {
        if self.in_tx {
            return Ok(());
        }
        match self.client().batch_execute("BEGIN").await {
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
        const UPSERT: &str = "INSERT INTO project_sequences (workspace_id, project_id, last_sequence_id, last_sort_order) \
             VALUES ($1, $2, 1, 10000) \
             ON CONFLICT (workspace_id, project_id) DO UPDATE SET \
               last_sequence_id = project_sequences.last_sequence_id + 1, \
               last_sort_order  = project_sequences.last_sort_order + 10000 \
             RETURNING last_sequence_id, last_sort_order";
        let (ws, project) = (scope.workspace_id(), scope.project_id());
        let r = if self.in_tx {
            let other = self.connector.checkout(true).await?;
            let r = Pg(other.client()).query_one(UPSERT, &[&ws.0, &project.0]).await;
            drop(other); // autocommit: nothing is open on it, it is clean
            r
        } else {
            self.client().query_one(UPSERT, &[&ws.0, &project.0]).await
        }
        .map_err(db_err)?;
        Ok(Allocation { sequence_id: r.get(0), sort_order: r.get(1) })
    }

    async fn insert_work_item(&mut self, scope: &Scope, w: &WorkItem) -> Result<(), DbError> {
        check_tenant(scope, w.workspace_id, Some(w.project_id))?;
        self.ensure_tx().await?;
        self.client()
            .execute(
                &format!(
                    "INSERT INTO work_items ({WI_COLS}) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14)"
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
                    &w.created_by.0,
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

    async fn soft_delete_work_item(&mut self, scope: &Scope, id: WorkItemId, at: Timestamp) -> Result<bool, DbError> {
        let (ws, project) = (scope.workspace_id(), scope.project_id());
        self.ensure_tx().await?;
        let n = self
            .client()
            .execute(
                "UPDATE work_items SET deleted_at = $4 \
                 WHERE workspace_id = $1 AND project_id = $2 AND id = $3 AND deleted_at IS NULL",
                &[&ws.0, &project.0, &id.0, &at],
            )
            .await
            .map_err(db_err)?;
        Ok(n == 1)
    }

    async fn insert_outbox(&mut self, scope: &Scope, row: &OutboxRow) -> Result<(), DbError> {
        check_tenant(scope, row.envelope.workspace_id, row.envelope.project_id())?;
        let envelope = serde_json::to_value(&row.envelope).map_err(|e| DbError::Backend(e.to_string()))?;
        let params: Params = &[
            &row.envelope.event_id.0,
            &row.envelope.workspace_id.0,
            &envelope,
            &row.created_at,
        ];
        self.ensure_tx().await?;
        self.client()
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
        let conn = this.conn.take().expect("transaction already finished");
        if !in_tx {
            // Nothing was written, so no transaction was opened: the connection is clean and goes back as it is.
            return Ok(());
        }
        match conn.client().batch_execute("COMMIT").await {
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
        let conn = this.conn.take().expect("transaction already finished");
        if !in_tx {
            return Ok(());
        }
        match conn.client().batch_execute("ROLLBACK").await {
            Ok(()) => Ok(()),
            Err(e) => {
                conn.discard();
                Err(db_err(e))
            }
        }
    }
}
