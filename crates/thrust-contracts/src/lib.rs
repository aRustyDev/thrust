//! Data contracts: identifiers, domain records, the event envelope (ADR-0046) and the API DTOs.
//!
//! Dependency rule (checked by `scripts/check-deps.sh`): this crate depends on serde, uuid and
//! chrono only. No tokio, no wasm-bindgen, no driver.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
pub use chrono;
pub use uuid::Uuid;

pub type Timestamp = DateTime<Utc>;

macro_rules! id_type {
    ($(#[$m:meta])* $name:ident) => {
        $(#[$m])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub Uuid);

        impl $name {
            pub fn new(u: Uuid) -> Self {
                Self(u)
            }
            pub fn as_uuid(&self) -> Uuid {
                self.0
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                self.0.fmt(f)
            }
        }

        impl std::str::FromStr for $name {
            type Err = uuid::Error;
            fn from_str(s: &str) -> Result<Self, Self::Err> {
                Ok(Self(Uuid::parse_str(s)?))
            }
        }
    };
}

id_type!(
    /// Tenant key (ADR-0041). Every repository call carries one.
    WorkspaceId
);
id_type!(ProjectId);
id_type!(UserId);
id_type!(WorkItemId);
id_type!(EventId);
id_type!(ActivityId);
id_type!(ApiKeyId);

/// Project-level role. Plane: Guest 5 / Member 15 / Admin 20.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Guest,
    Member,
    Admin,
}

impl Role {
    pub fn as_str(&self) -> &'static str {
        match self {
            Role::Guest => "guest",
            Role::Member => "member",
            Role::Admin => "admin",
        }
    }
    pub fn parse(s: &str) -> Option<Role> {
        match s {
            "guest" => Some(Role::Guest),
            "member" => Some(Role::Member),
            "admin" => Some(Role::Admin),
            _ => None,
        }
    }
}

/// Who may see a project without being a member of it (Plane's `network`: 0 secret, 2 public). A secret project is seen
/// by its members and by the workspace's admins; a public one also by every workspace member who is not a guest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectNetwork {
    Secret,
    #[default]
    Public,
}

impl ProjectNetwork {
    pub fn as_str(&self) -> &'static str {
        match self {
            ProjectNetwork::Secret => "secret",
            ProjectNetwork::Public => "public",
        }
    }
    pub fn parse(s: &str) -> Option<ProjectNetwork> {
        match s {
            "secret" => Some(ProjectNetwork::Secret),
            "public" => Some(ProjectNetwork::Public),
            _ => None,
        }
    }
}

/// What kind of state a work item is in (Plane's `StateGroup`). A project's states each belong to one; `Triage` is the one that
/// intake uses, and the lists of a project's states leave it out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StateGroup {
    Backlog,
    Unstarted,
    Started,
    Completed,
    Cancelled,
    Triage,
}

impl StateGroup {
    pub const ALL: [StateGroup; 6] = [
        StateGroup::Backlog,
        StateGroup::Unstarted,
        StateGroup::Started,
        StateGroup::Completed,
        StateGroup::Cancelled,
        StateGroup::Triage,
    ];

    pub fn as_str(&self) -> &'static str {
        match self {
            StateGroup::Backlog => "backlog",
            StateGroup::Unstarted => "unstarted",
            StateGroup::Started => "started",
            StateGroup::Completed => "completed",
            StateGroup::Cancelled => "cancelled",
            StateGroup::Triage => "triage",
        }
    }

    pub fn parse(s: &str) -> Option<StateGroup> {
        StateGroup::ALL.into_iter().find(|g| g.as_str() == s)
    }
}

/// The workspace segment of a route (`/workspaces/{slug}/...`). Unvalidated on purpose: a slug that
/// names no workspace is a 404 from `authorize`, never a parse error.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct WorkspaceSlug(pub String);

impl WorkspaceSlug {
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for WorkspaceSlug {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Authenticated caller: the user and nothing else. A session belongs to a user, not to a workspace
/// (ADR-0052 round 29); `authorize` resolves the workspace per request from the route's slug.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Principal {
    pub user_id: UserId,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkItem {
    pub id: WorkItemId,
    pub workspace_id: WorkspaceId,
    pub project_id: ProjectId,
    /// Per-project running number ("PLATF-7" -> 7). Never reused (ADR-0043 B1).
    pub sequence_id: i64,
    pub sort_order: i64,
    pub name: String,
    pub description: Option<String>,
    pub external_source: Option<String>,
    pub external_id: Option<String>,
    pub created_by: UserId,
    pub created_at: Timestamp,
    pub archived_at: Option<Timestamp>,
    pub is_draft: bool,
    pub deleted_at: Option<Timestamp>,
}

/// A work item's priority: Plane's five, written as it writes them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Priority {
    Urgent,
    High,
    Medium,
    Low,
    None,
}

impl Priority {
    /// In the order a list by priority starts with (`urgent` first).
    pub const ALL: [Priority; 5] = [Priority::Urgent, Priority::High, Priority::Medium, Priority::Low, Priority::None];

