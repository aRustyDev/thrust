//! Workspaces (issue #44): made, listed, asked about, and the reads that belong to a workspace itself.
//!
//! A person makes a workspace and becomes its admin in one step. The rules a workspace must satisfy whoever asks (a name
//! and a slug of a bounded length, a slug of the right characters that is not reserved and not taken, a name that is not
//! a URL and has a letter or number) are here, and each refusal is an `Invalid` or a `Conflict`; their wording is the
//! caller's. The reads that name a workspace take a [`WorkspaceScope`], which only [`crate::authorize_workspace`] makes.

use thrust_contracts::*;
use thrust_ports::*;

use crate::reserved_slugs::RESERVED_WORKSPACE_SLUGS;
use crate::{App, CoreError, Me};

/// Longest workspace name and slug, and organization size, in characters (the pin's columns).
pub const MAX_WORKSPACE_NAME: usize = 80;
pub const MAX_WORKSPACE_SLUG: usize = 48;
pub const MAX_ORGANIZATION_SIZE: usize = 20;

/// Whether `slug` is one nobody may take: Plane's `RESTRICTED_WORKSPACE_SLUGS`, exact and case-sensitive.
pub fn is_reserved_slug(slug: &str) -> bool {
    RESERVED_WORKSPACE_SLUGS.binary_search(&slug).is_ok()
}

/// Whether `slug` is made of letters, digits, hyphens and underscores only, and is not empty (Django's `SlugField`, ASCII).
pub fn slug_has_valid_characters(slug: &str) -> bool {
    !slug.is_empty() && slug.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Whether `name` holds a letter or a number of any script (Plane's `has_alphanumeric`, which is Python's `str.isalnum`):
/// a name of only symbols is no name.
pub fn name_has_alphanumeric(name: &str) -> bool {
    name.chars().any(char::is_alphanumeric)
}

/// What a person asks for when they make a workspace. The name and slug are as they will be stored: the caller has trimmed
/// them, as Django REST Framework does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewWorkspaceRequest {
    pub name: String,
    pub slug: String,
    pub organization_size: Option<String>,
    /// What the creator does, kept on their membership.
    pub company_role: Option<String>,
    /// An IANA name; `UTC` when none is given.
    pub timezone: Option<String>,
}

/// Make a workspace and its first member, the caller, who is its admin. `Invalid` for what breaks a rule above, `Conflict`
/// for a slug that is taken (also when another request took it between the check and the insert: the unique index decides),
/// and nothing is written then. The workspace's colour is made here from the random source.
pub async fn create_workspace(app: &App, who: &Principal, req: NewWorkspaceRequest) -> Result<WorkspaceListing, CoreError> {
    let bad = |what: &str| Err(CoreError::Invalid(what.to_string()));
    if req.name.is_empty() || req.name.chars().count() > MAX_WORKSPACE_NAME {
        return bad("a workspace name is 1 to 80 characters");
    }
    if req.slug.chars().count() > MAX_WORKSPACE_SLUG || !slug_has_valid_characters(&req.slug) {
        return bad("a workspace slug is 1 to 48 letters, digits, hyphens and underscores");
    }
    if is_reserved_slug(&req.slug) {
        return bad("that slug is reserved");
    }
    if crate::contains_url(&req.name) {
        return bad("a workspace name cannot contain a URL");
    }
    if !name_has_alphanumeric(&req.name) {
        return bad("a workspace name needs a letter or a number");
    }
    if req.organization_size.as_deref().is_some_and(|s| s.chars().count() > MAX_ORGANIZATION_SIZE) {
        return bad("an organization size is at most 20 characters");
    }
    let timezone = req.timezone.unwrap_or_else(|| "UTC".to_string());
    if !crate::is_known_timezone(&timezone) {
        return bad("not a known time zone");
    }
    if app.ports.db.user_by_id(who.user_id).await?.is_none() {
        return Err(CoreError::Unauthenticated);
    }

    let now = app.ports.clock.now();
    let id = WorkspaceId(app.ports.ids.new_uuid());
    let workspace = Workspace {
        id,
        slug: WorkspaceSlug::new(req.slug),
        name: req.name,
        owner_id: Some(who.user_id),
        organization_size: req.organization_size,
        timezone,
        background_color: format!("#{}", crate::auth::random_hex(app, 3)?),
        created_at: now,
        updated_at: now,
    };
    let owner = WorkspaceMember {
        id: app.ports.ids.new_uuid(),
        workspace_id: id,
        user_id: who.user_id,
        role: Role::Admin,
        company_role: req.company_role,
        is_active: true,
        props: serde_json::json!({}),
        created_at: now,
        updated_at: now,
    };
    app.ports.db.insert_workspace(&NewWorkspace { workspace: workspace.clone(), owner }).await?;
    Ok(WorkspaceListing { workspace, role: Role::Admin, total_members: 1 })
}

/// Whether no workspace has this slug and it is not reserved: what the slug check answers.
pub async fn slug_available(app: &App, slug: &str) -> Result<bool, CoreError> {
    Ok(!is_reserved_slug(slug) && !app.ports.db.workspace_slug_taken(slug).await?)
}

/// Whether any workspace exists (what the instance reports as `workspaces_exist`).
pub async fn any_workspace_exists(app: &App) -> Result<bool, CoreError> {
    Ok(app.ports.db.any_workspace().await?)
}

/// The workspaces the caller is an active member of, newest first.
pub async fn my_workspaces(app: &App, who: &Principal) -> Result<Vec<WorkspaceListing>, CoreError> {
    Ok(app.ports.db.workspaces_of(who.user_id).await?)
}

/// Which workspace the app should open for the caller: the last one they used if they are still an active member of it, and
/// otherwise the one they joined first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceChoice {
    /// The last workspace the profile names, when the caller is still an active member of it.
    pub last: Option<Workspace>,
    /// What to fall back to: the last workspace itself when there is one, else the earliest-made of the caller's workspaces.
    pub fallback: Option<Workspace>,
}

pub async fn workspace_choice(app: &App, me: &Me) -> Result<WorkspaceChoice, CoreError> {
    let mine = app.ports.db.workspaces_of(me.user.id).await?;
    let last = me
        .profile
        .last_workspace_id
        .and_then(|id| mine.iter().find(|l| l.workspace.id == id))
        .map(|l| l.workspace.clone());
    let fallback = last.clone().or_else(|| {
        mine.iter().map(|l| &l.workspace).min_by_key(|w| (w.created_at, w.id.0)).cloned()
    });
    Ok(WorkspaceChoice { last, fallback })
}

/// The caller's own membership of the workspace.
pub async fn my_membership(app: &App, scope: &WorkspaceScope) -> Result<WorkspaceMember, CoreError> {
    app.ports
        .db
        .workspace_member(scope)
        .await?
        .ok_or_else(|| CoreError::Internal("a member with a scope has no membership row".into()))
}

/// Everyone in the workspace, newest first, deactivated members included.
pub async fn members(app: &App, scope: &WorkspaceScope) -> Result<Vec<MemberWithUser>, CoreError> {
    Ok(app.ports.db.workspace_members(scope).await?)
}

/// The caller's role in each project of the workspace.
pub async fn my_project_roles(app: &App, scope: &WorkspaceScope) -> Result<Vec<(ProjectId, Role)>, CoreError> {
    Ok(app.ports.db.project_roles(scope).await?)
}
