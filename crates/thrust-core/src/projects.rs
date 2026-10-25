//! Projects (issue #44, the project stage): made, changed, opened, and what the app reads about one (who is in it, its states, its
//! labels, the caller's own settings for it, the workspace's counts).
//!
//! Who may do what is the core's, as the pin has it: a member or admin of the workspace makes a project and becomes its admin
//! (and so does the lead they name); the workspace's admins and the project's own admins change it; a project that is archived
//! is not changed; and a project is opened by its members, with the pin's three different refusals for anyone else (missing or
//! archived, secret, public). The wording and the order of a refusal are the compat layer's. The reads that name one project take a
//! [`Scope`], which only [`crate::authorize`] makes.

use serde_json::{json, Value};
use thiserror::Error;
use thrust_contracts::chrono::TimeDelta;
use thrust_contracts::*;
use thrust_ports::*;

use crate::{App, CoreError};

/// Longest project name and key, in characters (the pin's columns).
pub const MAX_PROJECT_NAME: usize = 255;
pub const MAX_PROJECT_IDENTIFIER: usize = 12;

/// The characters a project's name and key may not hold: Plane's `Project.FORBIDDEN_IDENTIFIER_CHARS_PATTERN`.
const FORBIDDEN: &str = "&+,:;$^}{*=?@#|'<>.()%!-";

/// Whether the pin's pattern `^.*[&+,:;$^}{*=?@#|'<>.()%!-].*$` matches `s`, which is Python's `re.match`: a forbidden character
/// on the first line, and nothing but that line (a single trailing newline aside, which `$` skips). A forbidden character after
/// a newline in the middle of the text is not found, as on the pin.
pub fn has_forbidden_project_chars(s: &str) -> bool {
    let line = s.strip_suffix('\n').unwrap_or(s);
    !line.contains('\n') && line.chars().any(|c| FORBIDDEN.contains(c))
}

/// Why a project's own routes refuse a caller, in the cases the pin tells apart.
#[derive(Debug, Error)]
pub enum ProjectError {
    /// No such project in the workspace.
    #[error("no such project")]
    NotFound,
    #[error("forbidden")]
    Forbidden,
    /// Opening a secret project the caller is not in.
    #[error("a secret project")]
    Secret,
    /// Opening a public project the caller is not in.
    #[error("not a member of the project")]
    NotAMember,
    /// Changing a project that is archived.
    #[error("the project is archived")]
    Archived,
    #[error(transparent)]
    Core(#[from] CoreError),
}

impl From<DbError> for ProjectError {
    fn from(e: DbError) -> Self {
        ProjectError::Core(e.into())
    }
}

/// The columns of a project a request may set, each only when it names it. A field that holds an `Option` says whether the
/// request names it (the outer one) and what it sets it to (the inner, `None` for null).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ProjectPatch {
    pub name: Option<String>,
    pub identifier: Option<String>,
    pub description: Option<String>,
    pub description_text: Option<Option<Value>>,
    pub description_html: Option<Option<Value>>,
    pub emoji: Option<Option<String>>,
    pub icon_prop: Option<Option<Value>>,
    pub network: Option<ProjectNetwork>,
    pub logo_props: Option<Value>,
    pub project_lead: Option<Option<UserId>>,
    pub default_assignee: Option<Option<UserId>>,
    pub cycle_view: Option<bool>,
    pub issue_views_view: Option<bool>,
    pub module_view: Option<bool>,
    pub page_view: Option<bool>,
    pub intake_view: Option<bool>,
    pub guest_view_all_features: Option<bool>,
    pub is_time_tracking_enabled: Option<bool>,
    pub is_issue_type_enabled: Option<bool>,
    pub cover_image: Option<Option<String>>,
    pub archive_in: Option<i32>,
    pub close_in: Option<i32>,
    pub timezone: Option<String>,
    pub external_source: Option<Option<String>>,
    pub external_id: Option<Option<String>>,
}

impl ProjectPatch {
    /// Set what the patch names on `row`.
    pub fn apply(&self, row: &mut ProjectRow) {
        macro_rules! set {
            ($($field:ident),+) => {
                $(if let Some(v) = &self.$field { row.$field = v.clone(); })+
            };
        }
        set!(
            name, identifier, description, description_text, description_html, emoji, icon_prop, network, logo_props, project_lead,
            default_assignee, cycle_view, issue_views_view, module_view, page_view, intake_view, guest_view_all_features,
            is_time_tracking_enabled, is_issue_type_enabled, cover_image, archive_in, close_in, timezone, external_source, external_id
        );
    }
}