    pub fn as_str(&self) -> &'static str {
        match self {
            Priority::Urgent => "urgent",
            Priority::High => "high",
            Priority::Medium => "medium",
            Priority::Low => "low",
            Priority::None => "none",
        }
    }

    pub fn parse(s: &str) -> Option<Priority> {
        Priority::ALL.into_iter().find(|p| p.as_str() == s)
    }

    /// 0 for `urgent` to 4 for `none`: the number Plane orders a list by priority with.
    pub fn rank(&self) -> u8 {
        match self {
            Priority::Urgent => 0,
            Priority::High => 1,
            Priority::Medium => 2,
            Priority::Low => 3,
            Priority::None => 4,
        }
    }
}

/// What the compat plane keeps of a work item beyond what the slice does (issue #44, stage 6): its state, priority, dates, who
/// last changed it, its description as HTML, who it is assigned to and its labels. It lives on the same row as the slice's
/// [`WorkItem`] (migration 0013), which does not carry it, so the slice's JSON is what it was.
#[derive(Debug, Clone, PartialEq)]
pub struct WorkItemAttrs {
    /// `None` for an item the slice made: it has no state until a compat request gives it one.
    pub state_id: Option<Uuid>,
    pub priority: Priority,
    pub start_date: Option<chrono::NaiveDate>,
    pub target_date: Option<chrono::NaiveDate>,
    /// When the item entered a state of the `completed` group.
    pub completed_at: Option<Timestamp>,
    pub updated_at: Timestamp,
    pub updated_by: Option<UserId>,
    pub description_html: String,
    /// Ascending by id.
    pub assignee_ids: Vec<UserId>,
    /// Ascending by id.
    pub label_ids: Vec<Uuid>,
}

impl WorkItemAttrs {
    /// What a work item made by the slice has: the pin's defaults, from its creation.
    pub fn bare(created_at: Timestamp) -> WorkItemAttrs {
        WorkItemAttrs {
            state_id: None,
            priority: Priority::None,
            start_date: None,
            target_date: None,
            completed_at: None,
            updated_at: created_at,
            updated_by: None,
            description_html: "<p></p>".to_string(),
            assignee_ids: Vec::new(),
            label_ids: Vec::new(),
        }
    }
}

/// A work item with its compat attributes, and the group of its state (the list shows it as `state__group`).
#[derive(Debug, Clone, PartialEq)]
pub struct WorkItemRow {
    pub item: WorkItem,
    pub attrs: WorkItemAttrs,
    pub state_group: Option<StateGroup>,
}

/// Which hidden-row classes a read includes (ADR-0043 C1 named scopes).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Visibility {
    /// Default: excludes deleted, archived and draft rows.
    Visible,
    /// Includes soft-deleted rows (and archived/draft). Admin-only at the use-case layer.
    WithDeleted,
}

/// Result of `allocate_sequence`: the next per-project number plus `sort_order`
/// (largest existing + 10_000, as Plane's `Issue.save()` does).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Allocation {
    pub sequence_id: i64,
    pub sort_order: i64,
}

/// Event envelope, schema v1 (ADR-0046). Must stay under 128 KB on the wire (ADR-0070).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Envelope {
    pub event_id: EventId,
    pub schema_version: u32,
    pub workspace_id: WorkspaceId,
    /// `<entity>.<verb>`, e.g. `work_item.created`.
    pub event_type: String,
    pub entity_type: String,
    pub entity_id: Uuid,
    pub actor_id: Option<UserId>,
    pub occurred_at: Timestamp,
    pub payload: serde_json::Value,
    pub trace_id: Option<String>,
}

impl Envelope {
    pub fn project_id(&self) -> Option<ProjectId> {
        self.payload
            .get("project_id")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse().ok())
    }
}

/// Transactional-outbox row (ADR-0046): written in the same transaction as the entity.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OutboxRow {
    pub envelope: Envelope,
    pub created_at: Timestamp,
    pub dispatched_at: Option<Timestamp>,
    pub attempts: i32,
}

