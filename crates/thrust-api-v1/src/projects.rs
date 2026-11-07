//! The projects of Plane's v1 API: list, lite list, create, retrieve, change, delete, archive and unarchive.
//!
//! Who may do what is the pin's `ProjectBasePermission`: a read is for any active member of the workspace; a creation for a member or
//! an admin of it; and a change, a deletion, archiving and unarchiving for a project admin, or a member of the project who administers
//! the workspace (core's `project_writer`). A read shows the projects the caller is in and the public ones. The rules are core's; this
//! is the pin's wire shape over them: the 43-key body (44 in a list, which adds the caller's `sort_order`), DRF's field validation of a
//! body, and the pin's two odd 409s.

use std::cmp::Ordering;
use std::collections::HashMap;

use axum::body::Bytes;
use axum::extract::RawQuery;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{json, Map, Value};
use thrust_contracts::chrono::SecondsFormat;
use thrust_contracts::{ProjectId, ProjectNetwork, Role, Timestamp, UserId, Uuid};
use thrust_core::{CoreError, ProjectError, ProjectPatch};
use thrust_ports::{ProjectListing, ProjectRow, WorkspaceScope};

use crate::fields::{bool_field, char_field, fail, int_field, no_such_pk, refuse_if_any, uuid_field, FieldErrors};
use crate::pagination::PageRequest;
use crate::request::{json_body, query_map, Body};
use crate::{Ctx, PlaneUuid, V1Error, V1Path};

/// The orders a project list takes (`PROJECT_ORDER_BY_ALLOWLIST`), each with an optional `-`; anything else is the default.
const ORDER_BY: [&str; 5] = ["created_at", "updated_at", "name", "network", "sort_order"];

pub(crate) fn routes() -> Router {
    Router::new()
        .route("/api/v1/workspaces/{slug}/projects/", get(list).post(create))
        .route("/api/v1/workspaces/{slug}/projects-lite/", get(list_lite))
        .route("/api/v1/workspaces/{slug}/projects/{project_id}/", get(retrieve).patch(update).delete(destroy))
        .route("/api/v1/workspaces/{slug}/projects/{project_id}/archive/", post(archive).delete(unarchive))
}

// --- the wire shape -----------------------------------------------------------------------------------------------------------

/// A DRF datetime: UTC, microseconds, `Z`.
fn ts(t: Timestamp) -> String {
    t.to_rfc3339_opts(SecondsFormat::Micros, true)
}

fn role_number(role: Role) -> u8 {
    match role {
        Role::Guest => 5,
        Role::Member => 15,
        Role::Admin => 20,
    }
}

fn network_number(network: ProjectNetwork) -> u8 {
    match network {
        ProjectNetwork::Secret => 0,
        ProjectNetwork::Public => 2,
    }
}

/// The pin's `ProjectSerializer`: 43 keys, and `sort_order` besides when the caller's place in the list is part of the answer. The
/// pin counts cycles and modules; Thrust has neither, so both are 0 (a deviation the register carries), and no deploy board exists.
fn project_body(l: &ProjectListing, total_members: i64, with_sort_order: bool) -> Map<String, Value> {
    let p = &l.project;
    let user = |u: Option<UserId>| u.map_or(Value::Null, |u| json!(u.0));
    let mut body = json!({
        "id": p.id.0,
        "workspace": p.workspace_id.0,
        "name": p.name,
        "identifier": p.identifier,
        "description": p.description,
        "description_text": p.description_text,
        "description_html": p.description_html,
        "network": network_number(p.network),
        "emoji": p.emoji,
        "icon_prop": p.icon_prop,
        "logo_props": p.logo_props,
        "cover_image": p.cover_image,
        // the cover image's asset, once there are assets, is its address; until then it is the text the project holds
        "cover_image_url": p.cover_image,
        "cover_image_asset": p.cover_image_asset,
        "default_assignee": user(p.default_assignee),
        "project_lead": user(p.project_lead),
        "module_view": p.module_view,
        "cycle_view": p.cycle_view,
        "issue_views_view": p.issue_views_view,
        "page_view": p.page_view,
        "intake_view": p.intake_view,
        "is_time_tracking_enabled": p.is_time_tracking_enabled,
        "is_issue_type_enabled": p.is_issue_type_enabled,
        "guest_view_all_features": p.guest_view_all_features,
        "estimate": p.estimate,
        "archive_in": p.archive_in,
        "close_in": p.close_in,
        "default_state": p.default_state,
        "archived_at": p.archived_at.map(ts),
        "timezone": p.timezone,
        "external_source": p.external_source,
        "external_id": p.external_id,
        "created_at": ts(p.created_at),
        "updated_at": ts(p.updated_at),
        "created_by": user(p.created_by),
        "updated_by": user(p.updated_by),
        "deleted_at": null,
        "is_member": l.role.is_some(),
        "member_role": l.role.map(role_number),
        "total_members": total_members,
        "total_cycles": 0,
        "total_modules": 0,
        "is_deployed": false,
    });
    if let (true, Some(map)) = (with_sort_order, body.as_object_mut()) {
        map.insert("sort_order".into(), json!(l.sort_order));
    }
    match body {
        Value::Object(map) => map,
        _ => Map::new(),
    }
}

