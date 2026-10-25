//! What the app reads when it opens a workspace and its home page (S17, S19 to S26): the projects the member can see, the
//! sidebar, the work-item list settings, and the home page's widgets, links, recent visits and unread count.
//!
//! The shapes are the pin's, field for field. Where the pin is odd it is copied: two kinds of 403 (the views that check the
//! caller's role with a decorator answer `{"error"}`, the two that use a permission class answer `{"detail"}`), and the
//! work-item list settings holding their display filters one level deeper than any other list does.
//!
//! S19, S23, S25 and S26 are stubs (D-021): what they list does not exist yet (favourites, notifications, quick links, recent
//! visits), so they answer the empty list or the zero count the pin gives a member who has none. They still ask who the caller
//! is, so a stranger gets the pin's refusal and not an empty list.

use axum::extract::Path;
use axum::{Extension, Json};
use serde_json::{json, Map, Value};
use thrust_contracts::*;
use thrust_core::App;
use thrust_ports::*;

use crate::users::{ts, ts_or_null, Caller};
use crate::workspaces::{id_or_null, role_number, scope};
use crate::PlaneError;

/// The 403 of a view whose role check is Plane's `allow_permission` decorator.
pub(crate) fn refused_by_decorator() -> PlaneError {
    PlaneError::Forbidden(json!({"error": "You don't have the required permissions."}))
}

/// The 403 of a view whose role check is a DRF permission class.
pub(crate) fn refused_by_class() -> PlaneError {
    PlaneError::Forbidden(json!({"detail": "You do not have permission to perform this action."}))
}

/// The caller's way into the workspace for a view guarded by the decorator, which lets in the roles `allowed`.
pub(crate) async fn decorated(app: &App, who: &Principal, slug: String, allowed: &[Role]) -> Result<WorkspaceScope, PlaneError> {
    match scope(app, who, slug).await? {
        Some(scope) if allowed.contains(&scope.role()) => Ok(scope),
        _ => Err(refused_by_decorator()),
    }
}

pub(crate) const ALL_ROLES: [Role; 3] = [Role::Guest, Role::Member, Role::Admin];

/// Plane's `network` numbers: 0 secret, 2 public.
pub(crate) fn network_number(n: ProjectNetwork) -> u8 {
    match n {
        ProjectNetwork::Secret => 0,
        ProjectNetwork::Public => 2,
    }
}

/// A project as S17 lists it: the 21 keys of the pin's `.values()` query, not the serializer's longer shape.
fn project_json(l: &ProjectListing) -> Value {
    let p = &l.project;
    json!({
        "id": p.id.0,
        "name": p.name,
        "identifier": p.identifier,
        "sort_order": l.sort_order,
        "logo_props": p.logo_props,
        "member_role": l.role.map(role_number),
        // there is no intake yet, so no work item waits in one
        "intake_count": 0,
        "archived_at": ts_or_null(p.archived_at),
        "workspace": p.workspace_id.0,
        "cycle_view": p.cycle_view,
        "issue_views_view": p.issue_views_view,
        "module_view": p.module_view,
        "page_view": p.page_view,
        "inbox_view": p.intake_view,
        "guest_view_all_features": p.guest_view_all_features,
        "project_lead": id_or_null(p.project_lead),
        "network": network_number(p.network),
        "created_at": ts(p.created_at),
        "updated_at": ts(p.updated_at),
        "created_by": id_or_null(p.created_by),
        "updated_by": id_or_null(p.updated_by),
    })
}

/// S17. The projects of the workspace the caller can see, newest first.
pub(crate) async fn projects(Caller(who): Caller, Extension(app): Extension<App>, Path(slug): Path<String>) -> Result<Json<Value>, PlaneError> {
    let scope = decorated(&app, &who, slug, &ALL_ROLES).await?;
    let list = thrust_core::workspace_projects(&app, &scope).await?;
    Ok(Json(Value::Array(list.iter().map(project_json).collect())))
}

/// S19. What the caller has starred. A stub (D-021): nothing can be starred yet. A guest is refused: favourites are for
/// admins and members.
pub(crate) async fn favorites(Caller(who): Caller, Extension(app): Extension<App>, Path(slug): Path<String>) -> Result<Json<Value>, PlaneError> {
    decorated(&app, &who, slug, &[Role::Member, Role::Admin]).await?;
    Ok(Json(json!([])))
}

