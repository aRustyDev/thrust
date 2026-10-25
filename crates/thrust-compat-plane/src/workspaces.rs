//! Workspaces: the user's own settings and list, the slug check, making a workspace, and the three reads a workspace page
//! makes about itself (S08, S09, S12 to S16, S18).
//!
//! The shapes are the pin's, field for field. Where the pin is odd it is copied, because the fixtures show it and the React
//! app was written against it: a user who is not a member gets a 403 from the member list and the project roles but a
//! blank 200 from `workspace-members/me`, and the two 403s have different bodies.

use std::collections::HashMap;

use axum::body::Bytes;
use axum::extract::{Path, Query};
use axum::http::StatusCode;
use axum::{Extension, Json};
use serde_json::{json, Map, Value};
use thrust_contracts::*;
use thrust_core::{App, CoreError, NewWorkspaceRequest};
use thrust_ports::*;

use crate::fields::{char_field, fail, py_str, FieldErrors};
use crate::users::{non_empty, object_body, ts, Caller};
use crate::PlaneError;

/// Plane's role numbers: Guest 5, Member 15, Admin 20.
fn role_number(r: Role) -> u8 {
    match r {
        Role::Guest => 5,
        Role::Member => 15,
        Role::Admin => 20,
    }
}

fn id_or_null(id: Option<UserId>) -> Value {
    id.map_or(Value::Null, |u| json!(u.0))
}

/// A workspace as S09 lists it and S14 answers a creation (17 keys). `created_by` is the owner: a workspace is made by the
/// person who owns it.
fn workspace_json(l: &WorkspaceListing) -> Value {
    let w = &l.workspace;
    json!({
        "id": w.id.0,
        "created_at": ts(w.created_at),
        "updated_at": ts(w.updated_at),
        "created_by": id_or_null(w.owner_id),
        "updated_by": null,
        "deleted_at": null,
        "name": w.name,
        "logo": null,
        "logo_asset": null,
        "logo_url": null,
        "owner": id_or_null(w.owner_id),
        "slug": w.slug.as_str(),
        "organization_size": w.organization_size,
        "timezone": w.timezone,
        "background_color": w.background_color,
        "total_members": l.total_members,
        "role": role_number(l.role),
    })
}

/// A member's view settings before they have set any: the pin's `get_default_props()`, which is also what `view_props` starts
/// as.
fn default_view_props() -> Value {
    json!({
        "filters": {
            "priority": null, "state": null, "state_group": null, "assignees": null, "created_by": null,
            "labels": null, "start_date": null, "target_date": null, "subscriber": null,
        },
        "display_filters": {
            "group_by": null, "order_by": "-created_at", "type": null, "sub_issue": true, "show_empty_groups": true,
            "layout": "list", "calendar_date_range": "",
        },
        "display_properties": {
            "assignee": true, "attachment_count": true, "created_on": true, "due_date": true, "estimate": true,
            "key": true, "labels": true, "link": true, "priority": true, "start_date": true, "state": true,
            "sub_issue_count": true, "updated_on": true,
        },
    })
}