/// The pin's `ProjectLiteSerializer`: nine keys.
fn lite_body(p: &ProjectRow) -> Value {
    json!({
        "id": p.id.0,
        "identifier": p.identifier,
        "name": p.name,
        "cover_image": p.cover_image,
        "icon_prop": p.icon_prop,
        "emoji": p.emoji,
        "description": p.description,
        "cover_image_url": p.cover_image,
        "archived_at": p.archived_at.map(ts),
    })
}

/// `fields=a,b`: the body keeps only those keys (the pin's `BaseSerializer`); a name it has no key for is ignored. No `fields` is all.
fn project_fields(body: Map<String, Value>, q: &HashMap<String, String>) -> Value {
    let wanted: Vec<&str> =
        q.get("fields").map(|f| f.split(',').filter(|s| !s.is_empty()).collect()).unwrap_or_default();
    if wanted.is_empty() {
        return Value::Object(body);
    }
    Value::Object(body.into_iter().filter(|(k, _)| wanted.contains(&k.as_str())).collect())
}

// --- the order ----------------------------------------------------------------------------------------------------------------

/// `sanitize_order_by`: the requested order if its key (without any `-`) is allowed, else the default. Returns the key and whether
/// it descends.
fn order_by(q: &HashMap<String, String>, default: &str) -> (String, bool) {
    let requested = q.get("order_by").map_or(default, String::as_str);
    let bare = requested.strip_prefix('-').unwrap_or(requested);
    let chosen = if ORDER_BY.contains(&bare) { requested } else { default };
    match chosen.strip_prefix('-') {
        Some(key) => (key.to_string(), true),
        None => (chosen.to_string(), false),
    }
}

/// A caller's place in the list that is not there sorts as the greatest, as Postgres does: last when ascending, first when descending.
fn place(a: Option<f64>, b: Option<f64>) -> Ordering {
    match (a, b) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Greater,
        (Some(_), None) => Ordering::Less,
        (Some(a), Some(b)) => a.total_cmp(&b),
    }
}

/// Order the list (it comes oldest first, ties by id, and the sort is stable, so that is how ties stay).
fn sort_projects(listings: &mut [ProjectListing], key: &str, descending: bool) {
    listings.sort_by(|a, b| {
        let ordering = match key {
            "created_at" => a.project.created_at.cmp(&b.project.created_at),
            "updated_at" => a.project.updated_at.cmp(&b.project.updated_at),
            "name" => a.project.name.cmp(&b.project.name),
            "network" => network_number(a.project.network).cmp(&network_number(b.project.network)),
            _ => place(a.sort_order, b.sort_order),
        };
        if descending {
            ordering.reverse()
        } else {
            ordering
        }
    });
}

// --- reading a body -----------------------------------------------------------------------------------------------------------

/// The pin's refusals of a project, as v1 words them. `state` is the default state a change named, for the message that quotes it.
fn project_error(e: ProjectError, state: Option<Uuid>) -> V1Error {
    match e {
        ProjectError::NotFound => V1Error::error(StatusCode::NOT_FOUND, "Project does not exist"),
        ProjectError::Forbidden | ProjectError::Secret | ProjectError::NotAMember => V1Error::PermissionDenied,
        ProjectError::Archived => V1Error::error(StatusCode::BAD_REQUEST, "Archived project cannot be updated"),
        ProjectError::NotAState => {
            let mut errors = FieldErrors::new();
            fail(&mut errors, "default_state", no_such_pk(state.unwrap_or_default()));
            V1Error::Fields(errors)
        }
        // the pin answers any integrity error that says "already exists" with the name's message, whichever of the two unique
        // indexes it was: a project made through v1 has no row of its own in the table of keys that its own check reads
        ProjectError::Core(CoreError::Conflict(_)) => {
            V1Error::Error(StatusCode::CONFLICT, json!({ "name": "The project name is already taken" }))
        }
        ProjectError::Core(other) => other.into(),
    }
}

