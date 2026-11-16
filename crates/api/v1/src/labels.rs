//! The labels of Plane's v1 API: list, create, retrieve, change and delete (`/api/v1/workspaces/{slug}/projects/{id}/labels/`).
//!
//! Who may do what is the pin's: a read is for any member of the project, and a write for a project **admin or member**
//! (`ProjectMemberPermission`: a guest is refused, as is a workspace admin who is not in the project). The rules are core's
//! (`thrust_core::labels`); this is the pin's wire shape over them: the 15-key body, DRF's field validation in the order of the
//! serializer's fields, a create that answers 201, and the three different answers to a name that is taken.
//!
//! Where the label views differ from the state views, and this follows the label ones: the queryset of a read, a change and a deletion
//! is the same (live labels, none of an archived project), the detail read ignores `fields`, and a list is ordered by `-created_at`
//! with no `order_by` of its own.

use axum::body::Bytes;
use axum::extract::RawQuery;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::get;
use axum::{Json, Router};
use serde_json::{json, Map, Value};
use thrust_contracts::{ProjectId, UserId};
use thrust_core::{LabelError, LabelPatch, Permission};
use thrust_ports::LabelRow;

use crate::dto::{into_object, LabelDto};
use crate::fields::{
    fail, float_field, name_was_padded, no_such_pk, no_such_pk_shown, not_a_dictionary, read_into, refuse_if_any,
    FieldErrors,
};
use crate::pagination::{Page, PageRequest};
use crate::projects::ts;
use crate::request::{json_body, pick_fields, query_map, Body};
use crate::values::{NullableKey, NullableText, Prose, Text};
use crate::{with_allow, Ctx, PlaneUuid, V1Error, V1Path};

pub(crate) fn routes() -> Router {
    Router::new()
        .route(
            "/api/v1/workspaces/{slug}/projects/{project_id}/labels/",
            with_allow("GET, POST", get(list).post(create)),
        )
        .route(
            "/api/v1/workspaces/{slug}/projects/{project_id}/labels/{label_id}/",
            with_allow("GET, PATCH, DELETE", get(retrieve).patch(update).delete(destroy)),
        )
}

// --- the wire shape -----------------------------------------------------------------------------------------------------------

/// The pin's `LabelSerializer` (`fields = "__all__"`), as [`LabelDto`]: 15 keys. A work item read with `expand=labels` holds these.
pub(crate) fn label_dto(l: &LabelRow) -> LabelDto {
    let user = |u: Option<UserId>| u.map(|u| u.0);
    LabelDto {
        id: l.id,
        created_at: ts(l.created_at),
        updated_at: ts(l.updated_at),
        deleted_at: None,
        created_by: user(l.created_by),
        updated_by: user(l.updated_by),
        workspace: l.workspace_id.0,
        project: l.project_id.0,
        parent: l.parent_id,
        name: l.name.clone(),
        description: l.description.clone(),
        color: l.color.clone(),
        sort_order: l.sort_order,
        external_source: l.external_source.clone(),
        external_id: l.external_id.clone(),
    }
}

/// [`label_dto`] as the JSON object the label routes then cut to the fields asked for.
pub(crate) fn label_body(l: &LabelRow) -> Result<Map<String, Value>, V1Error> {
    into_object(&label_dto(l))
}

/// What the pin answers for a refusal. A name that is taken is a 409 naming the label when a label is made, and the base view's 400
/// "The payload is not valid" when one is changed (the database's integrity error reaches the base view there).
fn label_error(e: LabelError, changing: bool) -> V1Error {
    match e {
        LabelError::NotFound => V1Error::NotFound,
        LabelError::NameTaken { .. } if changing => V1Error::error(StatusCode::BAD_REQUEST, "The payload is not valid"),
        LabelError::NameTaken { id: Some(id) } => V1Error::Error(
            StatusCode::CONFLICT,
            json!({ "error": "Label with the same name already exists in the project", "id": id }),
        ),
        // the clash was with a label that is no longer there to name: the pin's view fails on the missing row
        LabelError::NameTaken { id: None } => {
            V1Error::internal("a label's name is taken by a row that is gone: the pin's view fails on the missing row")
        }
        LabelError::ExternalTaken { id } => V1Error::Error(
            StatusCode::CONFLICT,
            json!({ "error": "Label with the same external id and external source already exists", "id": id }),
        ),
        LabelError::NoSuchParent(parent) => {
            let mut errors = FieldErrors::new();
            fail(&mut errors, "parent", no_such_pk(parent));
            V1Error::Fields(errors)
        }
        LabelError::Core(other) => other.into(),
    }
}