/// What a project row must satisfy whoever asks: a name and a key of the right length without the forbidden characters, a time
/// zone that exists, and months from 0 to 12. Each refusal is an `Invalid`; their wording is the caller's.
fn check_project(row: &ProjectRow) -> Result<(), CoreError> {
    let bad = |what: &str| Err(CoreError::Invalid(what.to_string()));
    if row.name.is_empty() || row.name.chars().count() > MAX_PROJECT_NAME || has_forbidden_project_chars(&row.name) {
        return bad("a project name is 1 to 255 characters without &+,:;$^}{*=?@#|'<>.()%!-");
    }
    if row.identifier.is_empty() || row.identifier.chars().count() > MAX_PROJECT_IDENTIFIER || has_forbidden_project_chars(&row.identifier) {
        return bad("a project key is 1 to 12 characters without &+,:;$^}{*=?@#|'<>.()%!-");
    }
    if !crate::is_known_timezone(&row.timezone) {
        return bad("not a known time zone");
    }
    if !(0..=12).contains(&row.archive_in) || !(0..=12).contains(&row.close_in) {
        return bad("months run from 0 to 12");
    }
    // the column holds an object (migration 0010); the pin stores any value, which the app never sends (D-027)
    if !row.logo_props.is_object() {
        return bad("a project's logo is a JSON object");
    }
    Ok(())
}

/// Whether `user` is an active member of the workspace: a project's lead must be one (deviation D-026: the pin lets any user lead a
/// project and makes them its admin without making them a member of the workspace, which Thrust's schema does not allow).
pub async fn is_workspace_member(app: &App, scope: &WorkspaceScope, user: UserId) -> Result<bool, CoreError> {
    Ok(app.ports.db.is_workspace_member(scope, user).await?)
}

/// A user a project names (its lead, its default assignee) must exist, and a lead the request names must be a member of the
/// workspace (`check_lead`: the creation, and a change that names one).
async fn check_users(app: &App, scope: &WorkspaceScope, row: &ProjectRow, check_lead: bool) -> Result<(), CoreError> {
    for user in [row.project_lead, row.default_assignee].into_iter().flatten() {
        if app.ports.db.user_by_id(user).await?.is_none() {
            return Err(CoreError::Invalid("a project names a user who does not exist".into()));
        }
    }
    if let Some(lead) = row.project_lead.filter(|_| check_lead) {
        if !is_workspace_member(app, scope, lead).await? {
            return Err(CoreError::Invalid("a project's lead is a member of its workspace".into()));
        }
    }
    Ok(())
}

/// Plane's `DEFAULT_STATES`: name, colour, `sequence`, group, whether it is the project's default. The sixth is the one intake
/// uses; the lists of a project's states leave it out.
pub const DEFAULT_STATES: [(&str, &str, f64, StateGroup, bool); 6] = [
    ("Backlog", "#60646C", 15000.0, StateGroup::Backlog, true),
    ("Todo", "#60646C", 25000.0, StateGroup::Unstarted, false),
    ("In Progress", "#F59E0B", 35000.0, StateGroup::Started, false),
    ("Done", "#46A758", 45000.0, StateGroup::Completed, false),
    ("Cancelled", "#9AA4BC", 55000.0, StateGroup::Cancelled, false),
    ("Triage", "#4E5355", 65000.0, StateGroup::Triage, false),
];

/// The work-item list settings a person starts with in a project: the pin's defaults for its `ProjectUserProperty` (flat, unlike
/// the workspace's, which holds two of them one level deeper), and its preferences.
fn default_project_properties(app: &App, project: ProjectId, workspace: WorkspaceId, user: UserId, at: Timestamp) -> ProjectUserProperties {
    ProjectUserProperties {
        id: app.ports.ids.new_uuid(),
        project_id: project,
        workspace_id: workspace,
        user_id: user,
        filters: json!({
            "priority": null, "state": null, "state_group": null, "assignees": null, "created_by": null,
            "labels": null, "start_date": null, "target_date": null, "subscriber": null,
        }),
        display_filters: json!({
            "group_by": null, "order_by": "-created_at", "type": null, "sub_issue": true, "show_empty_groups": true,
            "layout": "list", "calendar_date_range": "",
        }),
        display_properties: json!({
            "assignee": true, "attachment_count": true, "created_on": true, "due_date": true, "estimate": true,
            "key": true, "labels": true, "link": true, "priority": true, "start_date": true, "state": true,
            "sub_issue_count": true, "updated_on": true,
        }),
        rich_filters: json!({}),
        preferences: json!({"pages": {"block_display": true}, "navigation": {"default_tab": "work_items", "hide_in_more_menu": []}}),
        // the adapter gives a project its member has just joined the place the pin does
        sort_order: 65535.0,
        created_at: at,
        updated_at: at,
    }
}