/// A DRF `ValidationError("message")` raised from `validate()`: it has no field, so it is a non-field error.
fn non_field(message: &str) -> V1Error {
    let mut errors = FieldErrors::new();
    errors.insert("non_field_errors".into(), json!([message]));
    V1Error::Fields(errors)
}

/// What a body says of a project, read as the pin's `ProjectCreateSerializer` (and, for a change, `ProjectUpdateSerializer`) reads
/// it: every field it knows is checked and what is wrong is collected into one 400; then `validate()`, which stops at its first
/// refusal. A field it does not know (a `network`, an `id`) is ignored.
async fn project_patch(
    ctx: &Ctx,
    scope: &WorkspaceScope,
    b: &Map<String, Value>,
    partial: bool,
) -> Result<ProjectPatch, V1Error> {
    let mut errors = FieldErrors::new();
    let mut p = ProjectPatch::default();

    // reads a field if the body names it, and files what is wrong with it
    fn read<T>(
        errors: &mut FieldErrors,
        b: &Map<String, Value>,
        field: &str,
        f: impl FnOnce(&Value) -> Result<T, String>,
    ) -> Option<T> {
        let v = b.get(field)?;
        match f(v) {
            Ok(x) => Some(x),
            Err(m) => {
                fail(errors, field, m);
                None
            }
        }
    }
    let text = |max: usize, blank: bool, null: bool| move |v: &Value| char_field(v, Some(max), blank, null);

    // the two a creation must have
    for field in ["name", "identifier"] {
        if !partial && !b.contains_key(field) {
            fail(&mut errors, field, "This field is required.");
        }
    }
    let max_name = thrust_core::MAX_PROJECT_NAME;
    let max_key = thrust_core::MAX_PROJECT_IDENTIFIER;
    p.name = read(&mut errors, b, "name", text(max_name, false, false)).flatten();
    // the model saves a key in upper case
    p.identifier =
        read(&mut errors, b, "identifier", text(max_key, false, false)).flatten().map(|key| key.to_uppercase());
    // a `TextField` the column allows blank, a `CharField` or `TextField` that allows null
    p.description = read(&mut errors, b, "description", |v| char_field(v, None, true, false)).flatten();
    p.emoji = read(&mut errors, b, "emoji", text(255, true, true));
    p.cover_image = read(&mut errors, b, "cover_image", |v| char_field(v, None, true, true));
    p.external_source = read(&mut errors, b, "external_source", text(255, true, true));
    p.external_id = read(&mut errors, b, "external_id", text(255, true, true));
    p.icon_prop = b.get("icon_prop").map(|v| if v.is_null() { None } else { Some(v.clone()) });
    p.module_view = read(&mut errors, b, "module_view", bool_field);
    p.cycle_view = read(&mut errors, b, "cycle_view", bool_field);
    p.issue_views_view = read(&mut errors, b, "issue_views_view", bool_field);
    p.page_view = read(&mut errors, b, "page_view", bool_field);
    p.intake_view = read(&mut errors, b, "intake_view", bool_field);
    p.guest_view_all_features = read(&mut errors, b, "guest_view_all_features", bool_field);
    p.is_issue_type_enabled = read(&mut errors, b, "is_issue_type_enabled", bool_field);
    p.is_time_tracking_enabled = read(&mut errors, b, "is_time_tracking_enabled", bool_field);
    let months = |v: &Value| int_field(v, 0, 12).and_then(|n| i32::try_from(n).map_err(|e| e.to_string()));
    p.archive_in = read(&mut errors, b, "archive_in", months);
    p.close_in = read(&mut errors, b, "close_in", months);
    p.timezone = read(&mut errors, b, "timezone", |v| {
        let zone = char_field(v, None, false, false)?.unwrap_or_default();
        if thrust_core::is_known_timezone(&zone) {
            Ok(zone)
        } else {
            Err(format!("\"{zone}\" is not a valid choice."))
        }
    });

    // the fields that name a row: a user who is there; for a change, a state and an estimate
    for field in ["project_lead", "default_assignee"] {
        let Some(v) = b.get(field) else { continue };
        match uuid_field(v) {
            Err(m) => fail(&mut errors, field, m),
            Ok(None) => set_user(&mut p, field, None),
            Ok(Some(id)) if ctx.app.ports.db.user_by_id(UserId(id)).await?.is_none() => {
                fail(&mut errors, field, no_such_pk(id));
            }
            Ok(Some(id)) => set_user(&mut p, field, Some(UserId(id))),
        }
    }
    if partial {
        p.default_state = read(&mut errors, b, "default_state", uuid_field);
        // there are no estimates here, so no key names one; null names none
        if let Some(id) = read(&mut errors, b, "estimate", uuid_field).flatten() {
            fail(&mut errors, "estimate", no_such_pk(id));
        }
    }
    refuse_if_any(errors)?;

    // `validate()`, which raises at the first thing wrong
    if p.name.as_deref().is_some_and(thrust_core::has_forbidden_project_chars) {
        return Err(non_field("Project name cannot contain special characters."));
    }
    // (upper-casing does not change whether a character is forbidden)
    if p.identifier.as_deref().is_some_and(thrust_core::has_forbidden_project_chars) {
        return Err(non_field("Project identifier cannot contain special characters."));
    }
    if let Some(Some(lead)) = p.project_lead {
        if !thrust_core::is_workspace_member(&ctx.app, scope, lead).await? {
            return Err(V1Error::Error(
                StatusCode::BAD_REQUEST,
                json!({ "project_lead": "The provided user is not a member of this workspace." }),
            ));
        }
    }
    if let Some(Some(assignee)) = p.default_assignee {
        if !thrust_core::is_workspace_member(&ctx.app, scope, assignee).await? {
            return Err(non_field("Default assignee should be a user in the workspace"));
        }
    }
    Ok(p)
}

