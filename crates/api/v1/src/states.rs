//! The states of Plane's v1 API: list, create, retrieve, change and delete (`/api/v1/workspaces/{slug}/projects/{id}/states/`).
//!
//! Who may do what is the pin's: a read is for any member of the project, and a write for a project **admin** (`ProjectAdminPermission`:
//! a member of the project is refused, as is a workspace admin who is not in it). A read shows the states that are not the intake
//! one, and none of an archived project. The rules are core's (`thrust_core::states`); this is the pin's wire shape over them: the
//! 18-key body, DRF's field validation, a create that answers 200, and the three different answers to a name that is taken.

use std::ops::RangeInclusive;

use axum::body::Bytes;
use axum::extract::RawQuery;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::get;
use axum::{Json, Router};
use serde_json::{json, Map, Value};
use thrust_contracts::{ProjectId, StateGroup, UserId};
use thrust_core::{Permission, StateError, StatePatch};
use thrust_ports::StateRow;
use unicode_normalization::UnicodeNormalization;

use crate::dto::{into_object, StateDto};
use crate::fields::{
    bool_field, choice_of, float_field, name_was_padded, not_a_dictionary, read_into, refuse_if_any, FieldErrors,
};
use crate::pagination::{Page, PageRequest};
use crate::projects::ts;
use crate::request::{json_body, pick_fields, query_map, Body};
use crate::values::{NullableText, Prose, Text};
use crate::{with_allow, Ctx, PlaneUuid, V1Error, V1Path};

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

/// The scalar values that `unicode-normalization` (Unicode 18.0.0) folds to ASCII and that the pin's Python 3.12 (Unicode 15.0.0)
/// has no character for, so that its `slugify` leaves them as they are and drops them: U+209D to U+209F (which fold to `w`, `y`,
/// `z`), U+A7F1 (`s`) and U+1CCD6 to U+1CCF9 (letters and digits, `a`, `b`, `c` and on). `slugify` drops them too, which is what the
/// pin does today; the pin folds them when it moves to a Python with a newer Unicode, and then this list is emptied.
/// `tests/slugify_parity.rs` holds it to exactly those characters, from the pin's own table of what it has no character for.
pub const NEWER_THAN_THE_PINS_UNICODE: [RangeInclusive<char>; 3] =
    ['\u{209D}'..='\u{209F}', '\u{A7F1}'..='\u{A7F1}', '\u{1CCD6}'..='\u{1CCF9}'];