// --- reading a body -----------------------------------------------------------------------------------------------------------

/// What a body says of a label, as the pin's `LabelCreateUpdateSerializer` reads it, field by field in the order the serializer lists
/// them (name, color, description, external_source, external_id, parent, sort_order). The pin names a body whose keys may all be left
/// out `Patched…`, and this is that: a change reads into it as it is, and a creation reads into it and must also name
/// [`PatchedLabelCreateUpdateRequest::REQUIRED`]. A key left out is `None`. Each field's type says what it takes, and the document is
/// written from these types (the schema `PatchedLabelCreateUpdateRequest` is this struct, and `Creations` in `openapi.rs` makes
/// `LabelCreateUpdateRequest` of it with `REQUIRED`), so the bounds below are the only place they are written. `color` and
/// `description` can be blank and are never `null`; the two external keys and `parent` can be `null`.
#[derive(Debug, Default)]
#[cfg_attr(
    feature = "openapi",
    derive(utoipa::ToSchema),
    schema(description = "What a label's change takes: the creation's keys, none required.")
)]
pub(crate) struct PatchedLabelCreateUpdateRequest {
    #[cfg_attr(feature = "openapi", schema(inline, nullable = false))]
    name: Option<Text<1, 255>>,
    #[cfg_attr(feature = "openapi", schema(inline, nullable = false))]
    color: Option<Text<0, 255>>,
    #[cfg_attr(feature = "openapi", schema(inline, nullable = false))]
    description: Option<Prose>,
    #[cfg_attr(feature = "openapi", schema(inline, nullable = false))]
    external_source: Option<NullableText<0, 255>>,
    #[cfg_attr(feature = "openapi", schema(inline, nullable = false))]
    external_id: Option<NullableText<0, 255>>,
    #[cfg_attr(feature = "openapi", schema(inline, nullable = false))]
    parent: Option<NullableKey>,
    #[cfg_attr(feature = "openapi", schema(inline, nullable = false))]
    sort_order: Option<f64>,
}

