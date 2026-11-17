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

use crate::dto::{into_object, ProjectDto, ProjectLiteDto};
use crate::fields::{fail, no_such_pk, refuse_if_any, request, FieldErrors};
use crate::pagination::{Page, PageRequest};
use crate::request::{json_body, pick_fields, query_map, Body};
use crate::values::{AnyJson, NullableProse, NullableText, NullableUuid, Prose, Text, Timezone, Whole};
use crate::{with_allow, Ctx, PlaneUuid, V1Error, V1Path};

/// The orders a project list takes (`PROJECT_ORDER_BY_ALLOWLIST`), each with an optional `-`; anything else is the default.
const ORDER_BY: [&str; 5] = ["created_at", "updated_at", "name", "network", "sort_order"];

pub(crate) fn routes() -> Router {
    Router::new()
        .route("/api/v1/workspaces/{slug}/projects/", with_allow("GET, POST", get(list).post(create)))
        .route("/api/v1/workspaces/{slug}/projects-lite/", with_allow("GET", get(list_lite)))
        .route(
            "/api/v1/workspaces/{slug}/projects/{project_id}/",
            with_allow("GET, PATCH, DELETE", get(retrieve).patch(update).delete(destroy)),
        )
        .route(
            "/api/v1/workspaces/{slug}/projects/{project_id}/archive/",
            with_allow("POST, DELETE", post(archive).delete(unarchive)),
        )
}

// --- the wire shape -----------------------------------------------------------------------------------------------------------

