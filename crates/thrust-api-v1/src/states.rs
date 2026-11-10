//! The states of Plane's v1 API: list, create, retrieve, change and delete (`/api/v1/workspaces/{slug}/projects/{id}/states/`).
//!
//! Who may do what is the pin's: a read is for any member of the project, and a write for a project **admin** (`ProjectAdminPermission`:
//! a member of the project is refused, as is a workspace admin who is not in it). A read shows the states that are not the intake
//! one, and none of an archived project. The rules are core's (`thrust_core::states`); this is the pin's wire shape over them: the
//! 18-key body, DRF's field validation, a create that answers 200, and the three different answers to a name that is taken.

use axum::body::Bytes;
use axum::extract::RawQuery;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::get;
use axum::{Json, Router};
use serde_json::{json, Map, Value};
use thrust_contracts::{ProjectId, StateGroup, UserId};
use thrust_core::{Permission, StateError, StatePatch};
use thrust_ports::StateRow;

use crate::fields::{
    bool_field, char_field, choice_field, float_field, name_was_padded, not_a_dictionary, read, refuse_if_any,
    FieldErrors,
};
use crate::pagination::PageRequest;
use crate::projects::ts;
use crate::request::{json_body, pick_fields, query_map, Body};
use crate::{with_allow, Ctx, PlaneUuid, V1Error, V1Path};

/// The groups a state may be in (`StateGroup`), as DRF's choice field lists them.
const GROUPS: [&str; 6] = ["backlog", "unstarted", "started", "completed", "cancelled", "triage"];

pub(crate) fn routes() -> Router {
    Router::new()
        .route(
            "/api/v1/workspaces/{slug}/projects/{project_id}/states/",
            with_allow("GET, POST", get(list).post(create)),
        )
        .route(
            "/api/v1/workspaces/{slug}/projects/{project_id}/states/{state_id}/",
            with_allow("GET, PATCH, DELETE", get(retrieve).patch(update).delete(destroy)),
        )
}

// --- the wire shape -----------------------------------------------------------------------------------------------------------

/// Django's `slugify` of an ASCII name: lower case, what is not a word character, a space or a hyphen dropped, runs of hyphens and
/// spaces made one hyphen, and hyphens and underscores trimmed from the ends. Django first folds accents to ASCII (`é` to `e`);
/// here a character that is not ASCII is dropped (the register carries that difference).
pub fn slugify(name: &str) -> String {
    let space = |c: char| matches!(c, ' ' | '\t' | '\n' | '\r' | '\x0b' | '\x0c' | '\x1c'..='\x1f');
    let mut out = String::new();
    let mut in_run = false;
    for c in name.to_lowercase().chars() {
        if c == '-' || space(c) {
            if !in_run {
                out.push('-');
                in_run = true;
            }
        } else if c.is_ascii_alphanumeric() || c == '_' {
            out.push(c);
            in_run = false;
        }
    }
    out.trim_matches(['-', '_']).to_string()
}

/// The pin's `StateSerializer`: 18 keys. `is_triage` is whether the group is the intake's, which a state of this list never is; the
/// pin stores it apart from the group, and a request can set it on a state of another group (the register carries that).
fn state_body(s: &StateRow) -> Map<String, Value> {
    let user = |u: Option<UserId>| u.map_or(Value::Null, |u| json!(u.0));
    let body = json!({
        "id": s.id,
        "created_at": ts(s.created_at),
        "updated_at": ts(s.updated_at),
        "deleted_at": null,
        "name": s.name,
        "description": s.description,
        "color": s.color,
        "slug": slugify(&s.name),
        "sequence": s.sequence,
        "group": s.group.as_str(),
        "is_triage": s.group == StateGroup::Triage,
        "default": s.is_default,
        "external_source": s.external_source,
        "external_id": s.external_id,
        "created_by": user(s.created_by),
        "updated_by": user(s.updated_by),
        "project": s.project_id.0,
        "workspace": s.workspace_id.0,
    });
    match body {
        Value::Object(map) => map,
        _ => Map::new(),
    }
}

// --- reading a body -----------------------------------------------------------------------------------------------------------

/// What the pin answers for a refusal. A name that is taken is a 409 naming the state when a state is made, and the base view's 400
/// "The payload is not valid" when one is changed (the database's integrity error reaches the base view there).
fn state_error(e: StateError, changing: bool) -> V1Error {
    match e {
        StateError::NotFound => V1Error::NotFound,
        StateError::DefaultState => V1Error::error(StatusCode::BAD_REQUEST, "Default state cannot be deleted"),
        StateError::NotEmpty => {
            V1Error::error(StatusCode::BAD_REQUEST, "The state is not empty, only empty states can be deleted")
        }
        StateError::NameTaken { .. } if changing => V1Error::error(StatusCode::BAD_REQUEST, "The payload is not valid"),
        StateError::NameTaken { id } => V1Error::Error(
            StatusCode::CONFLICT,
            json!({ "error": "State with the same name already exists in the project", "id": id }),
        ),
        StateError::ExternalTaken { id } => V1Error::Error(
            StatusCode::CONFLICT,
            json!({ "error": "State with the same external id and external source already exists", "id": id }),
        ),
        // `StateSerializer.validate`, which runs after the fields were fine
        StateError::TriageGroup => {
            let mut errors = FieldErrors::new();
            errors.insert("non_field_errors".into(), json!(["Cannot create triage state"]));
            V1Error::Fields(errors)
        }
        StateError::Core(other) => other.into(),
    }
}