impl PatchedLabelCreateUpdateRequest {
    /// The keys a creation must name; a change names none of them. The document's creation is written from this list, so it is the
    /// parser's.
    pub(crate) const REQUIRED: [&'static str; 1] = ["name"];

    /// The first stage: read every key the body names, with no database, and file what is wrong. The errors are handed back and not
    /// refused, because the second stage ([`Self::check_parent`]) adds its own to the same 400, as the pin reports every field at once.
    fn read(b: &Map<String, Value>, partial: bool) -> (Self, FieldErrors) {
        let mut errors = FieldErrors::new();
        for field in Self::REQUIRED {
            if !partial && !b.contains_key(field) {
                fail(&mut errors, field, "This field is required.");
            }
        }
        let request = read_into!(PatchedLabelCreateUpdateRequest, errors, b, {
            name: Text::read,
            color: Text::read,
            description: Prose::read,
            external_source: NullableText::read,
            external_id: NullableText::read,
            parent: NullableKey::read,
            sort_order: float_field,
        });
        (request, errors)
    }

    /// The second stage: a `parent` that names a label must name a live one of this project (the pin's field takes any label of any
    /// project; this takes one of this project, D-032).
    async fn check_parent(
        &self,
        ctx: &Ctx,
        scope: &thrust_core::Scope,
        errors: &mut FieldErrors,
    ) -> Result<(), V1Error> {
        let Some(key) = self.parent.as_ref().and_then(NullableKey::key) else { return Ok(()) };
        if !thrust_core::is_label_of_project(&ctx.app, scope, key.id).await.map_err(|e| label_error(e, false))? {
            fail(errors, "parent", no_such_pk_shown(&key.shown));
        }
        Ok(())
    }
}

impl From<PatchedLabelCreateUpdateRequest> for LabelPatch {
    fn from(r: PatchedLabelCreateUpdateRequest) -> Self {
        LabelPatch {
            name: r.name.map(Text::into_string),
            description: r.description.map(Prose::into_string),
            color: r.color.map(Text::into_string),
            sort_order: r.sort_order,
            parent: r.parent.map(NullableKey::into_id),
            external_source: r.external_source.map(NullableText::into_option),
            external_id: r.external_id.map(NullableText::into_option),
        }
    }
}

/// The creation's body in the document: `PatchedLabelCreateUpdateRequest` with `name` required, which `Creations` (`openapi.rs`) writes
/// from the change's schema. The type is the name the path declarations use for it.
#[cfg(feature = "openapi")]
#[derive(utoipa::ToSchema)]
#[allow(dead_code, reason = "a name for the document: nothing builds it")]
pub(crate) struct LabelCreateUpdateRequest;

/// What a body says of a label, read and checked: every field first, then the database's part, and what is wrong in either is one 400.
async fn label_patch(
    ctx: &Ctx,
    scope: &thrust_core::Scope,
    b: &Map<String, Value>,
    partial: bool,
) -> Result<LabelPatch, V1Error> {
    let (request, mut errors) = PatchedLabelCreateUpdateRequest::read(b, partial);
    request.check_parent(ctx, scope, &mut errors).await?;
    refuse_if_any(errors)?;
    Ok(request.into())
}

// --- the routes ---------------------------------------------------------------------------------------------------------------

/// `GET .../labels/`: the project's labels, newest first, paged. There is no `order_by` here (the pin's view orders by the URL's
/// keyword argument, which no route has), and `expand` is ignored (D-032). The pin's schema lists both as parameters, so the
/// document does (`ListQuery`): a request that sends them is answered as one that does not.
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        get,
        path = "/api/v1/workspaces/{slug}/projects/{project_id}/labels/",
        operation_id = "list_labels",
        tag = "Labels",
        params(("slug" = String, Path), ("project_id" = thrust_contracts::Uuid, Path), crate::openapi::ListQuery),
        responses(
            crate::openapi::Refused,
            (status = 200, description = "Paginated list of labels", body = Page<LabelDto>),
            (status = 400, description = "Invalid request data provided"),
            (status = 404, description = "Project not found"),
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
    let all = thrust_core::visible_labels(&ctx.app, &scope).await.map_err(|e| label_error(e, false))?;
    let total = all.len();
    let results =
        plan.slice(all).iter().map(|l| Ok(pick_fields(label_body(l)?, &q))).collect::<Result<Vec<_>, V1Error>>()?;
    Ok(Json(plan.page(results, total)))
}

/// `POST .../labels/`: make a label. The pin answers 201.
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        post,
        path = "/api/v1/workspaces/{slug}/projects/{project_id}/labels/",
        operation_id = "create_label",
        tag = "Labels",
        params(("slug" = String, Path), ("project_id" = thrust_contracts::Uuid, Path)),
        request_body(content = LabelCreateUpdateRequest, content_type = "application/json"),
        responses(
            crate::openapi::Refused,
            (status = 201, description = "Label created successfully", body = LabelDto),
            (status = 400, description = "Invalid request data provided"),
            (status = 404, description = "The requested resource was not found."),
            (status = 409, description = "Label with the same name already exists"),
        ),
        security(("ApiKeyAuthentication" = []))
    )
)]
pub(crate) async fn create(
    V1Path((slug, project)): V1Path<(String, PlaneUuid)>,
    ctx: Ctx,
    headers: HeaderMap,
    body: Bytes,
) -> Result<(StatusCode, Json<Value>), V1Error> {
    let scope = ctx.project_scope(&slug, ProjectId(project.0), Permission::WriteLabel).await?;
    let map = match json_body(&headers, &body)? {
        Body::Object(map) => map,
        Body::Array => return Err(not_a_dictionary()),
    };
    let patch = label_patch(&ctx, &scope, &map, false).await?;
    let padded = name_was_padded(&map, patch.name.as_deref());
    match thrust_core::create_label(&ctx.app, &scope, patch).await {
        // the pin looks the label that has the name up by the text as it was sent, which DRF trimmed before the database saw it: a
        // name sent with spaces round it finds nothing, and the view's `label.id` raises
        Err(LabelError::NameTaken { .. }) if padded => Err(V1Error::internal(
            "a padded label name clashes: the pin looks it up untrimmed, finds nothing, and `label.id` raises",
        )),
        made => {
            let label = made.map_err(|e| label_error(e, false))?;
            Ok((StatusCode::CREATED, Json(Value::Object(label_body(&label)?))))
        }
    }
}