/// A DRF datetime: UTC, microseconds, `Z`.
pub(crate) fn ts(t: Timestamp) -> String {
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

/// The pin's `ProjectSerializer` (`ProjectDto`): 43 keys, and `sort_order` besides when the caller's place in the list is part of the
/// answer. The pin counts cycles and modules; Thrust has neither, so both are 0 (a deviation the register carries), and no deploy
/// board exists.
fn project_body(l: &ProjectListing, total_members: i64, with_sort_order: bool) -> Result<Map<String, Value>, V1Error> {
    let p = &l.project;
    let user = |u: Option<UserId>| u.map(|u| u.0);
    into_object(&ProjectDto {
        id: p.id.0,
        workspace: p.workspace_id.0,
        name: p.name.clone(),
        identifier: p.identifier.clone(),
        description: p.description.clone(),
        description_text: p.description_text.clone(),
        description_html: p.description_html.clone(),
        network: network_number(p.network),
        emoji: p.emoji.clone(),
        icon_prop: p.icon_prop.clone(),
        logo_props: p.logo_props.clone(),
        cover_image: p.cover_image.clone(),
        // the cover image's asset, once there are assets, is its address; until then it is the text the project holds
        cover_image_url: p.cover_image.clone(),
        cover_image_asset: p.cover_image_asset,
        default_assignee: user(p.default_assignee),
        project_lead: user(p.project_lead),
        module_view: p.module_view,
        cycle_view: p.cycle_view,
        issue_views_view: p.issue_views_view,
        page_view: p.page_view,
        intake_view: p.intake_view,
        is_time_tracking_enabled: p.is_time_tracking_enabled,
        is_issue_type_enabled: p.is_issue_type_enabled,
        guest_view_all_features: p.guest_view_all_features,
        estimate: p.estimate,
        archive_in: p.archive_in,
        close_in: p.close_in,
        default_state: p.default_state,
        archived_at: p.archived_at.map(ts),
        timezone: p.timezone.clone(),
        external_source: p.external_source.clone(),
        external_id: p.external_id.clone(),
        created_at: ts(p.created_at),
        updated_at: ts(p.updated_at),
        created_by: user(p.created_by),
        updated_by: user(p.updated_by),
        deleted_at: None,
        is_member: l.role.is_some(),
        member_role: l.role.map(role_number),
        total_members,
        total_cycles: 0,
        total_modules: 0,
        is_deployed: false,
        sort_order: with_sort_order.then_some(l.member_sort_order),
    })
}

/// The pin's `ProjectLiteSerializer` (`ProjectLiteDto`): nine keys.
fn lite_body(p: &ProjectRow) -> Result<Value, V1Error> {
    into_object(&ProjectLiteDto {
        id: p.id.0,
        identifier: p.identifier.clone(),
        name: p.name.clone(),
        cover_image: p.cover_image.clone(),
        icon_prop: p.icon_prop.clone(),
        emoji: p.emoji.clone(),
        description: p.description.clone(),
        cover_image_url: p.cover_image.clone(),
        archived_at: p.archived_at.map(ts),
    })
    .map(Value::Object)
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
            _ => place(a.member_sort_order, b.member_sort_order),
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

request! {
    #[schema(description = "What a project's change takes (the pin's `ProjectUpdateSerializer`): the creation's keys, none required, and the two ids that a\nchange can set.")]
    /// What a body says of a project, as the pin's `ProjectCreateSerializer` (and, for a change, `ProjectUpdateSerializer`) reads it. The
    /// pin names a body whose keys may all be left out `Patched…`, and this is that: a change reads into it as it is, and a creation reads
    /// into it, must also name [`PatchedProjectUpdateRequest::REQUIRED`], and does not read [`PatchedProjectUpdateRequest::CHANGE_ONLY`].
    /// A key left out is `None`. Each field's type says what it takes, and the document is written from these types (the schema
    /// `PatchedProjectUpdateRequest` is this struct, and `Creations` in `openapi.rs` makes `ProjectCreateRequest` of it with `REQUIRED`
    /// and without `CHANGE_ONLY`), so the bounds below are the only place they are written; the two lengths are core's own constants. A
    /// field it does not know (a `network`, an `id`) is ignored.
    pub(crate) struct PatchedProjectUpdateRequest {
        name: Text<1, { thrust_core::MAX_PROJECT_NAME }>,
        identifier: Text<1, { thrust_core::MAX_PROJECT_IDENTIFIER }>,
        description: Prose,
        emoji: NullableText<0, 255>,
        cover_image: NullableProse,
        external_source: NullableText<0, 255>,
        external_id: NullableText<0, 255>,
        icon_prop: AnyJson,
        module_view: bool,
        cycle_view: bool,
        issue_views_view: bool,
        page_view: bool,
        intake_view: bool,
        guest_view_all_features: bool,
        is_issue_type_enabled: bool,
        is_time_tracking_enabled: bool,
        archive_in: Whole<0, { thrust_core::MAX_PROJECT_MONTHS }>,
        close_in: Whole<0, { thrust_core::MAX_PROJECT_MONTHS }>,
        timezone: Timezone,
        project_lead: NullableUuid,
        default_assignee: NullableUuid,
        // the two that only a change reads (`CHANGE_ONLY`)
        default_state: NullableUuid,
        estimate: NullableUuid,
    }
}

impl PatchedProjectUpdateRequest {
    /// The keys a creation must name; a change names none of them. The document's creation is written from this list, so it is the
    /// parser's.
    pub(crate) const REQUIRED: [&'static str; 2] = ["name", "identifier"];

    /// The keys that only a change reads (the pin's `ProjectUpdateSerializer` has them and `ProjectCreateSerializer` has not): a
    /// creation that names one is read as if it did not. The document's creation is written without them, from this list.
    pub(crate) const CHANGE_ONLY: [&'static str; 2] = ["default_state", "estimate"];

    /// The first stage: read every key the body names, with no database, and file what is wrong. The errors are handed back and not
    /// refused, because the second stage ([`Self::check_rows`]) adds its own to the same 400, as the pin reports every field at once.
    fn read(b: &Map<String, Value>, partial: bool) -> (Self, FieldErrors) {
        let mut errors = FieldErrors::new();
        for field in Self::REQUIRED {
            if !partial && !b.contains_key(field) {
                fail(&mut errors, field, "This field is required.");
            }
        }
        // a creation does not read what only a change has
        let creation_body: Map<String, Value>;
        let b = if partial {
            b
        } else {
            creation_body = b
                .iter()
                .filter(|(k, _)| !Self::CHANGE_ONLY.contains(&k.as_str()))
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            &creation_body
        };
        let request = PatchedProjectUpdateRequest::read_fields(&mut errors, b);
        (request, errors)
    }

    /// The second stage: the fields that name a row. A person must be a user that is there (the rows of the database); there are no
    /// estimates here, so no key names one, and `null` names none.
    async fn check_rows(&self, ctx: &Ctx, errors: &mut FieldErrors) -> Result<(), V1Error> {
        for (field, person) in [("project_lead", self.project_lead), ("default_assignee", self.default_assignee)] {
            let Some(id) = person.and_then(NullableUuid::id) else { continue };
            if ctx.app.ports.db.user_by_id(UserId(id)).await?.is_none() {
                fail(errors, field, no_such_pk(id));
            }
        }
        if let Some(id) = self.estimate.and_then(NullableUuid::id) {
            fail(errors, "estimate", no_such_pk(id));
        }
        Ok(())
    }

    /// The third stage, the serializer's `validate()`, which runs only when every field was fine and raises at the first thing wrong.
    async fn validate(&self, ctx: &Ctx, scope: &WorkspaceScope) -> Result<(), V1Error> {
        // (upper-casing the identifier does not change whether a character is forbidden)
        if self.name.as_ref().map(Text::as_str).is_some_and(thrust_core::has_forbidden_project_chars) {
            return Err(non_field("Project name cannot contain special characters."));
        }
        if self.identifier.as_ref().map(Text::as_str).is_some_and(thrust_core::has_forbidden_project_chars) {
            return Err(non_field("Project identifier cannot contain special characters."));
        }
        if let Some(lead) = self.project_lead.and_then(NullableUuid::id) {
            if !thrust_core::is_workspace_member(&ctx.app, scope, UserId(lead)).await? {
                return Err(V1Error::Error(
                    StatusCode::BAD_REQUEST,
                    json!({ "project_lead": "The provided user is not a member of this workspace." }),
                ));
            }
        }
        if let Some(assignee) = self.default_assignee.and_then(NullableUuid::id) {
            if !thrust_core::is_workspace_member(&ctx.app, scope, UserId(assignee)).await? {
                return Err(non_field("Default assignee should be a user in the workspace"));
            }
        }
        Ok(())
    }
}

impl From<PatchedProjectUpdateRequest> for ProjectPatch {
    fn from(r: PatchedProjectUpdateRequest) -> Self {
        ProjectPatch {
            name: r.name.map(Text::into_string),
            // the model saves a key in upper case
            identifier: r.identifier.map(|key| key.into_string().to_uppercase()),
            description: r.description.map(Prose::into_string),
            emoji: r.emoji.map(NullableText::into_option),
            icon_prop: r.icon_prop.map(AnyJson::into_option),
            project_lead: r.project_lead.map(|p| p.id().map(UserId)),
            default_assignee: r.default_assignee.map(|p| p.id().map(UserId)),
            cycle_view: r.cycle_view,
            issue_views_view: r.issue_views_view,
            module_view: r.module_view,
            page_view: r.page_view,
            intake_view: r.intake_view,
            guest_view_all_features: r.guest_view_all_features,
            is_time_tracking_enabled: r.is_time_tracking_enabled,
            is_issue_type_enabled: r.is_issue_type_enabled,
            cover_image: r.cover_image.map(NullableProse::into_option),
            archive_in: r.archive_in.map(Whole::get),
            close_in: r.close_in.map(Whole::get),
            timezone: r.timezone.map(Timezone::into_string),
            external_source: r.external_source.map(NullableText::into_option),
            external_id: r.external_id.map(NullableText::into_option),
            default_state: r.default_state.map(NullableUuid::id),
            // `estimate` was checked and names no row here: nothing is kept of it
            ..ProjectPatch::default()
        }
    }
}

/// The creation's body in the document: `PatchedProjectUpdateRequest` with `name` and `identifier` required and without the keys that
/// only a change has, which `Creations` (`openapi.rs`) writes from the change's schema. The type is the name the path declarations
/// use for it.
#[cfg(feature = "openapi")]
#[derive(utoipa::ToSchema)]
#[allow(dead_code, reason = "a name for the document: nothing builds it")]
pub(crate) struct ProjectCreateRequest;

/// What a body says of a project, read and checked: every field first, then the rows they name, and what is wrong in either is one 400;
/// then the serializer's `validate()`.
async fn project_patch(
    ctx: &Ctx,
    scope: &WorkspaceScope,
    b: &Map<String, Value>,
    partial: bool,
) -> Result<ProjectPatch, V1Error> {
    let (request, mut errors) = PatchedProjectUpdateRequest::read(b, partial);
    request.check_rows(ctx, &mut errors).await?;
    refuse_if_any(errors)?;
    request.validate(ctx, scope).await?;
    Ok(request.into())
}

// --- the routes ---------------------------------------------------------------------------------------------------------------

/// `GET .../projects/`: the projects the caller sees, ordered and paged.
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        get,
        path = "/api/v1/workspaces/{slug}/projects/",
        operation_id = "list_projects",
        tag = "Projects",
        summary = "List or retrieve projects",
        params(("slug" = String, Path), crate::openapi::ListQuery),
        responses(
            crate::openapi::Refused,
            (status = 200, description = "Paginated list of projects", body = Page<ProjectDto>),
            (status = 404, description = "Project not found"),
        ),
        security(("ApiKeyAuthentication" = []))
    )
)]
pub(crate) async fn list(
    V1Path(slug): V1Path<String>,
    ctx: Ctx,
    RawQuery(raw): RawQuery,
) -> Result<Json<Page<Value>>, V1Error> {
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
        .map(|l| Ok(pick_fields(project_body(l, members.get(&l.project.id).copied().unwrap_or(0), true)?, &q)))
        .collect::<Result<Vec<_>, V1Error>>()?;
    Ok(Json(plan.page(results, total)))
}

/// `GET .../projects-lite/`: the same projects in nine keys; archived ones only when asked for.
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        get,
        path = "/api/v1/workspaces/{slug}/projects-lite/",
        operation_id = "list_projects_lite",
        tag = "Projects",
        summary = "List projects (lite)",
        params(("slug" = String, Path), crate::openapi::LiteQuery),
        responses(
            crate::openapi::Refused,
            (status = 200, description = "Paginated list of projects with minimal fields", body = Page<ProjectLiteDto>),
            (status = 404, description = "Workspace not found"),
        ),
        security(("ApiKeyAuthentication" = []))
    )
)]
pub(crate) async fn list_lite(
    V1Path(slug): V1Path<String>,
    ctx: Ctx,
    RawQuery(raw): RawQuery,
) -> Result<Json<Page<Value>>, V1Error> {
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
    let results = plan.slice(listings).iter().map(|l| lite_body(&l.project)).collect::<Result<Vec<_>, V1Error>>()?;
    Ok(Json(plan.page(results, total)))
}

