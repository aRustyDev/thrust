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

/// Authenticated caller. Produced by the `SessionStore` port, consumed by the use-cases.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Principal {
    pub user_id: UserId,
    pub workspace_id: WorkspaceId,
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
    pub workspaces: Vec<WorkspaceId>,
    pub users: Vec<UserId>,
    pub projects: Vec<SeedProject>,
    pub memberships: Vec<SeedMembership>,
    pub sessions: Vec<SeedSession>,
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
    pub principal: Principal,
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