/// Activity projection row written by the queue consumer (idempotent on `event_id`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Activity {
    pub id: ActivityId,
    pub event_id: EventId,
    pub workspace_id: WorkspaceId,
    pub project_id: ProjectId,
    pub entity_id: Uuid,
    pub verb: String,
    pub actor_id: Option<UserId>,
    pub created_at: Timestamp,
}

// ---------------------------------------------------------------------------------------------
// Test fixtures (served by the `/internal/seed` route that exists only when test routes are on).
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SeedFixture {
    pub workspaces: Vec<SeedWorkspace>,
    /// Users that exist, by id only: they have no email and no password, so they cannot sign in, but a session or an
    /// API key can belong to them. A user who signs in is in `logins` as well.
    pub users: Vec<UserId>,
    /// Workspace membership. A project member must also be a member of the project's workspace.
    #[serde(default)]
    pub workspace_members: Vec<SeedWorkspaceMember>,
    /// The roles of some workspace members (issue #44): a seeded member is a `member` unless one of these says otherwise.
    #[serde(default)]
    pub workspace_roles: Vec<SeedWorkspaceRole>,
    pub projects: Vec<SeedProject>,
    /// What some of the `projects` have beyond their name (issue #44): identifier, network, archived, lead, creator, time.
    #[serde(default)]
    pub project_details: Vec<SeedProjectDetails>,
    pub memberships: Vec<SeedMembership>,
    /// Where members' project lists put the `projects`: the `project_user_properties` rows.
    #[serde(default)]
    pub project_sort_orders: Vec<SeedProjectSortOrder>,
    /// States and labels of the `projects` (issue #44, the project stage).
    #[serde(default)]
    pub states: Vec<SeedState>,
    #[serde(default)]
    pub labels: Vec<SeedLabel>,
    pub sessions: Vec<SeedSession>,
    /// Users with an email and optionally a password. The password is hashed by the core before the database sees it.
    #[serde(default)]
    pub logins: Vec<SeedLogin>,
    /// Links from an identity provider's subject to a user (`accounts` rows).
    #[serde(default)]
    pub accounts: Vec<SeedAccount>,
    /// API keys with a known token (the database only ever sees its hash), optionally already revoked or expired.
    #[serde(default)]
    pub api_keys: Vec<SeedApiKey>,
}

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeedLogin {
    pub user_id: UserId,
    pub email: String,
    /// `None`: the user has no password and can only be reached through an identity provider.
    #[serde(default)]
    pub password: Option<String>,
    #[serde(default = "yes")]
    pub is_active: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeedAccount {
    pub user_id: UserId,
    pub provider: String,
    pub provider_account_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeedApiKey {
    pub token: String,
    pub user_id: UserId,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub expires_at: Option<Timestamp>,
    #[serde(default)]
    pub revoked: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeedWorkspace {
    pub id: WorkspaceId,
    pub slug: WorkspaceSlug,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeedWorkspaceMember {
    pub workspace_id: WorkspaceId,
    pub user_id: UserId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeedProject {
    pub id: ProjectId,
    pub workspace_id: WorkspaceId,
    pub name: String,
}

/// The role a test gives a member of a workspace: a `workspace_members` entry makes a member with the role `member`, and this sets
/// the role of that member, who is made when the seed does not have them.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeedWorkspaceRole {
    pub workspace_id: WorkspaceId,
    pub user_id: UserId,
    pub role: Role,
}

/// What a seeded project has beyond the id, workspace and name of its [`SeedProject`] (the fixture's `project_details`). A
/// project with no entry has what a project made with no options has: the identifier [`default_project_identifier`] makes
/// from its id, a public network, not archived, no lead and no creator, and the database's own time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeedProjectDetails {
    pub project_id: ProjectId,
    #[serde(default)]
    pub identifier: Option<String>,
    #[serde(default)]
    pub network: ProjectNetwork,
    #[serde(default)]
    pub archived: bool,
    #[serde(default)]
    pub project_lead: Option<UserId>,
    #[serde(default)]
    pub created_by: Option<UserId>,
    /// When it was made; the epoch in memory and the database's `now()` in Postgres when none is given.
    #[serde(default)]
    pub created_at: Option<Timestamp>,
}

impl SeedProjectDetails {
    /// The details of a project made with no options.
    pub fn of(project_id: ProjectId) -> Self {
        Self {
            project_id,
            identifier: None,
            network: ProjectNetwork::default(),
            archived: false,
            project_lead: None,
            created_by: None,
            created_at: None,
        }
    }
}

/// Where a member's project list puts a project: their `project_user_properties` row. A member with no entry has no such
/// row, as one who was added to the project by something other than its creation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeedProjectSortOrder {
    pub project_id: ProjectId,
    pub user_id: UserId,
    pub sort_order: f64,
}

/// A state a test seeds in a project (`states`): everything but who made it and when, which a seeded state does not have.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeedState {
    pub id: Uuid,
    pub project_id: ProjectId,
    pub workspace_id: WorkspaceId,
    pub name: String,
    pub color: String,
    pub sequence: f64,
    pub group: StateGroup,
    #[serde(default)]
    pub is_default: bool,
}

/// A label a test seeds in a project (`labels`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeedLabel {
    pub id: Uuid,
    pub project_id: ProjectId,
    pub workspace_id: WorkspaceId,
    pub name: String,
    #[serde(default)]
    pub color: String,
    #[serde(default)]
    pub parent_id: Option<Uuid>,
    pub sort_order: f64,
}