/// `POST .../projects/`: make a project; the caller becomes its admin.
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        post,
        path = "/api/v1/workspaces/{slug}/projects/",
        operation_id = "create_project",
        tag = "Projects",
        summary = "Create project",
        params(("slug" = String, Path)),
        request_body(content = ProjectCreateRequest, content_type = "application/json"),
        responses(
            crate::openapi::Refused,
            (status = 201, description = "Project created successfully", body = ProjectDto),
            (status = 404, description = "Workspace not found"),
            (status = 409, description = "Project name already taken"),
        ),
        security(("ApiKeyAuthentication" = []))
    )
)]
pub(crate) async fn create(
    V1Path(slug): V1Path<String>,
    ctx: Ctx,
    headers: HeaderMap,
    body: Bytes,
) -> Result<(StatusCode, Json<Value>), V1Error> {
    let scope = ctx.workspace_scope(&slug, Role::Member).await?;
    let map = match json_body(&headers, &body)? {
        Body::Object(map) => map,
        // `{**request.data}` of a list raises, and the view's catch-all answers it
        Body::Array => {
            return Err(V1Error::unexpected("a JSON array body to project create: `{**request.data}` raises (PQ-406)"))
        }
    };
    let mut patch = project_patch(&ctx, &scope, &map, false).await?;
    // the pin's serializer reads no `logo_props` from a creation (measured: one that is given, in the emoji form, is ignored, as null is)
    // and gives the project a random icon and colour; the corpus erases the two values, so what is compared is the shape
    patch.logo_props = Some(default_logo(&ctx));
    let detail = thrust_core::create_project(&ctx.app, &scope, patch).await.map_err(|e| project_error(e, None))?;
    let members = i64::try_from(detail.members.len()).unwrap_or(i64::MAX);
    Ok((StatusCode::CREATED, Json(Value::Object(project_body(&detail.listing, members, false)?))))
}