/// The six per-member settings, each the member's own where they have set it and the pin's default where not.
fn member_props(m: &WorkspaceMember) -> [(&'static str, Value); 6] {
    let set = |key: &str, default: Value| m.props.get(key).cloned().unwrap_or(default);
    [
        ("view_props", set("view_props", default_view_props())),
        ("default_props", set("default_props", default_view_props())),
        ("issue_props", set("issue_props", json!({"subscribed": true, "assigned": true, "created": true, "all_issues": true}))),
        ("getting_started_checklist", set("getting_started_checklist", json!({}))),
        ("tips", set("tips", json!({}))),
        ("explored_features", set("explored_features", json!({}))),
    ]
}

/// A membership row as the member routes show it (minus `member`, which S15 and S18 spell differently). `created_by` is the
/// member themselves: a membership is made by whoever joins, or is seeded, until invitations exist.
fn member_json(m: &WorkspaceMember) -> Map<String, Value> {
    let mut row = Map::new();
    row.insert("id".into(), json!(m.id));
    row.insert("workspace".into(), json!(m.workspace_id.0));
    row.insert("role".into(), json!(role_number(m.role)));
    row.insert("company_role".into(), json!(m.company_role.clone().unwrap_or_default()));
    row.insert("is_active".into(), json!(m.is_active));
    row.insert("created_at".into(), json!(ts(m.created_at)));
    row.insert("updated_at".into(), json!(ts(m.updated_at)));
    row.insert("created_by".into(), json!(m.user_id.0));
    row.insert("updated_by".into(), Value::Null);
    row.insert("deleted_at".into(), Value::Null);
    for (k, v) in member_props(m) {
        row.insert(k.into(), v);
    }
    row
}

/// S12. Invitations belong to a later phase (F-A07, P1): nobody has one, so the list is empty. A stub (D-021).
pub(crate) async fn invitations(Caller(_): Caller) -> Json<Value> {
    Json(json!([]))
}

/// S09. The workspaces the caller is an active member of, newest first.
pub(crate) async fn my_workspaces(Caller(who): Caller, Extension(app): Extension<App>) -> Result<Json<Value>, PlaneError> {
    let list = thrust_core::my_workspaces(&app, &who).await?;
    Ok(Json(Value::Array(list.iter().map(workspace_json).collect())))
}

/// S08. Which workspace the app opens for the caller. The last one they used, if they are still an active member of it; else
/// the one they joined first. `invites` is 0 until invitations exist (D-021).
pub(crate) async fn settings(Caller(who): Caller, Extension(app): Extension<App>) -> Result<Json<Value>, PlaneError> {
    let me = thrust_core::me(&app, &who).await?;
    let choice = thrust_core::workspace_choice(&app, &me).await?;
    let workspace = match &choice.last {
        Some(w) => json!({
            "last_workspace_id": w.id.0,
            "last_workspace_slug": w.slug.as_str(),
            "last_workspace_name": w.name,
            "last_workspace_logo": "",
            "fallback_workspace_id": w.id.0,
            "fallback_workspace_slug": w.slug.as_str(),
            "invites": 0,
        }),
        // no `last_workspace_name` or `last_workspace_logo` here: the pin leaves them out
        None => json!({
            "last_workspace_id": null,
            "last_workspace_slug": null,
            "fallback_workspace_id": choice.fallback.as_ref().map(|w| w.id.0),
            "fallback_workspace_slug": choice.fallback.as_ref().map(|w| w.slug.as_str()),
            "invites": 0,
        }),
    };
    Ok(Json(json!({"id": me.user.id.0, "email": me.user.email.unwrap_or_default(), "workspace": workspace})))
}

/// S13. Whether the slug can be taken. Case sensitive, and a reserved slug is never available.
pub(crate) async fn slug_check(
    Caller(_): Caller,
    Extension(app): Extension<App>,
    Query(q): Query<HashMap<String, String>>,
) -> Result<Json<Value>, PlaneError> {
    let Some(slug) = q.get("slug").filter(|s| !s.is_empty()) else {
        return Err(PlaneError::BadRequest(json!({"error": "Workspace Slug is required"})));
    };
    Ok(Json(json!({"status": thrust_core::slug_available(&app, slug).await?})))
}

/// Python's truthiness of a JSON value, which the pin's `if not name or not slug` is.
fn truthy(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64() != Some(0.0),
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Array(a)) => !a.is_empty(),
        Some(Value::Object(o)) => !o.is_empty(),
    }
}

/// S14. Make a workspace. The pin checks in two stages and says so in the shape of its refusals: the view's own checks answer
/// `{"error": ...}` (both fields given, their lengths, a URL in the name), then the serializer's answer
/// `{field: [message]}` for everything it finds wrong at once. The same order is kept.
pub(crate) async fn create(Caller(who): Caller, Extension(app): Extension<App>, body: Bytes) -> Result<(StatusCode, Json<Value>), PlaneError> {
    let b = object_body(&body)?;
    let error = |message: &str| PlaneError::BadRequest(json!({ "error": message }));
    if !truthy(b.get("name")) || !truthy(b.get("slug")) {
        return Err(error("Both name and slug are required"));
    }
    let (name, slug) = (py_str(&b["name"]), py_str(&b["slug"]));
    if name.chars().count() > thrust_core::MAX_WORKSPACE_NAME || slug.chars().count() > thrust_core::MAX_WORKSPACE_SLUG {
        return Err(error("The maximum length for name is 80 and for slug is 48"));
    }
    if thrust_core::contains_url(&name) {
        return Err(error("Name cannot contain a URL"));
    }

    // the serializer: every field is checked, and every refusal reported
    let mut errors = FieldErrors::new();
    let name = name.trim().to_string();
    let slug = slug.trim().to_string();
    if name.is_empty() {
        fail(&mut errors, "name", "This field may not be blank.");
    } else if !thrust_core::name_has_alphanumeric(&name) {
        fail(&mut errors, "name", "Name must contain at least one letter or number");
    }
    if !thrust_core::slug_has_valid_characters(&slug) {
        fail(&mut errors, "slug", "Enter a valid \"slug\" consisting of letters, numbers, underscores or hyphens.");
    } else if thrust_core::is_reserved_slug(&slug) {
        fail(&mut errors, "slug", "Slug is not valid");
    } else if !thrust_core::slug_available(&app, &slug).await? {
        fail(&mut errors, "slug", "Workspace with this slug already exists.");
    }
    let organization_size = match b.get("organization_size") {
        None => None,
        Some(v) => match char_field(v, Some(thrust_core::MAX_ORGANIZATION_SIZE), true, true) {
            Ok(size) => size,
            Err(m) => {
                fail(&mut errors, "organization_size", m);
                None
            }
        },
    };
    let timezone = match b.get("timezone") {
        None => None,
        Some(Value::Null) => {
            fail(&mut errors, "timezone", "This field may not be null.");
            None
        }
        Some(v) => {
            let text = py_str(v);
            if thrust_core::is_known_timezone(&text) {
                Some(text)
            } else {
                fail(&mut errors, "timezone", format!("\"{text}\" is not a valid choice."));
                None
            }
        }
    };
    if !errors.is_empty() {
        return Err(PlaneError::Fields(errors));
    }

    // `company_role` is read straight off the body and kept as given
    let company_role = match b.get("company_role") {
        None => Some(String::new()),
        Some(Value::Null) => None,
        Some(v) => Some(py_str(v)),
    };
    let request = NewWorkspaceRequest { name, slug, organization_size, company_role, timezone };
    match thrust_core::create_workspace(&app, &who, request).await {
        Ok(listing) => Ok((StatusCode::CREATED, Json(workspace_json(&listing)))),
        // another request took the slug between the check and the insert: the unique index decided
        Err(CoreError::Conflict(_)) => Err(PlaneError::Conflict(json!({"slug": "The workspace with the slug already exists"}))),
        Err(e) => Err(e.into()),
    }
}