/// The pin's `slugify` (`django.utils.text`, through `django.template.defaultfilters`), step for step: the name in NFKD (an accent
/// is split from its letter, `é` into `e` and an accent; a compatibility character into what it stands for, `²` into `2`, `ﬃ` into
/// `ffi`, a no-break space into a space), then only what is ASCII, lower case, what is not a word character, a space or a hyphen
/// dropped, runs of hyphens and spaces made one hyphen, and hyphens and underscores trimmed from the ends.
///
/// NFKD is not transliteration: a letter it does not split (`ß`, `Ø`, `æ`) is dropped, as the pin drops it, and a name of CJK or
/// emoji alone has an empty slug there too. `tests/slugify_parity.rs` holds this to the pin's answer for every scalar value.
///
/// The pin's Python (3.12) has Unicode 15.0.0 and this crate has a newer one, so it would fold characters that the pin has no
/// character for and so drops: [`NEWER_THAN_THE_PINS_UNICODE`] are dropped here first, and the slug is the pin's (owner,
/// 2026-11-11). That list is a copy of a version gap; D-031 (2) says what ends it.
pub fn slugify(name: &str) -> String {
    let space = |c: char| matches!(c, ' ' | '\t' | '\n' | '\r' | '\x0b' | '\x0c' | '\x1c'..='\x1f');
    let mut out = String::new();
    let mut in_run = false;
    let known_to_the_pin = name.chars().filter(|c| !NEWER_THAN_THE_PINS_UNICODE.iter().any(|run| run.contains(c)));
    for c in known_to_the_pin.nfkd().filter(char::is_ascii).map(|c| c.to_ascii_lowercase()) {
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

/// The pin's `StateSerializer` (`StateDto`): 18 keys. `is_triage` is whether the group is the intake's, which a state of this list
/// never is; the pin stores it apart from the group, and a request can set it on a state of another group (the register carries that).
fn state_body(s: &StateRow) -> Result<Map<String, Value>, V1Error> {
    let user = |u: Option<UserId>| u.map(|u| u.0);
    into_object(&StateDto {
        id: s.id,
        created_at: ts(s.created_at),
        updated_at: ts(s.updated_at),
        deleted_at: None,
        name: s.name.clone(),
        description: s.description.clone(),
        color: s.color.clone(),
        slug: slugify(&s.name),
        sequence: s.sequence,
        group: s.group.as_str(),
        is_triage: s.group == StateGroup::Triage,
        default: s.is_default,
        external_source: s.external_source.clone(),
        external_id: s.external_id.clone(),
        created_by: user(s.created_by),
        updated_by: user(s.updated_by),
        project: s.project_id.0,
        workspace: s.workspace_id.0,
    })
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

/// A state's group, read as DRF's choice field over the groups `StateGroup` has: the list is `StateGroup::ALL`, for the parser and for
/// the document alike.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Group(StateGroup);

impl Group {
    fn read(v: &Value) -> Result<Self, String> {
        choice_of(v, StateGroup::parse).map(Self)
    }
}

#[cfg(feature = "openapi")]
impl utoipa::PartialSchema for Group {
    fn schema() -> utoipa::openapi::RefOr<utoipa::openapi::Schema> {
        utoipa::openapi::RefOr::T(utoipa::openapi::Schema::Object(crate::openapi::state_group()))
    }
}

#[cfg(feature = "openapi")]
impl utoipa::ToSchema for Group {
    fn name() -> std::borrow::Cow<'static, str> {
        std::borrow::Cow::Borrowed("StateGroupChoice")
    }
}

/// What a body says of a state, as the pin's `StateSerializer` reads it: every field it knows is read into this, and what is wrong is
/// one 400. The pin names a body whose keys may all be left out `Patched…`, and this is that: a change reads into it as it is, and a
/// creation reads into it and must also name [`PatchedStateRequest::REQUIRED`]. A key left out is `None`. Each field's type says what
/// it takes, and the document is written from these types (the schema `PatchedStateRequest` is this struct, and `Creations` in
/// `openapi.rs` makes `StateRequest` of it with `REQUIRED`), so the bounds below are the only place they are written. Only the two
/// external keys take a `null`.
#[derive(Debug, Default)]
#[cfg_attr(
    feature = "openapi",
    derive(utoipa::ToSchema),
    schema(description = "What a state's change takes: the creation's keys, none required.")
)]
pub(crate) struct PatchedStateRequest {
    #[cfg_attr(feature = "openapi", schema(inline, nullable = false))]
    name: Option<Text<1, 255>>,
    #[cfg_attr(feature = "openapi", schema(inline, nullable = false))]
    description: Option<Prose>,
    #[cfg_attr(feature = "openapi", schema(inline, nullable = false))]
    color: Option<Text<1, 255>>,
    #[cfg_attr(feature = "openapi", schema(inline, nullable = false))]
    sequence: Option<f64>,
    #[cfg_attr(feature = "openapi", schema(inline, nullable = false))]
    group: Option<Group>,
    #[cfg_attr(feature = "openapi", schema(inline, nullable = false))]
    default: Option<bool>,
    #[cfg_attr(feature = "openapi", schema(inline, nullable = false))]
    external_source: Option<NullableText<0, 255>>,
    #[cfg_attr(feature = "openapi", schema(inline, nullable = false))]
    external_id: Option<NullableText<0, 255>>,
    // read and checked and then not kept: it follows the group here (the register carries the difference)
    #[cfg_attr(feature = "openapi", schema(inline, nullable = false))]
    #[allow(
        dead_code,
        reason = "read, checked, and not kept: it is the document's key and the pin's, and here it follows the group"
    )]
    is_triage: Option<bool>,
}