/// `GET .../labels/{id}/`. The pin's detail read builds its serializer with neither `fields` nor `expand`: both are ignored, so the
/// document has no query for it.
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        get,
        path = "/api/v1/workspaces/{slug}/projects/{project_id}/labels/{pk}/",
        operation_id = "get_labels",
        tag = "Labels",
        params(
            ("slug" = String, Path),
            ("project_id" = thrust_contracts::Uuid, Path),
            ("pk" = thrust_contracts::Uuid, Path)
        ),
        responses(
            crate::openapi::Refused,
            (status = 200, description = "Labels", body = LabelDto),
            (status = 404, description = "Label not found"),
        ),
        security(("ApiKeyAuthentication" = []))
    )
)]
pub(crate) async fn retrieve(
    V1Path((slug, project, label)): V1Path<(String, PlaneUuid, PlaneUuid)>,
    ctx: Ctx,
) -> Result<Json<Value>, V1Error> {
    let scope = ctx.project_scope(&slug, ProjectId(project.0), Permission::ReadProject).await?;
    let label = thrust_core::visible_label(&ctx.app, &scope, label.0).await.map_err(|e| label_error(e, false))?;
    Ok(Json(Value::Object(label_body(&label)?)))
}

/// `PATCH .../labels/{id}/`: change a label. The base view reads the body before the handler runs, so a body that cannot be parsed
/// is refused before the label is looked for; the handler then finds the label (an archived project has none to find), and only then
/// does the serializer read what the body says, so a list, or a field that is wrong, comes after the 404.
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        patch,
        path = "/api/v1/workspaces/{slug}/projects/{project_id}/labels/{pk}/",
        operation_id = "update_label",
        tag = "Labels",
        params(
            ("slug" = String, Path),
            ("project_id" = thrust_contracts::Uuid, Path),
            ("pk" = thrust_contracts::Uuid, Path)
        ),
        // not required: an empty body is an empty object (`json_body`), as DRF reads it
        request_body(content = Option<PatchedLabelCreateUpdateRequest>, content_type = "application/json"),
        responses(
            crate::openapi::Refused,
            (status = 200, description = "Label updated successfully", body = LabelDto),
            (status = 400, description = "Invalid request data provided"),
            (status = 404, description = "Label not found"),
            (status = 409, description = "Resource with same external ID already exists"),
        ),
        security(("ApiKeyAuthentication" = []))
    )
)]
pub(crate) async fn update(
    V1Path((slug, project, label)): V1Path<(String, PlaneUuid, PlaneUuid)>,
    ctx: Ctx,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<Value>, V1Error> {
    let scope = ctx.project_scope(&slug, ProjectId(project.0), Permission::WriteLabel).await?;
    let parsed = json_body(&headers, &body)?;
    thrust_core::visible_label(&ctx.app, &scope, label.0).await.map_err(|e| label_error(e, true))?;
    let map = match parsed {
        Body::Object(map) => map,
        Body::Array => return Err(not_a_dictionary()),
    };
    let patch = label_patch(&ctx, &scope, &map, true).await?;
    let label = thrust_core::change_label(&ctx.app, &scope, label.0, patch).await.map_err(|e| label_error(e, true))?;
    Ok(Json(Value::Object(label_body(&label)?)))
}

/// `DELETE .../labels/{id}/`.
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        delete,
        path = "/api/v1/workspaces/{slug}/projects/{project_id}/labels/{pk}/",
        operation_id = "delete_label",
        tag = "Labels",
        params(
            ("slug" = String, Path),
            ("project_id" = thrust_contracts::Uuid, Path),
            ("pk" = thrust_contracts::Uuid, Path)
        ),
        responses(
            crate::openapi::Refused,
            (status = 204, description = "Resource deleted successfully"),
            (status = 404, description = "Label not found"),
        ),
        security(("ApiKeyAuthentication" = []))
    )
)]
pub(crate) async fn destroy(
    V1Path((slug, project, label)): V1Path<(String, PlaneUuid, PlaneUuid)>,
    ctx: Ctx,
) -> Result<StatusCode, V1Error> {
    let scope = ctx.project_scope(&slug, ProjectId(project.0), Permission::WriteLabel).await?;
    thrust_core::delete_label(&ctx.app, &scope, label.0).await.map_err(|e| label_error(e, true))?;
    Ok(StatusCode::NO_CONTENT)
}
