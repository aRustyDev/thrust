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
    pub projects: Vec<SeedProject>,
    pub memberships: Vec<SeedMembership>,
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