/// The pin's `PROJECT_ICON_DEFAULT_ICONS` and `PROJECT_ICON_DEFAULT_COLORS` (`ProjectCreateSerializer`), the lists a new project's
/// default icon is drawn from.
pub const DEFAULT_ICONS: [&str; 28] = [
    "home",
    "apps",
    "settings",
    "star",
    "favorite",
    "done",
    "check_circle",
    "add_task",
    "create_new_folder",
    "dataset",
    "terminal",
    "key",
    "rocket",
    "public",
    "quiz",
    "mood",
    "gavel",
    "eco",
    "diamond",
    "forest",
    "bolt",
    "sync",
    "cached",
    "library_add",
    "view_timeline",
    "view_kanban",
    "empty_dashboard",
    "cycle",
];
pub const DEFAULT_COLORS: [&str; 8] =
    ["#95999f", "#6d7b8a", "#5e6ad2", "#02b5ed", "#02b55c", "#f2be02", "#e57a00", "#f38e82"];

/// A new project's default `logo_props`: an icon and a colour, each drawn from the pin's lists with the app's random source.
fn default_logo(ctx: &Ctx) -> Value {
    let mut draw = [0_u8; 2];
    // a source that fails gives the first of each: the shape is what a client relies on, not the choice
    let _ = ctx.app.ports.rng.fill(&mut draw);
    let [icon, color] = draw;
    let name = DEFAULT_ICONS.get(usize::from(icon) % DEFAULT_ICONS.len()).copied().unwrap_or("home");
    let color = DEFAULT_COLORS.get(usize::from(color) % DEFAULT_COLORS.len()).copied().unwrap_or("#95999f");
    json!({ "in_use": "icon", "icon": { "name": name, "color": color } })
}