/// Whether the workspace has a project with this name, and one with this key (exact). A name or key that is `None` is not asked
/// about, and `except` is a project that is left out (the one being renamed).
pub async fn project_clashes(
    app: &App,
    scope: &WorkspaceScope,
    name: Option<&str>,
    identifier: Option<&str>,
    except: Option<ProjectId>,
) -> Result<ProjectClashes, CoreError> {
    Ok(app.ports.db.project_clashes(scope, name, identifier, except).await?)
}

/// Make a project. The caller must be an admin or a member of the workspace (a guest is `Forbidden`); the patch names at least a
/// name and a key. The caller becomes the project's admin, and so does the lead they name, if someone else. The project gets the
/// pin's five states and the intake one, and its members each a settings row. A name or key that is taken is `Conflict`, decided by
/// the database's unique indexes so that two creations racing for one name cannot both win; a rule a project must satisfy is
/// `Invalid`.
pub async fn create_project(app: &App, scope: &WorkspaceScope, patch: ProjectPatch) -> Result<ProjectDetail, ProjectError> {
    if scope.role() == Role::Guest {
        return Err(ProjectError::Forbidden);
    }
    let (Some(name), Some(identifier)) = (patch.name.clone(), patch.identifier.clone()) else {
        return Err(CoreError::Invalid("a project needs a name and a key".into()).into());
    };
    let (now, id, ws, me) = (app.ports.clock.now(), ProjectId(app.ports.ids.new_uuid()), scope.workspace_id(), scope.user_id());
    let mut row = ProjectRow::bare(id, ws, name, now);
    row.identifier = identifier;
    row.created_by = Some(me);
    patch.apply(&mut row);
    check_project(&row)?;
    check_users(app, scope, &row, true).await?;

    // the creator, then the lead when it is someone else; the pin lists a project's people newest first, and a row made after
    // another has a later time, so each is a microsecond after the one before
    let mut people = vec![me];
    if let Some(lead) = row.project_lead.filter(|l| *l != me) {
        people.push(lead);
    }
    let members = people
        .iter()
        .enumerate()
        .map(|(i, user)| {
            let at = now + TimeDelta::microseconds(i as i64);
            NewProjectMember {
                member: ProjectMemberRow {
                    id: app.ports.ids.new_uuid(),
                    project_id: id,
                    workspace_id: ws,
                    user_id: *user,
                    role: Role::Admin,
                    comment: None,
                    view_props: json!({}),
                    default_props: json!({}),
                    preferences: json!({}),
                    sort_order: 65535.0,
                    created_by: Some(me),
                    created_at: at,
                    updated_at: at,
                },
                properties: default_project_properties(app, id, ws, *user, at),
            }
        })
        .collect();
    let states = DEFAULT_STATES
        .iter()
        .map(|(name, color, sequence, group, is_default)| StateRow {
            id: app.ports.ids.new_uuid(),
            workspace_id: ws,
            project_id: id,
            name: (*name).to_string(),
            description: String::new(),
            color: (*color).to_string(),
            sequence: *sequence,
            group: *group,
            is_default: *is_default,
            created_by: Some(me),
            created_at: now,
            updated_at: now,
        })
        .collect();
    app.ports.db.insert_project(scope, &NewProject { project: row, members, states }).await?;
    app.ports.db.project_detail(scope, id).await?.ok_or_else(|| CoreError::Internal("a project that was just made is not there".into()).into())
}

/// The project as it is, when the caller may change it: the checks of [`update_project`] that come before it looks at what the
/// request says (the pin answers these first, so a caller who may not, or a project that is archived, is told so before the
/// request's fields are found wrong).
pub async fn updatable_project(app: &App, scope: &WorkspaceScope, id: ProjectId) -> Result<ProjectDetail, ProjectError> {
    let Some(detail) = app.ports.db.project_detail(scope, id).await? else {
        return Err(if scope.role() == Role::Admin { ProjectError::NotFound } else { ProjectError::Forbidden });
    };
    if scope.role() != Role::Admin && detail.listing.role != Some(Role::Admin) {
        return Err(ProjectError::Forbidden);
    }
    if detail.listing.project.archived_at.is_some() {
        return Err(ProjectError::Archived);
    }
    Ok(detail)
}

