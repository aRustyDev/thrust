//! The OpenAPI document of `/api/v1` (issue #58, ADR-0032), written from the types the routes answer with. Only with the `openapi`
//! feature: nothing here is in the Worker or the server.
//!
//! What it is for. The pin is Plane's own schema (`tools/oracle-v1/schema/openapi.v1.json`, OpenAPI 3.0.3, the contract). This
//! document says what THIS crate sends and accepts, so `oasdiff` can say where the two differ (`scripts/check-openapi.sh`). The
//! answers are derived from the types the routes serialize (`dto`), so they cannot say one thing while the route does another. A
//! request is read by hand (`fields.rs`: DRF's coercions and DRF's words), so the request bodies below are descriptions of what
//! that reading takes, written by hand; a request type that a parser returns is a later step.
//!
//! `cargo run -p thrust-api-v1 --features openapi --example openapi` prints the document.

use utoipa::openapi::schema::{Object, ObjectBuilder, SchemaType, Type};
use utoipa::openapi::security::{ApiKey, ApiKeyValue, SecurityScheme};
use utoipa::{IntoParams, IntoResponses, Modify, OpenApi, ToSchema};

use crate::dto::{LabelDto, ProjectDto, ProjectLiteDto, StateDto};
use crate::pagination::Page;
use thrust_contracts::Uuid;

/// The document: every operation of the resources that are described so far (a resource is added as its bodies are types).
#[derive(OpenApi)]
#[openapi(
    info(
        title = "Thrust /api/v1",
        version = "0.0.0",
        description = "Plane's public v1 API as Thrust serves it. Compared with the pinned Plane schema by `oasdiff`."
    ),
    paths(
        crate::projects::list,
        crate::projects::create,
        crate::projects::list_lite,
        crate::projects::retrieve,
        crate::projects::update,
        crate::projects::destroy,
        crate::projects::archive,
        crate::projects::unarchive,
        crate::states::list,
        crate::states::create,
        crate::states::retrieve,
        crate::states::update,
        crate::states::destroy,
        crate::labels::list,
        crate::labels::create,
        crate::labels::retrieve,
        crate::labels::update,
        crate::labels::destroy,
    ),
    components(schemas(
        ProjectDto,
        ProjectLiteDto,
        Page<ProjectDto>,
        Page<ProjectLiteDto>,
        ProjectCreateRequest,
        PatchedProjectUpdateRequest,
        StateDto,
        Page<StateDto>,
        StateRequest,
        PatchedStateRequest,
        LabelDto,
        Page<LabelDto>,
        LabelCreateUpdateRequest,
        PatchedLabelCreateUpdateRequest
    )),
    modifiers(&ApiKeyAuthentication)
)]
struct ApiDoc;

/// The document as a value.
pub fn openapi() -> utoipa::openapi::OpenApi {
    ApiDoc::openapi()
}

/// The pin's one security scheme: the `X-API-Key` header, on every operation.
struct ApiKeyAuthentication;

impl Modify for ApiKeyAuthentication {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        let components = openapi.components.get_or_insert_with(Default::default);
        components.add_security_scheme(
            "ApiKeyAuthentication",
            SecurityScheme::ApiKey(ApiKey::Header(ApiKeyValue::with_description(
                "X-API-Key",
                "API key authentication. Provide your API key in the X-API-Key header.",
            ))),
        );
    }
}

// --- schemas that a Rust type does not say -------------------------------------------------------------------------------------
//
// A field's Rust type gives its JSON type, and `Option` its nullability; these are what it cannot give. Each is used by the request
// descriptions and by the bodies the routes answer with, so a bound or a list is written once.

/// A whole number of months, 0 to 12 (`archive_in`, `close_in`), with no `format`, as the pin writes it.
pub(crate) fn months() -> Object {
    ObjectBuilder::new().schema_type(Type::Integer).minimum(Some(0)).maximum(Some(12)).build()
}

/// A whole number with no `format` and no bound (`per_page`).
pub(crate) fn whole_number() -> Object {
    ObjectBuilder::new().schema_type(Type::Integer).build()
}

/// A whole number that can be `null`, with no `format` (`member_role`: `null` for a project the caller sees only because it is public).
pub(crate) fn nullable_whole_number() -> Object {
    ObjectBuilder::new().schema_type([Type::Integer, Type::Null].into_iter().collect::<SchemaType>()).build()
}

/// A project's `network`: 0 (secret) or 2 (public).
pub(crate) fn network() -> Object {
    ObjectBuilder::new().schema_type(Type::Integer).enum_values(Some([0, 2])).build()
}