impl PatchedStateRequest {
    /// The keys a creation must name; a change names none of them. The document's creation is written from this list, so it is the
    /// parser's.
    pub(crate) const REQUIRED: [&'static str; 2] = ["name", "color"];

    /// Read a body: every key it names is checked, and what is wrong is one 400. `sequence` is read on a change; a creation that names
    /// one has it replaced by the database.
    fn read(b: &Map<String, Value>, partial: bool) -> Result<Self, V1Error> {
        let mut errors = FieldErrors::new();
        for field in Self::REQUIRED {
            if !partial && !b.contains_key(field) {
                crate::fields::fail(&mut errors, field, "This field is required.");
            }
        }
        let request = read_into!(PatchedStateRequest, errors, b, {
            name: Text::read,
            description: Prose::read,
            color: Text::read,
            sequence: float_field,
            group: Group::read,
            default: bool_field,
            external_source: NullableText::read,
            external_id: NullableText::read,
            is_triage: bool_field,
        });
        refuse_if_any(errors)?;
        Ok(request)
    }
}

impl From<PatchedStateRequest> for StatePatch {
    fn from(r: PatchedStateRequest) -> Self {
        StatePatch {
            name: r.name.map(Text::into_string),
            description: r.description.map(Prose::into_string),
            color: r.color.map(Text::into_string),
            sequence: r.sequence,
            group: r.group.map(|g| g.0),
            is_default: r.default,
            external_source: r.external_source.map(NullableText::into_option),
            external_id: r.external_id.map(NullableText::into_option),
        }
    }
}

/// The creation's body in the document: `PatchedStateRequest` with `name` and `color` required, which `Creations` (`openapi.rs`) writes from
/// the change's schema. The type is the name the path declarations use for it.
#[cfg(feature = "openapi")]
#[derive(utoipa::ToSchema)]
#[allow(dead_code, reason = "a name for the document: nothing builds it")]
pub(crate) struct StateRequest;

// --- the routes ---------------------------------------------------------------------------------------------------------------

/// `GET .../states/`: the project's states by `sequence`, paged.
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        get,
        path = "/api/v1/workspaces/{slug}/projects/{project_id}/states/",
        operation_id = "list_states",
        tag = "States",
        summary = "List states",
        params(("slug" = String, Path), ("project_id" = thrust_contracts::Uuid, Path), crate::openapi::StatesQuery),
        responses(
            crate::openapi::Refused,
            (status = 200, description = "Paginated list of states", body = Page<StateDto>),
            (status = 404, description = "The requested resource was not found."),
        ),
        security(("ApiKeyAuthentication" = []))
    )
)]
pub(crate) async fn list(
    V1Path((slug, project)): V1Path<(String, PlaneUuid)>,
    ctx: Ctx,
    RawQuery(raw): RawQuery,
) -> Result<Json<Page<Value>>, V1Error> {
    let scope = ctx.project_scope(&slug, ProjectId(project.0), Permission::ReadProject).await?;
    let q = query_map(raw.as_deref());
    let plan =
        PageRequest::parse(q.get("per_page").map(String::as_str), q.get("cursor").map(String::as_str))?.plan()?;
    let all = thrust_core::visible_states(&ctx.app, &scope).await.map_err(|e| state_error(e, false))?;
    let total = all.len();
    let results =
        plan.slice(all).iter().map(|s| Ok(pick_fields(state_body(s)?, &q))).collect::<Result<Vec<_>, V1Error>>()?;
    Ok(Json(plan.page(results, total)))
}

/// `POST .../states/`: make a state. The pin answers 200, not 201.
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        post,
        path = "/api/v1/workspaces/{slug}/projects/{project_id}/states/",
        operation_id = "create_state",
        tag = "States",
        summary = "Create state",
        params(("slug" = String, Path), ("project_id" = thrust_contracts::Uuid, Path)),
        request_body(content = StateRequest, content_type = "application/json"),
        responses(
            crate::openapi::Refused,
            (status = 200, description = "State created", body = StateDto),
            (status = 400, description = "Invalid request data provided"),
            (status = 404, description = "The requested resource was not found."),
            (status = 409, description = "State with the same name already exists"),
        ),
        security(("ApiKeyAuthentication" = []))
    )
)]
pub(crate) async fn create(
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
    let patch: StatePatch = PatchedStateRequest::read(&map, false)?.into();
    let padded = name_was_padded(&map, patch.name.as_deref());
    match thrust_core::create_state(&ctx.app, &scope, patch).await {
        // the pin looks the state that has the name up by the text as it was sent, which DRF trimmed before the database saw it: a
        // name sent with spaces round it finds nothing, and the view's `state.id` raises
        Err(StateError::NameTaken { .. }) if padded => Err(V1Error::internal(
            "a padded state name clashes: the pin looks it up untrimmed, finds nothing, and `state.id` raises",
        )),
        made => Ok(Json(Value::Object(state_body(&made.map_err(|e| state_error(e, false))?)?))),
    }
}