/// Change a project. The workspace's admins and the project's own admins may (anyone else in the workspace is `Forbidden`, also
/// for a project that is not there; a workspace admin gets `NotFound` for it), and not while the project is archived. What the
/// patch names is set, and the project is saved whole, as the pin saves it, with the caller as the one who changed it and the time.
pub async fn update_project(app: &App, scope: &WorkspaceScope, id: ProjectId, patch: ProjectPatch) -> Result<ProjectDetail, ProjectError> {
    let detail = updatable_project(app, scope, id).await?;
    let mut row = detail.listing.project;
    patch.apply(&mut row);
    row.updated_by = Some(scope.user_id());
    row.updated_at = app.ports.clock.now();
    check_project(&row)?;
    check_users(app, scope, &row, matches!(patch.project_lead, Some(Some(_)))).await?;
    if !app.ports.db.update_project(scope, &row).await? {
        return Err(ProjectError::NotFound);
    }
    app.ports.db.project_detail(scope, id).await?.ok_or(ProjectError::NotFound)
}

/// Open a project. A project that is not there or is archived is `NotFound`; a member of it gets it; anyone else in the workspace
/// is told it is `Secret` or that they are `NotAMember`, whatever their role in the workspace.
pub async fn open_project(app: &App, scope: &WorkspaceScope, id: ProjectId) -> Result<ProjectDetail, ProjectError> {
    let detail = app.ports.db.project_detail(scope, id).await?.filter(|d| d.listing.project.archived_at.is_none()).ok_or(ProjectError::NotFound)?;
    if detail.members.contains(&scope.user_id()) {
        return Ok(detail);
    }
    Err(if detail.listing.project.network == ProjectNetwork::Secret { ProjectError::Secret } else { ProjectError::NotAMember })
}

/// The caller's own row in a project with who they are, or `None` when they have none.
pub async fn my_project_member(app: &App, scope: &WorkspaceScope, project: ProjectId) -> Result<Option<ProjectMemberWithUser>, CoreError> {
    Ok(app.ports.db.project_member(scope, project).await?)
}

/// The people in the project who are active members of its workspace, newest row first.
pub async fn project_members(app: &App, scope: &Scope) -> Result<Vec<ProjectMemberRow>, CoreError> {
    Ok(app.ports.db.project_members(scope).await?)
}

/// The caller's work-item list settings for the project: made, with the pin's defaults, if their joining did not.
pub async fn project_properties(app: &App, scope: &Scope) -> Result<ProjectUserProperties, CoreError> {
    let new = default_project_properties(app, scope.project_id(), scope.workspace_id(), scope.user_id(), app.ports.clock.now());
    Ok(app.ports.db.ensure_project_user_properties(scope, &new).await?)
}

/// The project's labels by `sort_order`.
pub async fn project_labels(app: &App, scope: &Scope) -> Result<Vec<LabelRow>, CoreError> {
    Ok(app.ports.db.project_labels(scope).await?)
}

/// The project's states by `sequence`, the intake one left out.
pub async fn project_states(app: &App, scope: &Scope) -> Result<Vec<StateRow>, CoreError> {
    Ok(app.ports.db.project_states(scope).await?)
}

/// The states of every project of the workspace the caller is in, by `sequence`, the intake ones left out.
pub async fn workspace_states(app: &App, scope: &WorkspaceScope) -> Result<Vec<StateRow>, CoreError> {
    Ok(app.ports.db.workspace_states(scope).await?)
}

/// The project's intake state; `NotFound` when it has none (a project made here always has one).
pub async fn intake_state(app: &App, scope: &Scope) -> Result<StateRow, CoreError> {
    app.ports.db.triage_state(scope).await?.ok_or(CoreError::NotFound)
}

/// Counts for the projects of the workspace (all of them, or those in `only`), newest first, whoever asks.
pub async fn project_stats(app: &App, scope: &WorkspaceScope, only: Option<&[ProjectId]>) -> Result<Vec<ProjectStat>, CoreError> {
    Ok(app.ports.db.project_stats(scope, only).await?)
}