/// `GET .../projects/{id}/`: a project the caller is in or a public one.
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        get,
        path = "/api/v1/workspaces/{slug}/projects/{pk}/",
        operation_id = "retrieve_project",
        tag = "Projects",
        summary = "Retrieve project",
        params(("slug" = String, Path), ("pk" = Uuid, Path)),
        responses(
            crate::openapi::Refused,
            (status = 200, description = "Project details", body = ProjectDto),
            (status = 404, description = "Project not found"),
        ),
        security(("ApiKeyAuthentication" = []))
    )
)]
pub(crate) async fn retrieve(
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
    Ok(Json(pick_fields(project_body(&detail.listing, members, false)?, &query_map(raw.as_deref()))))
}

/// `PATCH .../projects/{id}/`: change a project, for a writer. The pin reads the body first, then refuses an archived project, then
/// checks the fields.
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        patch,
        path = "/api/v1/workspaces/{slug}/projects/{pk}/",
        operation_id = "update_project",
        tag = "Projects",
        summary = "Update project",
        params(("slug" = String, Path), ("pk" = Uuid, Path)),
        // not required: an empty body is an empty object (`json_body`), as DRF reads it
        request_body(content = Option<PatchedProjectUpdateRequest>, content_type = "application/json"),
        responses(
            crate::openapi::Refused,
            (status = 200, description = "Project updated successfully", body = ProjectDto),
            (status = 404, description = "Project not found"),
            (status = 409, description = "Project name already taken"),
        ),
        security(("ApiKeyAuthentication" = []))
    )
)]
pub(crate) async fn update(
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
        Body::Array => {
            return Err(V1Error::internal("a JSON array body to project update: `{**request.data}` raises (PQ-406)"))
        }
    };
    if before.listing.project.archived_at.is_some() {
        return Err(project_error(ProjectError::Archived, None));
    }
    let patch = project_patch(&ctx, &scope, &map, true).await?;
    let state = patch.default_state.flatten();
    let detail = thrust_core::change_project(&ctx.app, &scope, id, patch).await.map_err(|e| project_error(e, state))?;
    let members = i64::try_from(detail.members.len()).unwrap_or(i64::MAX);
    Ok(Json(Value::Object(project_body(&detail.listing, members, false)?)))
}