fn set_user(p: &mut ProjectPatch, field: &str, user: Option<UserId>) {
    if field == "project_lead" {
        p.project_lead = Some(user);
    } else {
        p.default_assignee = Some(user);
    }
}

// --- the routes ---------------------------------------------------------------------------------------------------------------

/// `GET .../projects/`: the projects the caller sees, ordered and paged.
async fn list(V1Path(slug): V1Path<String>, ctx: Ctx, RawQuery(raw): RawQuery) -> Result<Json<Value>, V1Error> {
    let scope = ctx.workspace_scope(&slug, Role::Guest).await?;
    let q = query_map(raw.as_deref());
    let plan =
        PageRequest::parse(q.get("per_page").map(String::as_str), q.get("cursor").map(String::as_str))?.plan()?;
    let (key, descending) = order_by(&q, "sort_order");

    let mut listings = thrust_core::visible_projects(&ctx.app, &scope).await?;
    sort_projects(&mut listings, &key, descending);
    let total = listings.len();
    let page = plan.slice(listings);

    let ids: Vec<ProjectId> = page.iter().map(|l| l.project.id).collect();
    let members: HashMap<ProjectId, i64> = thrust_core::project_stats(&ctx.app, &scope, Some(&ids))
        .await?
        .into_iter()
        .map(|s| (s.id, s.total_members))
        .collect();
    let results = page
        .iter()
        .map(|l| project_fields(project_body(l, members.get(&l.project.id).copied().unwrap_or(0), true), &q))
        .collect();
    Ok(Json(plan.envelope(results, total)))
}

/// `GET .../projects-lite/`: the same projects in nine keys; archived ones only when asked for.
async fn list_lite(V1Path(slug): V1Path<String>, ctx: Ctx, RawQuery(raw): RawQuery) -> Result<Json<Value>, V1Error> {
    let scope = ctx.workspace_scope(&slug, Role::Guest).await?;
    let q = query_map(raw.as_deref());
    let plan =
        PageRequest::parse(q.get("per_page").map(String::as_str), q.get("cursor").map(String::as_str))?.plan()?;
    let (key, descending) = order_by(&q, "-created_at");
    let with_archived = q.get("include_archived").is_some_and(|v| matches!(v.to_lowercase().as_str(), "true" | "1"));

    let mut listings = thrust_core::visible_projects(&ctx.app, &scope).await?;
    listings.retain(|l| with_archived || l.project.archived_at.is_none());
    sort_projects(&mut listings, &key, descending);
    let total = listings.len();
    let results = plan.slice(listings).iter().map(|l| lite_body(&l.project)).collect();
    Ok(Json(plan.envelope(results, total)))
}

