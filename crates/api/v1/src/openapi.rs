//! The OpenAPI document of `/api/v1` (issue #58, ADR-0032), written from the types the routes answer with. Only with the `openapi`
//! feature: nothing here is in the Worker or the server.
//!
//! What it is for. The pin is Plane's own schema (`tools/oracle-v1/schema/openapi.v1.json`, OpenAPI 3.0.3, the contract). This
//! document says what THIS crate sends and accepts, so `oasdiff` can say where the two differ (`scripts/check-openapi.sh`). The
//! answers are derived from the types the routes serialize (`dto`), so they cannot say one thing while the route does another. A
//! request is read by hand (`fields.rs`: DRF's coercions and DRF's words). The States' is read into a type that writes its own
//! schema (`states::PatchedStateRequest`, with the bounded values of `values.rs`), so for them the document is the parser's; the
//! other request bodies below are descriptions of what a reading takes, written by hand, until each parser returns a typed struct.
//!
//! `cargo run -p thrust-api-v1 --features openapi --example openapi` prints the document.

use utoipa::openapi::schema::{Object, ObjectBuilder, SchemaType, Type};
use utoipa::openapi::security::{ApiKey, ApiKeyValue, SecurityScheme};
use utoipa::{IntoParams, IntoResponses, Modify, OpenApi, ToSchema};

use crate::dto::{IssueDto, IssueSearchDto, LabelDto, Listing, PriorityEnum, ProjectDto, ProjectLiteDto, StateDto};
use crate::pagination::Page;
use thrust_contracts::Uuid;

/// The document: the 32 operations of the scope `m1` (Projects, States, Labels and Work Items), each written from the types its
/// route answers with.
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
        crate::work_items::list,
        crate::work_items::create,
        crate::work_items::retrieve,
        crate::work_items::update,
        crate::work_items::destroy,
        crate::work_items::search,
        crate::work_items::by_identifier,
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
        crate::states::StateRequest,
        crate::states::PatchedStateRequest,
        LabelDto,
        Page<LabelDto>,
        LabelCreateUpdateRequest,
        PatchedLabelCreateUpdateRequest,
        IssueDto,
        Page<IssueDto>,
        Listing<IssueDto>,
        PriorityEnum,
        IssueSearchDto,
        IssueRequest,
        PatchedIssueRequest
    )),
    modifiers(&ApiKeyAuthentication, &Creations, &OptionalBodies, &WorkItemsTwice)
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

/// A creation takes the keys of a change, and requires some of them. The change's schema is written from the type that the parser
/// reads the body into; the creation's is made of it here, with the list of required keys that the same parser checks. So the two
/// cannot differ in a key, a bound, a type or a `null`, and the one thing that tells them apart is the parser's own list.
struct Creations;

/// One creation: the name of its schema, the change it is made of, the keys it requires, and what it says of itself.
struct Creation {
    schema: &'static str,
    change: &'static str,
    required: &'static [&'static str],
    description: &'static str,
}

const CREATIONS: [Creation; 1] = [Creation {
    schema: "StateRequest",
    change: "PatchedStateRequest",
    required: &crate::states::PatchedStateRequest::REQUIRED,
    description: "What a state's creation takes (the pin's `StateSerializer` as a request): `name` and `color` are required, the rest is optional, and\nonly the two external keys can be `null`.",
}];

impl Modify for Creations {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        use utoipa::openapi::{RefOr, Schema};
        let components = openapi.components.get_or_insert_with(Default::default);
        for creation in &CREATIONS {
            // a change that is not an object is left as it is: the test of the pair says what is missing
            let Some(RefOr::T(Schema::Object(change))) = components.schemas.get(creation.change) else { continue };
            let mut object = change.clone();
            object.required = creation.required.iter().map(|key| (*key).to_string()).collect();
            object.description = Some(creation.description.to_string());
            components.schemas.insert(creation.schema.to_string(), RefOr::T(Schema::Object(object)));
        }
    }
}

/// A body that may be left out is `Option<T>` in a `#[utoipa::path]`, which utoipa writes as `oneOf [T, null]`. The pin writes it as
/// `T` in a body that is not `required`, which is what a change here is: an empty body is an empty object (`request::json_body`), and the
/// JSON value `null` is refused (400, "Request body must be a JSON object or array."), so the `null` is a claim the routes do not
/// keep. This takes it out, and the `oneOf` with it, which also hid the body from `oasdiff` (it reports a body wrapped in a `oneOf`).
struct OptionalBodies;