/// The last twelve hex digits of the id, upper-cased: unique where the ids differ in their last group, and the longest key
/// Plane allows (12). Migration 0010 backfills a project that exists already the same way.
pub fn default_project_identifier(id: ProjectId) -> String {
    id.0.simple().to_string()[20..].to_uppercase()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeedMembership {
    pub project_id: ProjectId,
    pub workspace_id: WorkspaceId,
    pub user_id: UserId,
    pub role: Role,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeedSession {
    pub token: String,
    pub user_id: UserId,
}

/// One row of a bulk seed (`/internal/seed-work-items`): the text of a live work item. Everything else is assigned: the
/// id by the `IdGen`, the number and `sort_order` from the project's counter, the time from the `Clock`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeedWorkItemSpec {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
}

/// Body of `/internal/seed-work-items`: `items` become live work items of one project, in order.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeedWorkItemsRequest {
    pub workspace_id: WorkspaceId,
    pub project_id: ProjectId,
    pub created_by: UserId,
    pub items: Vec<SeedWorkItemSpec>,
}

/// Answer of `/internal/seed-work-items`: the number given to the first item (the rest follow, one by one).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeedWorkItemsReport {
    pub first_sequence_id: i64,
    pub count: u64,
}

/// A work item the bulk seed inserts, as the port takes it: complete except for the numbers.
#[derive(Debug, Clone, PartialEq)]
pub struct SeedWorkItem {
    pub id: WorkItemId,
    pub name: String,
    pub description: Option<String>,
    pub created_by: UserId,
    pub created_at: Timestamp,
}

// ---------------------------------------------------------------------------------------------
// API DTOs
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateWorkItemRequest {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub external_source: Option<String>,
    #[serde(default)]
    pub external_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiErrorBody {
    pub error: ApiErrorDetail,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiErrorDetail {
    pub code: String,
    pub message: String,
}

// ---------------------------------------------------------------------------------------------
// Identity DTOs (issue #38). Secrets only ever travel in the one response that creates them.
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignInRequest {
    pub email: String,
    pub password: String,
}

/// The answer to a sign-in or an OIDC login. The session token is in the `Set-Cookie` header only, never in the body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignInResponse {
    pub user_id: UserId,
    /// The double-submit token; the same value is in the CSRF cookie. A client sends it back in `X-CSRFToken`.
    pub csrf_token: String,
    pub expires_at: Timestamp,
}

/// `GET /auth/csrf`, as Plane's `get-csrf-token` (its CSRF cookie is HttpOnly, so the token is handed out as JSON).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CsrfResponse {
    pub csrf_token: String,
}

/// Which credential a request was authenticated with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CredentialKind {
    Session,
    ApiKey,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionInfo {
    pub user_id: UserId,
    pub credential: CredentialKind,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CreateApiKeyRequest {
    #[serde(default)]
    pub label: Option<String>,
    /// Must be in the future. `None`: the key does not expire.
    #[serde(default)]
    pub expires_at: Option<Timestamp>,
}

/// The one response that carries the token. Only its SHA-256 is stored, so it cannot be shown again.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreatedApiKey {
    pub id: ApiKeyId,
    pub token: String,
    pub prefix: String,
    pub label: String,
    pub created_at: Timestamp,
    pub expires_at: Option<Timestamp>,
}

/// A key as listed: everything but the token.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiKeyInfo {
    pub id: ApiKeyId,
    pub prefix: String,
    pub label: String,
    pub created_at: Timestamp,
    pub expires_at: Option<Timestamp>,
    pub revoked_at: Option<Timestamp>,
    pub last_used_at: Option<Timestamp>,
}