/// A timezone: one of the names `thrust_core::is_known_timezone` says yes to, which is the list that a route checks a body against.
pub(crate) fn timezone() -> Object {
    ObjectBuilder::new()
        .schema_type(Type::String)
        .enum_values(Some(thrust_core::known_timezones().iter().copied()))
        .build()
}

/// A state's group: one of the six `StateGroup` has, which is what a route checks a body's `group` against and what a body says.
pub(crate) fn state_group() -> Object {
    ObjectBuilder::new()
        .schema_type(Type::String)
        .enum_values(Some(thrust_contracts::StateGroup::ALL.iter().map(|g| g.as_str())))
        .build()
}

/// The two answers that every operation can give before it does anything.
#[derive(IntoResponses)]
#[allow(dead_code, reason = "a description for the document: nothing builds it")]
pub(crate) enum Refused {
    /// Authentication credentials were not provided or are invalid.
    #[response(status = 401)]
    Unauthorized,
    /// Permission denied. User lacks required permissions.
    #[response(status = 403)]
    Forbidden,
}

/// The query of a list, with the five parameters the pin's list schemas give: `cursor`, `expand`, `fields`, `order_by` and `per_page`.
/// A route may take fewer of them than it lists: the labels' list ignores `expand` and `order_by`, as the pin's does (D-032 (2);
/// `tests/labels.rs`, `fields_and_pages_work_on_the_list_and_order_by_and_expand_do_nothing`), and sending them changes nothing.
#[derive(IntoParams)]
#[into_params(parameter_in = Query)]
#[allow(dead_code, reason = "a description for the document: nothing builds it")]
pub(crate) struct ListQuery {
    cursor: Option<String>,
    expand: Option<String>,
    fields: Option<String>,
    order_by: Option<String>,
    #[param(schema_with = whole_number)]
    per_page: Option<i64>,
}

/// The query of `projects-lite/`, which has no `fields` or `expand` and can include archived projects.
#[derive(IntoParams)]
#[into_params(parameter_in = Query)]
#[allow(dead_code, reason = "a description for the document: nothing builds it")]
pub(crate) struct LiteQuery {
    cursor: Option<String>,
    include_archived: Option<bool>,
    order_by: Option<String>,
    #[param(schema_with = whole_number)]
    per_page: Option<i64>,
}

/// The query of `states/`, which reads `fields` and the page and has no `order_by`.
#[derive(IntoParams)]
#[into_params(parameter_in = Query)]
#[allow(dead_code, reason = "a description for the document: nothing builds it")]
pub(crate) struct StatesQuery {
    cursor: Option<String>,
    expand: Option<String>,
    fields: Option<String>,
    #[param(schema_with = whole_number)]
    per_page: Option<i64>,
}

/// What a project's creation takes (the pin's `ProjectCreateSerializer`). `description` and the two switches are optional and not
/// nullable; the ids and the texts that can be `null` are `Option`.
#[derive(ToSchema)]
#[allow(dead_code, reason = "a description for the document: the route reads the body by hand (`fields.rs`)")]
pub(crate) struct ProjectCreateRequest {
    #[schema(min_length = 1, max_length = 255)]
    name: String,
    #[schema(required = false)]
    description: String,
    project_lead: Option<Uuid>,
    default_assignee: Option<Uuid>,
    #[schema(min_length = 1, max_length = 12)]
    identifier: String,
    icon_prop: Option<serde_json::Value>,
    #[schema(max_length = 255)]
    emoji: Option<String>,
    cover_image: Option<String>,
    #[schema(required = false)]
    module_view: bool,
    #[schema(required = false)]
    cycle_view: bool,
    #[schema(required = false)]
    issue_views_view: bool,
    #[schema(required = false)]
    page_view: bool,
    #[schema(required = false)]
    intake_view: bool,
    #[schema(required = false)]
    guest_view_all_features: bool,
    #[schema(required = false, schema_with = months)]
    archive_in: i32,
    #[schema(required = false, schema_with = months)]
    close_in: i32,
    #[schema(required = false, schema_with = timezone)]
    timezone: String,
    #[schema(max_length = 255)]
    external_source: Option<String>,
    #[schema(max_length = 255)]
    external_id: Option<String>,
    #[schema(required = false)]
    is_issue_type_enabled: bool,
    #[schema(required = false)]
    is_time_tracking_enabled: bool,
}