impl Modify for OptionalBodies {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        use utoipa::openapi::{RefOr, Schema};
        for item in openapi.paths.paths.values_mut() {
            let operations = [
                &mut item.get,
                &mut item.put,
                &mut item.post,
                &mut item.delete,
                &mut item.options,
                &mut item.head,
                &mut item.patch,
                &mut item.trace,
            ];
            for operation in operations.into_iter().flatten() {
                let Some(RefOr::T(body)) = operation.request_body.as_mut() else { continue };
                for content in body.content.values_mut() {
                    let RefOr::T(content) = content else { continue };
                    let Some(RefOr::T(Schema::OneOf(one_of))) = content.schema.as_mut() else { continue };
                    let is_null = |s: &RefOr<Schema>| matches!(s, RefOr::T(Schema::Object(o)) if o.schema_type == SchemaType::Type(Type::Null));
                    if one_of.items.len() == 2 && one_of.items.iter().any(is_null) {
                        if let Some(only) = one_of.items.iter().find(|s| !is_null(s)).cloned() {
                            content.schema = Some(only);
                        }
                    }
                }
            }
        }
    }
}

/// The pin documents each work item operation twice: under `issues/` and under `work-items/`, the second with an operation id that
/// ends in `_2` (`work_items::routes` registers both families on the same handlers). The `#[utoipa::path]` of those handlers describe
/// the `issues/` family, and this derives the other from it, so the two cannot differ: for every path that has `/issues/` in it, one
/// with `/work-items/` in its place and each operation id followed by `_2`.
struct WorkItemsTwice;