/// `GET .../states/{id}/`.
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        get,
        path = "/api/v1/workspaces/{slug}/projects/{project_id}/states/{state_id}/",
        operation_id = "retrieve_state",
        tag = "States",
        summary = "Retrieve state",
        params(
            ("slug" = String, Path),
            ("project_id" = thrust_contracts::Uuid, Path),
            ("state_id" = thrust_contracts::Uuid, Path)
        ),
        responses(
            crate::openapi::Refused,
            (status = 200, description = "State retrieved", body = StateDto),
            (status = 404, description = "The requested resource was not found."),
        ),
        security(("ApiKeyAuthentication" = []))
    )
)]
pub(crate) async fn retrieve(
    V1Path((slug, project, state)): V1Path<(String, PlaneUuid, PlaneUuid)>,
    ctx: Ctx,
    RawQuery(raw): RawQuery,
) -> Result<Json<Value>, V1Error> {
    let scope = ctx.project_scope(&slug, ProjectId(project.0), Permission::ReadProject).await?;
    let state = thrust_core::visible_state(&ctx.app, &scope, state.0).await.map_err(|e| state_error(e, false))?;
    Ok(Json(pick_fields(state_body(&state)?, &query_map(raw.as_deref()))))
}

/// `PATCH .../states/{id}/`: change a state. The base view reads the body before the handler runs (`BaseAPIView.initial`), so a body
/// that cannot be parsed (a media type it has no parser for, JSON that does not parse, a value that is no object or list) is refused
/// before the state is looked for; the handler then finds the state, and only then does the serializer read what the body says, so
/// a list, or a field that is wrong, comes after the 404.
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        patch,
        path = "/api/v1/workspaces/{slug}/projects/{project_id}/states/{state_id}/",
        operation_id = "update_state",
        tag = "States",
        summary = "Update state",
        params(
            ("slug" = String, Path),
            ("project_id" = thrust_contracts::Uuid, Path),
            ("state_id" = thrust_contracts::Uuid, Path)
        ),
        // not required: an empty body is an empty object (`json_body`), as DRF reads it
        request_body(content = Option<PatchedStateRequest>, content_type = "application/json"),
        responses(
            crate::openapi::Refused,
            (status = 200, description = "State updated", body = StateDto),
            (status = 400, description = "Invalid request data provided"),
            (status = 404, description = "The requested resource was not found."),
            (status = 409, description = "Resource with same external ID already exists"),
        ),
        security(("ApiKeyAuthentication" = []))
    )
)]
pub(crate) async fn update(
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
    let patch: StatePatch = PatchedStateRequest::read(&map, true)?.into();
    let state = thrust_core::change_state(&ctx.app, &scope, state.0, patch).await.map_err(|e| state_error(e, true))?;
    Ok(Json(Value::Object(state_body(&state)?)))
}

/// `DELETE .../states/{id}/`.
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        delete,
        path = "/api/v1/workspaces/{slug}/projects/{project_id}/states/{state_id}/",
        operation_id = "delete_state",
        tag = "States",
        summary = "Delete state",
        params(
            ("slug" = String, Path),
            ("project_id" = thrust_contracts::Uuid, Path),
            ("state_id" = thrust_contracts::Uuid, Path)
        ),
        responses(
            crate::openapi::Refused,
            (status = 204, description = "Resource deleted successfully"),
            (status = 400, description = "State cannot be deleted"),
            (status = 404, description = "The requested resource was not found."),
        ),
        security(("ApiKeyAuthentication" = []))
    )
)]
pub(crate) async fn destroy(
    V1Path((slug, project, state)): V1Path<(String, PlaneUuid, PlaneUuid)>,
    ctx: Ctx,
) -> Result<StatusCode, V1Error> {
    let scope = ctx.project_scope(&slug, ProjectId(project.0), Permission::WriteState).await?;
    thrust_core::delete_state(&ctx.app, &scope, state.0).await.map_err(|e| state_error(e, true))?;
    Ok(StatusCode::NO_CONTENT)
}