/// What a project's change takes (the pin's `ProjectUpdateSerializer`): the creation's keys, none required, and the two ids that a
/// change can set.
#[derive(ToSchema)]
#[allow(dead_code, reason = "a description for the document: the route reads the body by hand (`fields.rs`)")]
pub(crate) struct PatchedProjectUpdateRequest {
    #[schema(min_length = 1, max_length = 255, required = false)]
    name: String,
    #[schema(required = false)]
    description: String,
    project_lead: Option<Uuid>,
    default_assignee: Option<Uuid>,
    #[schema(min_length = 1, max_length = 12, required = false)]
    identifier: String,
    icon_prop: Option<serde_json::Value>,
    #[schema(max_length = 255)]
    emoji: Option<String>,
    cover_image: Option<String>,
    #[schema(required = false)]
    module_view: bool,
    #[schema(required = false)]
    cycle_view: bool,
    #[schema(required = false)]
    issue_views_view: bool,
    #[schema(required = false)]
    page_view: bool,
    #[schema(required = false)]
    intake_view: bool,
    #[schema(required = false)]
    guest_view_all_features: bool,
    #[schema(required = false, schema_with = months)]
    archive_in: i32,
    #[schema(required = false, schema_with = months)]
    close_in: i32,
    #[schema(required = false, schema_with = timezone)]
    timezone: String,
    #[schema(max_length = 255)]
    external_source: Option<String>,
    #[schema(max_length = 255)]
    external_id: Option<String>,
    #[schema(required = false)]
    is_issue_type_enabled: bool,
    #[schema(required = false)]
    is_time_tracking_enabled: bool,
    default_state: Option<Uuid>,
    estimate: Option<Uuid>,
}

/// What a state's creation takes (the pin's `StateSerializer` as a request): `name` and `color` are required, the rest is optional, and
/// only the two external keys can be `null`.
#[derive(ToSchema)]
#[allow(dead_code, reason = "a description for the document: the route reads the body by hand (`fields.rs`)")]
pub(crate) struct StateRequest {
    #[schema(min_length = 1, max_length = 255)]
    name: String,
    #[schema(required = false)]
    description: String,
    #[schema(min_length = 1, max_length = 255)]
    color: String,
    #[schema(required = false)]
    sequence: f64,
    #[schema(required = false, schema_with = state_group)]
    group: String,
    #[schema(required = false)]
    default: bool,
    #[schema(required = false)]
    is_triage: bool,
    #[schema(max_length = 255)]
    external_source: Option<String>,
    #[schema(max_length = 255)]
    external_id: Option<String>,
}

/// What a label's creation takes (the pin's `LabelCreateUpdateSerializer`): only `name` is required, and `name` cannot be blank. `color`
/// and `description` can be blank and cannot be `null`; the two external keys and `parent` can be `null`.
#[derive(ToSchema)]
#[allow(dead_code, reason = "a description for the document: the route reads the body by hand (`fields.rs`)")]
pub(crate) struct LabelCreateUpdateRequest {
    #[schema(min_length = 1, max_length = 255)]
    name: String,
    #[schema(required = false, max_length = 255)]
    color: String,
    #[schema(required = false)]
    description: String,
    #[schema(max_length = 255)]
    external_source: Option<String>,
    #[schema(max_length = 255)]
    external_id: Option<String>,
    parent: Option<Uuid>,
    #[schema(required = false)]
    sort_order: f64,
}

/// What a label's change takes: the creation's keys, none required.
#[derive(ToSchema)]
#[allow(dead_code, reason = "a description for the document: the route reads the body by hand (`fields.rs`)")]
pub(crate) struct PatchedLabelCreateUpdateRequest {
    #[schema(min_length = 1, max_length = 255, required = false)]
    name: String,
    #[schema(required = false, max_length = 255)]
    color: String,
    #[schema(required = false)]
    description: String,
    #[schema(max_length = 255)]
    external_source: Option<String>,
    #[schema(max_length = 255)]
    external_id: Option<String>,
    parent: Option<Uuid>,
    #[schema(required = false)]
    sort_order: f64,
}

/// What a state's change takes: the creation's keys, none required.
#[derive(ToSchema)]
#[allow(dead_code, reason = "a description for the document: the route reads the body by hand (`fields.rs`)")]
pub(crate) struct PatchedStateRequest {
    #[schema(min_length = 1, max_length = 255, required = false)]
    name: String,
    #[schema(required = false)]
    description: String,
    #[schema(min_length = 1, max_length = 255, required = false)]
    color: String,
    #[schema(required = false)]
    sequence: f64,
    #[schema(required = false, schema_with = state_group)]
    group: String,
    #[schema(required = false)]
    default: bool,
    #[schema(required = false)]
    is_triage: bool,
    #[schema(max_length = 255)]
    external_source: Option<String>,
    #[schema(max_length = 255)]
    external_id: Option<String>,
}