impl Modify for WorkItemsTwice {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        let first: Vec<(String, utoipa::openapi::path::PathItem)> = openapi
            .paths
            .paths
            .iter()
            .filter(|(path, _)| path.contains("/issues/"))
            .map(|(path, item)| (path.clone(), item.clone()))
            .collect();
        for (path, mut item) in first {
            let operations = [
                &mut item.get,
                &mut item.put,
                &mut item.post,
                &mut item.delete,
                &mut item.options,
                &mut item.head,
                &mut item.patch,
                &mut item.trace,
            ];
            for operation in operations.into_iter().flatten() {
                operation.operation_id = operation.operation_id.take().map(|id| format!("{id}_2"));
            }
            openapi.paths.paths.insert(path.replacen("/issues/", "/work-items/", 1), item);
        }
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

/// A work item's `point`: a whole number from 0 to 12 or `null`, with no `format`, as the pin writes it. The database checks the range,
/// and a request that names another is refused.
pub(crate) fn point() -> Object {
    ObjectBuilder::new()
        .schema_type([Type::Integer, Type::Null].into_iter().collect::<SchemaType>())
        .minimum(Some(0))
        .maximum(Some(12))
        .build()
}

/// A work item's `priority` as the pin writes it: the enum by reference, inside an `allOf` (how drf-spectacular gives a field of an enum
/// type a title of its own). The same enum either way; written like the pin's so that `oasdiff` compares the enum itself, and a value
/// that is added or lost is a finding, where two forms of one enum would need an entry that hid it.
pub(crate) fn priority() -> utoipa::openapi::RefOr<utoipa::openapi::Schema> {
    utoipa::openapi::RefOr::T(utoipa::openapi::Schema::AllOf(
        utoipa::openapi::schema::AllOfBuilder::new()
            .item(utoipa::openapi::Ref::from_schema_name("PriorityEnum"))
            .title(Some("Issue Priority"))
            .build(),
    ))
}

/// A `sequence_id` that a request may carry (and that is read, checked and not kept): a 32-bit whole number, with no `format`, as the
/// pin writes it.
pub(crate) fn request_sequence_id() -> Object {
    ObjectBuilder::new()
        .schema_type(Type::Integer)
        .minimum(Some(i64::from(i32::MIN)))
        .maximum(Some(i64::from(i32::MAX)))
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

/// The query of the work items' list: the page, `fields`, `expand`, `order_by` (an allowlist; another is the default), and the two halves
/// of an integration key, which together make the list the one work item that has it.
#[derive(IntoParams)]
#[into_params(parameter_in = Query)]
#[allow(dead_code, reason = "a description for the document: nothing builds it")]
pub(crate) struct WorkItemsQuery {
    cursor: Option<String>,
    expand: Option<String>,
    external_id: Option<String>,
    external_source: Option<String>,
    fields: Option<String>,
    order_by: Option<String>,
    #[param(schema_with = whole_number)]
    per_page: Option<i64>,
}

/// The query of a read of one work item by id: the pin lists the five below, and the route reads `fields` and `expand` and nothing else.
#[derive(IntoParams)]
#[into_params(parameter_in = Query)]
#[allow(dead_code, reason = "a description for the document: nothing builds it")]
pub(crate) struct WorkItemQuery {
    expand: Option<String>,
    external_id: Option<String>,
    external_source: Option<String>,
    fields: Option<String>,
    order_by: Option<String>,
}

/// The query of the workspace search. `search` is required in the pin's schema; a request without it is answered with no hits, as the
/// pin answers it.
#[derive(IntoParams)]
#[into_params(parameter_in = Query)]
#[allow(dead_code, reason = "a description for the document: nothing builds it")]
pub(crate) struct SearchQuery {
    #[param(schema_with = whole_number)]
    limit: Option<i64>,
    project_id: Option<Uuid>,
    search: String,
    workspace_search: Option<String>,
}

/// The path of a work item by the identifier of its project and its number (`PLATF-7`): the route splits the segment at its last hyphen.
#[derive(IntoParams)]
#[into_params(parameter_in = Path)]
#[allow(dead_code, reason = "a description for the document: nothing builds it")]
pub(crate) struct ByIdentifierPath {
    slug: String,
    project_identifier: String,
    #[param(schema_with = whole_number)]
    issue_identifier: i64,
}

/// The query of a work item by identifier: the pin's schema declares none, and the route reads `fields` and `expand` as a read by id does.
#[derive(IntoParams)]
#[into_params(parameter_in = Query)]
#[allow(dead_code, reason = "a description for the document: nothing builds it")]
pub(crate) struct ByIdentifierQuery {
    expand: Option<String>,
    fields: Option<String>,
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

/// What a work item's creation takes (the pin's `IssueSerializer` as a request): only `name` is required, and it cannot be blank. The
/// route reads every key the serializer has, and keeps the name, the description, the priority, the two dates, the state, the parent,
/// the point, the people, the labels and the integration key; the rest is read and checked and then not kept (D-033 (1)). `priority`
/// and the lists are not `null`; a date, a key and the relations can be.
#[derive(ToSchema)]
#[allow(dead_code, reason = "a description for the document: the route reads the body by hand (`fields.rs`)")]
pub(crate) struct IssueRequest {
    #[schema(min_length = 1, max_length = 255)]
    name: String,
    #[schema(required = false)]
    description_html: String,
    #[schema(required = false, schema_with = priority)]
    priority: PriorityEnum,
    #[schema(format = Date)]
    start_date: Option<String>,
    #[schema(format = Date)]
    target_date: Option<String>,
    parent: Option<Uuid>,
    state: Option<Uuid>,
    #[schema(schema_with = point)]
    point: Option<i32>,
    #[schema(required = false)]
    assignees: Vec<Uuid>,
    #[schema(required = false)]
    labels: Vec<Uuid>,
    #[schema(max_length = 255)]
    external_source: Option<String>,
    #[schema(max_length = 255)]
    external_id: Option<String>,
    // read, checked and not kept
    #[schema(required = false, schema_with = request_sequence_id)]
    sequence_id: i32,
    #[schema(required = false)]
    sort_order: f64,
    #[schema(format = Date)]
    archived_at: Option<String>,
    #[schema(required = false)]
    is_draft: bool,
    #[schema(format = DateTime)]
    deleted_at: Option<String>,
    created_by: Option<Uuid>,
    estimate_point: Option<Uuid>,
    #[schema(rename = "type")]
    type_: Option<Uuid>,
    type_id: Option<Uuid>,
}

/// What a work item's change takes: the creation's keys, none required.
#[derive(ToSchema)]
#[allow(dead_code, reason = "a description for the document: the route reads the body by hand (`fields.rs`)")]
pub(crate) struct PatchedIssueRequest {
    #[schema(min_length = 1, max_length = 255, required = false)]
    name: String,
    #[schema(required = false)]
    description_html: String,
    #[schema(required = false, schema_with = priority)]
    priority: PriorityEnum,
    #[schema(format = Date)]
    start_date: Option<String>,
    #[schema(format = Date)]
    target_date: Option<String>,
    parent: Option<Uuid>,
    state: Option<Uuid>,
    #[schema(schema_with = point)]
    point: Option<i32>,
    #[schema(required = false)]
    assignees: Vec<Uuid>,
    #[schema(required = false)]
    labels: Vec<Uuid>,
    #[schema(max_length = 255)]
    external_source: Option<String>,
    #[schema(max_length = 255)]
    external_id: Option<String>,
    // read, checked and not kept
    #[schema(required = false, schema_with = request_sequence_id)]
    sequence_id: i32,
    #[schema(required = false)]
    sort_order: f64,
    #[schema(format = Date)]
    archived_at: Option<String>,
    #[schema(required = false)]
    is_draft: bool,
    #[schema(format = DateTime)]
    deleted_at: Option<String>,
    created_by: Option<Uuid>,
    estimate_point: Option<Uuid>,
    #[schema(rename = "type")]
    type_: Option<Uuid>,
    type_id: Option<Uuid>,
}