/// S20. The states of the projects the caller is in (not the archived ones, and without the intake state), by `sequence`. Each
/// state's `order` is its place among the listed states of its group over how many there are, so it counts the states of every
/// project at once. Any active member may ask.
pub(crate) async fn states(Caller(who): Caller, Extension(app): Extension<App>, Path(slug): Path<String>) -> Result<Json<Value>, PlaneError> {
    let scope = scope(&app, &who, slug).await?.ok_or_else(refused_by_class)?;
    let states = thrust_core::workspace_states(&app, &scope).await?;
    let orders = crate::projects::state_orders(&states);
    Ok(Json(Value::Array(states.iter().zip(orders).map(|(s, o)| crate::projects::state_json(s, Some(o))).collect())))
}

/// S21. The caller's sidebar: each entry's pinned flag and place, by key.
pub(crate) async fn sidebar_preferences(Caller(who): Caller, Extension(app): Extension<App>, Path(slug): Path<String>) -> Result<Json<Value>, PlaneError> {
    let scope = decorated(&app, &who, slug, &ALL_ROLES).await?;
    let rows = thrust_core::sidebar_preferences(&app, &scope).await?;
    let mut by_key = Map::new();
    for r in rows {
        by_key.insert(r.key, json!({"is_pinned": r.is_pinned, "sort_order": r.sort_order}));
    }
    Ok(Json(Value::Object(by_key)))
}

/// S22. The caller's work-item list settings for the workspace. Any active member may ask. The pin makes the row, so its
/// `created_by` is the caller and nothing has changed it (`updated_by` is empty).
pub(crate) async fn user_properties(Caller(who): Caller, Extension(app): Extension<App>, Path(slug): Path<String>) -> Result<Json<Value>, PlaneError> {
    let scope = scope(&app, &who, slug).await?.ok_or_else(refused_by_class)?;
    let p = thrust_core::user_properties(&app, &scope).await?;
    Ok(Json(json!({
        "id": p.id,
        "created_at": ts(p.created_at),
        "updated_at": ts(p.updated_at),
        "created_by": scope.user_id().0,
        "updated_by": null,
        "deleted_at": null,
        "workspace": scope.workspace_id().0,
        "user": scope.user_id().0,
        "filters": p.filters,
        "display_filters": p.display_filters,
        "display_properties": p.display_properties,
        "rich_filters": p.rich_filters,
        "navigation_project_limit": p.navigation_project_limit,
        "navigation_control_preference": p.navigation_control_preference,
    })))
}

/// S23. How many notifications the caller has not read, and how many of those mention them. A stub (D-021): nobody is
/// notified yet.
pub(crate) async fn unread_notifications(Caller(who): Caller, Extension(app): Extension<App>, Path(slug): Path<String>) -> Result<Json<Value>, PlaneError> {
    decorated(&app, &who, slug, &ALL_ROLES).await?;
    Ok(Json(json!({"total_unread_notifications_count": 0, "mention_unread_notifications_count": 0})))
}

/// S24. The widgets of the caller's home page: whether each shows, what it is set to, and where it goes.
pub(crate) async fn home_preferences(Caller(who): Caller, Extension(app): Extension<App>, Path(slug): Path<String>) -> Result<Json<Value>, PlaneError> {
    let scope = decorated(&app, &who, slug, &ALL_ROLES).await?;
    let rows = thrust_core::home_preferences(&app, &scope).await?;
    Ok(Json(Value::Array(
        rows.iter()
            .map(|r| json!({"key": r.key, "is_enabled": r.is_enabled, "config": r.config, "sort_order": r.sort_order}))
            .collect(),
    )))
}

/// S25. The caller's links on the home page. A stub (D-021): none can be added yet.
pub(crate) async fn quick_links(Caller(who): Caller, Extension(app): Extension<App>, Path(slug): Path<String>) -> Result<Json<Value>, PlaneError> {
    decorated(&app, &who, slug, &ALL_ROLES).await?;
    Ok(Json(json!([])))
}

/// S26. What the caller opened lately, with or without the `entity_name` filter. A stub (D-021): nothing records a visit yet.
pub(crate) async fn recent_visits(Caller(who): Caller, Extension(app): Extension<App>, Path(slug): Path<String>) -> Result<Json<Value>, PlaneError> {
    decorated(&app, &who, slug, &ALL_ROLES).await?;
    Ok(Json(json!([])))
}