/// The caller's way into the workspace the slug names, or `None` when there is none (no such workspace, or not an active
/// member: the same answer, so nobody can probe for workspaces).
async fn scope(app: &App, who: &Principal, slug: String) -> Result<Option<WorkspaceScope>, PlaneError> {
    match thrust_core::authorize_workspace(app, who, &WorkspaceSlug::new(slug)).await {
        Ok(scope) => Ok(Some(scope)),
        Err(CoreError::NotFound) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// What the pin answers `workspace-members/me/` for a caller who is not an active member: not a 403 but a 200 holding the
/// serializer's initial values (the fixture records them), because the view serializes the `None` its query found.
fn blank_member() -> Value {
    json!({
        "company_role": "", "created_by": null, "default_props": null, "deleted_at": null, "explored_features": null,
        "getting_started_checklist": null, "is_active": false, "issue_props": null, "member": null, "role": null,
        "tips": null, "updated_by": null, "view_props": null, "workspace": null,
    })
}

/// S15. The caller's own membership: their role (which every permission check in the app reads) and their settings.
pub(crate) async fn member_me(Caller(who): Caller, Extension(app): Extension<App>, Path(slug): Path<String>) -> Result<Json<Value>, PlaneError> {
    let Some(scope) = scope(&app, &who, slug).await? else { return Ok(Json(blank_member())) };
    let m = thrust_core::my_membership(&app, &scope).await?;
    let mut row = member_json(&m);
    row.insert("member".into(), json!(m.user_id.0));
    // work-item drafts do not exist yet
    row.insert("draft_issue_count".into(), json!(0));
    Ok(Json(Value::Object(row)))
}

/// S16. The caller's role in each project of the workspace, by project id.
pub(crate) async fn project_roles(Caller(who): Caller, Extension(app): Extension<App>, Path(slug): Path<String>) -> Result<Json<Value>, PlaneError> {
    let Some(scope) = scope(&app, &who, slug).await? else {
        return Err(PlaneError::Forbidden(json!({"detail": "You do not have permission to perform this action."})));
    };
    let roles = thrust_core::my_project_roles(&app, &scope).await?;
    Ok(Json(Value::Object(roles.into_iter().map(|(p, r)| (p.0.to_string(), json!(role_number(r)))).collect())))
}

/// S18. Everyone in the workspace, deactivated members too (the app shows them as suspended). A guest sees the people
/// without their email and sign-in method.
pub(crate) async fn members(Caller(who): Caller, Extension(app): Extension<App>, Path(slug): Path<String>) -> Result<Json<Value>, PlaneError> {
    let Some(scope) = scope(&app, &who, slug).await? else {
        return Err(PlaneError::Forbidden(json!({"error": "You don't have the required permissions."})));
    };
    let guest = scope.role() == Role::Guest;
    let rows = thrust_core::members(&app, &scope).await?;
    let rows: Vec<Value> = rows
        .iter()
        .map(|r| {
            let u = &r.user;
            let mut member = Map::new();
            member.insert("id".into(), json!(u.id.0));
            member.insert("first_name".into(), json!(u.first_name));
            member.insert("last_name".into(), json!(u.last_name));
            member.insert("avatar".into(), json!(u.avatar));
            member.insert("avatar_url".into(), non_empty(&u.avatar));
            member.insert("display_name".into(), json!(u.display_name));
            member.insert("is_bot".into(), json!(false));
            if !guest {
                member.insert("email".into(), json!(u.email.clone().unwrap_or_default()));
                member.insert("last_login_medium".into(), json!("email"));
            }
            let mut row = member_json(&r.member);
            row.insert("member".into(), Value::Object(member));
            Value::Object(row)
        })
        .collect();
    Ok(Json(Value::Array(rows)))
}