/// What a body says of a state, as the pin's `StateSerializer` reads it: every field it knows is checked, and what is wrong is one
/// 400. `is_triage` is read and checked and then not kept (it follows the group here), and `sequence` is read on a change; a creation
/// that names one has it replaced by the database.
fn state_patch(b: &Map<String, Value>, partial: bool) -> Result<StatePatch, V1Error> {
    let mut errors = FieldErrors::new();
    for field in ["name", "color"] {
        if !partial && !b.contains_key(field) {
            crate::fields::fail(&mut errors, field, "This field is required.");
        }
    }
    let text = |blank: bool, null: bool| move |v: &Value| char_field(v, Some(255), blank, null);
    let patch = StatePatch {
        name: read(&mut errors, b, "name", text(false, false)).flatten(),
        description: read(&mut errors, b, "description", |v| char_field(v, None, true, false)).flatten(),
        color: read(&mut errors, b, "color", text(false, false)).flatten(),
        sequence: read(&mut errors, b, "sequence", float_field),
        group: read(&mut errors, b, "group", |v| {
            let group = choice_field(v, &GROUPS)?;
            StateGroup::parse(group).ok_or_else(|| format!("\"{group}\" is not a valid choice."))
        }),
        is_default: read(&mut errors, b, "default", bool_field),
        external_source: read(&mut errors, b, "external_source", text(true, true)),
        external_id: read(&mut errors, b, "external_id", text(true, true)),
    };
    // a value `is_triage` that is no boolean is refused, as any field of the pin's serializer is
    let _ = read(&mut errors, b, "is_triage", bool_field);
    refuse_if_any(errors)?;
    Ok(patch)
}

// --- the routes ---------------------------------------------------------------------------------------------------------------

/// `GET .../states/`: the project's states by `sequence`, paged.
async fn list(
    V1Path((slug, project)): V1Path<(String, PlaneUuid)>,
    ctx: Ctx,
    RawQuery(raw): RawQuery,
) -> Result<Json<Value>, V1Error> {
    let scope = ctx.project_scope(&slug, ProjectId(project.0), Permission::ReadProject).await?;
    let q = query_map(raw.as_deref());
    let plan =
        PageRequest::parse(q.get("per_page").map(String::as_str), q.get("cursor").map(String::as_str))?.plan()?;
    let all = thrust_core::visible_states(&ctx.app, &scope).await.map_err(|e| state_error(e, false))?;
    let total = all.len();
    let results = plan.slice(all).iter().map(|s| pick_fields(state_body(s), &q)).collect();
    Ok(Json(plan.envelope(results, total)))
}

/// `POST .../states/`: make a state. The pin answers 200, not 201.
async fn create(
    V1Path((slug, project)): V1Path<(String, PlaneUuid)>,
    ctx: Ctx,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<Value>, V1Error> {
    let scope = ctx.project_scope(&slug, ProjectId(project.0), Permission::WriteState).await?;
    let map = match json_body(&headers, &body)? {
        Body::Object(map) => map,
        Body::Array => return Err(not_a_dictionary()),
    };
    let patch = state_patch(&map, false)?;
    let padded = name_was_padded(&map, patch.name.as_deref());
    match thrust_core::create_state(&ctx.app, &scope, patch).await {
        // the pin looks the state that has the name up by the text as it was sent, which DRF trimmed before the database saw it: a
        // name sent with spaces round it finds nothing, and the view's `state.id` raises
        Err(StateError::NameTaken { .. }) if padded => Err(crate::plane_500()),
        made => Ok(Json(Value::Object(state_body(&made.map_err(|e| state_error(e, false))?)))),
    }
}

/// `GET .../states/{id}/`.
async fn retrieve(
    V1Path((slug, project, state)): V1Path<(String, PlaneUuid, PlaneUuid)>,
    ctx: Ctx,
    RawQuery(raw): RawQuery,
) -> Result<Json<Value>, V1Error> {
    let scope = ctx.project_scope(&slug, ProjectId(project.0), Permission::ReadProject).await?;
    let state = thrust_core::visible_state(&ctx.app, &scope, state.0).await.map_err(|e| state_error(e, false))?;
    Ok(Json(pick_fields(state_body(&state), &query_map(raw.as_deref()))))
}

/// `PATCH .../states/{id}/`: change a state. The base view reads the body before the handler runs (`BaseAPIView.initial`), so a body
/// that cannot be parsed (a media type it has no parser for, JSON that does not parse, a value that is no object or list) is refused
/// before the state is looked for; the handler then finds the state, and only then does the serializer read what the body says, so
/// a list, or a field that is wrong, comes after the 404.
async fn update(
    V1Path((slug, project, state)): V1Path<(String, PlaneUuid, PlaneUuid)>,
    ctx: Ctx,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<Value>, V1Error> {
    let scope = ctx.project_scope(&slug, ProjectId(project.0), Permission::WriteState).await?;
    let parsed = json_body(&headers, &body)?;
    thrust_core::find_state(&ctx.app, &scope, state.0).await.map_err(|e| state_error(e, true))?;
    let map = match parsed {
        Body::Object(map) => map,
        Body::Array => return Err(not_a_dictionary()),
    };
    let patch = state_patch(&map, true)?;
    let state = thrust_core::change_state(&ctx.app, &scope, state.0, patch).await.map_err(|e| state_error(e, true))?;
    Ok(Json(Value::Object(state_body(&state))))
}

/// `DELETE .../states/{id}/`.
async fn destroy(
    V1Path((slug, project, state)): V1Path<(String, PlaneUuid, PlaneUuid)>,
    ctx: Ctx,
) -> Result<StatusCode, V1Error> {
    let scope = ctx.project_scope(&slug, ProjectId(project.0), Permission::WriteState).await?;
    thrust_core::delete_state(&ctx.app, &scope, state.0).await.map_err(|e| state_error(e, true))?;
    Ok(StatusCode::NO_CONTENT)
}