/// `POST .../projects/`: make a project; the caller becomes its admin.
async fn create(
    V1Path(slug): V1Path<String>,
    ctx: Ctx,
    headers: HeaderMap,
    body: Bytes,
) -> Result<(StatusCode, Json<Value>), V1Error> {
    let scope = ctx.workspace_scope(&slug, Role::Member).await?;
    let map = match json_body(&headers, &body)? {
        Body::Object(map) => map,
        // `{**request.data}` of a list raises, and the view's catch-all answers it
        Body::Array => return Err(V1Error::error(StatusCode::INTERNAL_SERVER_ERROR, "An unexpected error occurred")),
    };
    let patch = project_patch(&ctx, &scope, &map, false).await?;
    let detail = thrust_core::create_project(&ctx.app, &scope, patch).await.map_err(|e| project_error(e, None))?;
    let members = i64::try_from(detail.members.len()).unwrap_or(i64::MAX);
    Ok((StatusCode::CREATED, Json(Value::Object(project_body(&detail.listing, members, false)))))
}

/// `GET .../projects/{id}/`: a project the caller is in or a public one.
async fn retrieve(
    V1Path((slug, id)): V1Path<(String, PlaneUuid)>,
    ctx: Ctx,
    RawQuery(raw): RawQuery,
) -> Result<Json<Value>, V1Error> {
    let scope = ctx.workspace_scope(&slug, Role::Guest).await?;
    let detail = thrust_core::visible_project(&ctx.app, &scope, ProjectId(id.0)).await.map_err(|e| match e {
        ProjectError::NotFound => V1Error::NotFound,
        other => project_error(other, None),
    })?;
    let members = i64::try_from(detail.members.len()).unwrap_or(i64::MAX);
    Ok(Json(project_fields(project_body(&detail.listing, members, false), &query_map(raw.as_deref()))))
}

/// `PATCH .../projects/{id}/`: change a project, for a writer. The pin reads the body first, then refuses an archived project, then
/// checks the fields.
async fn update(
    V1Path((slug, id)): V1Path<(String, PlaneUuid)>,
    ctx: Ctx,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<Value>, V1Error> {
    let id = ProjectId(id.0);
    let scope = ctx.workspace_access(&slug).await?;
    let before = thrust_core::project_writer(&ctx.app, &scope, id).await.map_err(|e| project_error(e, None))?;
    ctx.throttle().await?;

    let map = match json_body(&headers, &body)? {
        Body::Object(map) => map,
        // `{**request.data, ...}` of a list raises, and nothing in this view catches it
        Body::Array => return Err(crate::plane_500()),
    };
    if before.listing.project.archived_at.is_some() {
        return Err(project_error(ProjectError::Archived, None));
    }
    let patch = project_patch(&ctx, &scope, &map, true).await?;
    let state = patch.default_state.flatten();
    let detail = thrust_core::change_project(&ctx.app, &scope, id, patch).await.map_err(|e| project_error(e, state))?;
    let members = i64::try_from(detail.members.len()).unwrap_or(i64::MAX);
    Ok(Json(Value::Object(project_body(&detail.listing, members, false))))
}

/// `DELETE .../projects/{id}/`: delete a project, for a writer.
async fn destroy(V1Path((slug, id)): V1Path<(String, PlaneUuid)>, ctx: Ctx) -> Result<StatusCode, V1Error> {
    let id = ProjectId(id.0);
    let scope = ctx.workspace_access(&slug).await?;
    thrust_core::project_writer(&ctx.app, &scope, id).await.map_err(|e| project_error(e, None))?;
    ctx.throttle().await?;
    thrust_core::delete_project(&ctx.app, &scope, id).await.map_err(|e| project_error(e, None))?;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST .../projects/{id}/archive/`.
async fn archive(V1Path((slug, id)): V1Path<(String, PlaneUuid)>, ctx: Ctx) -> Result<StatusCode, V1Error> {
    set_archived(&slug, id, ctx, true).await
}

/// `DELETE .../projects/{id}/archive/`.
async fn unarchive(V1Path((slug, id)): V1Path<(String, PlaneUuid)>, ctx: Ctx) -> Result<StatusCode, V1Error> {
    set_archived(&slug, id, ctx, false).await
}

async fn set_archived(slug: &str, id: PlaneUuid, ctx: Ctx, archived: bool) -> Result<StatusCode, V1Error> {
    let id = ProjectId(id.0);
    let scope = ctx.workspace_access(slug).await?;
    thrust_core::project_writer(&ctx.app, &scope, id).await.map_err(|e| project_error(e, None))?;
    ctx.throttle().await?;
    thrust_core::set_project_archived(&ctx.app, &scope, id, archived).await.map_err(|e| project_error(e, None))?;
    Ok(StatusCode::NO_CONTENT)
}