/// `DELETE .../projects/{id}/`: delete a project, for a writer.
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        delete,
        path = "/api/v1/workspaces/{slug}/projects/{pk}/",
        operation_id = "delete_project",
        tag = "Projects",
        summary = "Delete project",
        params(("slug" = String, Path), ("pk" = Uuid, Path)),
        responses(
            crate::openapi::Refused,
            (status = 204, description = "Resource deleted successfully"),
            (status = 404, description = "The requested resource was not found."),
        ),
        security(("ApiKeyAuthentication" = []))
    )
)]
pub(crate) async fn destroy(V1Path((slug, id)): V1Path<(String, PlaneUuid)>, ctx: Ctx) -> Result<StatusCode, V1Error> {
    let id = ProjectId(id.0);
    let scope = ctx.workspace_access(&slug).await?;
    thrust_core::project_writer(&ctx.app, &scope, id).await.map_err(|e| project_error(e, None))?;
    ctx.throttle().await?;
    thrust_core::delete_project(&ctx.app, &scope, id).await.map_err(|e| project_error(e, None))?;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST .../projects/{id}/archive/`.
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        post,
        path = "/api/v1/workspaces/{slug}/projects/{project_id}/archive/",
        operation_id = "archive_project",
        tag = "Projects",
        summary = "Archive project",
        params(("slug" = String, Path), ("project_id" = Uuid, Path)),
        responses(
            crate::openapi::Refused,
            (status = 204, description = "Resource archived successfully"),
            (status = 404, description = "The requested resource was not found."),
        ),
        security(("ApiKeyAuthentication" = []))
    )
)]
pub(crate) async fn archive(V1Path((slug, id)): V1Path<(String, PlaneUuid)>, ctx: Ctx) -> Result<StatusCode, V1Error> {
    set_archived(&slug, id, ctx, true).await
}

/// `DELETE .../projects/{id}/archive/`.
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        delete,
        path = "/api/v1/workspaces/{slug}/projects/{project_id}/archive/",
        operation_id = "unarchive_project",
        tag = "Projects",
        summary = "Unarchive project",
        params(("slug" = String, Path), ("project_id" = Uuid, Path)),
        responses(
            crate::openapi::Refused,
            (status = 204, description = "Resource unarchived successfully"),
            (status = 404, description = "The requested resource was not found."),
        ),
        security(("ApiKeyAuthentication" = []))
    )
)]
pub(crate) async fn unarchive(
    V1Path((slug, id)): V1Path<(String, PlaneUuid)>,
    ctx: Ctx,
) -> Result<StatusCode, V1Error> {
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
