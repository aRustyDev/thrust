//! The work items of Plane's v1 API: list, create, retrieve, change and delete under a project, and the search and the lookup by
//! `KEY-n` under the workspace, each in two families of paths (`issues/` and `work-items/`) that share their views.
//! The rules are core's (`thrust_core::issues`, with the v1 surface); this is the pin's wire shape over them: the 29-key body,
//! the serializer's fields read and checked in DRF's words, `fields` and `expand`, the list's refusals and its single-object mode,
//! and the order in which the pin does things.
//!
//! What the pin does with a body, and what is built (research/0014, D-033): every field of its `IssueSerializer` is read and checked;
//! a name, a description, a priority, a state, a parent, a point, the two dates, the people, the labels and the integration key are
//! kept, and the rest (`created_at`, `created_by`, `deleted_at`, `archived_at`, `is_draft`, `sequence_id`, `sort_order`, an estimate
//! point and a type) is read and checked and then not kept.

use std::collections::{BTreeSet, HashMap};

use axum::body::Bytes;
use axum::extract::RawQuery;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::get;
use axum::{Json, Router};
use serde_json::{json, Map, Value};
use thrust_contracts::chrono::NaiveDate;
use thrust_contracts::{Priority, ProjectId, Role, UserId, Uuid, WorkItemId, WorkItemRow, WorkspaceSlug};
use thrust_core::{Ask, IssueAccess, IssueDraft, IssueError, IssueListQuery, IssueUpdate};
use thrust_ports::{WorkItemOrder, WorkItemOrderField, WorkspaceSearch};

use crate::dto::{
    into_object, IssueDto, IssueSearchDto, IssueSearchItemDto, LabelDto, LabelRef, Listing, StateBriefDto, StateRef,
    UserBriefDto, UserRef,
};
use crate::fields::{
    choice_of, fail, no_such_pk_shown, not_a_dictionary, parse_uuid_text, pk_field, py_int, py_type_name,
    refuse_if_any, request, whole_numbers, Field, FieldErrors, PkRead,
};
use crate::labels::label_dto;
use crate::pagination::PageRequest;
use crate::projects::ts;
use crate::request::{json_body, pick_fields, query_map, Body};
use crate::values::{
    Key, NullableDate, NullableDateTime, NullableKey, NullableText, NullableWhole, Prose, SentText, Whole,
};
use crate::{with_allow, Ctx, PlaneUuid, V1Error, V1Path};

pub(crate) fn routes() -> Router {
    let mut r = Router::new();
    for family in ["issues", "work-items"] {
        r = r
            .route(
                &format!("/api/v1/workspaces/{{slug}}/projects/{{project_id}}/{family}/"),
                with_allow("GET, POST", get(list).post(create)),
            )
            .route(
                &format!("/api/v1/workspaces/{{slug}}/projects/{{project_id}}/{family}/{{pk}}/"),
                with_allow("GET, PATCH, DELETE", get(retrieve).patch(update).delete(destroy)),
            )
            // `search/` is a literal and wins over the identifier below, as Django's route order makes it
            .route(&format!("/api/v1/workspaces/{{slug}}/{family}/search/"), with_allow("GET", get(search)))
            .route(&format!("/api/v1/workspaces/{{slug}}/{family}/{{ident}}/"), with_allow("GET", get(by_identifier)));
    }
    r
}

// --- who may, and what a refusal says -------------------------------------------------------------------------------------------

/// What the pin answers for a refusal of a work item route. `Fields` is not expected here (the request layer has checked what a
/// body names); one that comes anyway is the pin's generic 400.
fn item_error(e: IssueError) -> V1Error {
    match e {
        IssueError::Forbidden | IssueError::NotAllowedToView => V1Error::PermissionDenied,
        IssueError::NotFound => V1Error::NotFound,
        IssueError::StartAfterTarget => non_field("Start date cannot exceed target date"),
        IssueError::OnlyAdminOrCreator => {
            V1Error::error(StatusCode::FORBIDDEN, "Only admin or creator can delete the work item")
        }
        IssueError::Fields(_) => V1Error::error(StatusCode::BAD_REQUEST, "The payload is not valid"),
        IssueError::Core(other) => other.into(),
    }
}

/// A DRF `ValidationError("message")` raised from `validate()`: it has no field, so it is a non-field error.
fn non_field(message: &str) -> V1Error {
    let mut errors = FieldErrors::new();
    fail(&mut errors, "non_field_errors", message);
    V1Error::Fields(errors)
}

/// The caller's way into the project's work items for a read (any member), then the throttle.
async fn reader(ctx: &Ctx, slug: &str, project: ProjectId) -> Result<IssueAccess, V1Error> {
    let access = thrust_core::v1_issue_reader(&ctx.app, &ctx.principal, &WorkspaceSlug::new(slug), project)
        .await
        .map_err(item_error)?;
    ctx.throttle().await?;
    Ok(access)
}

/// The caller's way in for a write (an admin or a member), then the throttle.
async fn writer(ctx: &Ctx, slug: &str, project: ProjectId) -> Result<IssueAccess, V1Error> {
    let access = thrust_core::v1_issue_writer(&ctx.app, &ctx.principal, &WorkspaceSlug::new(slug), project)
        .await
        .map_err(item_error)?;
    ctx.throttle().await?;
    Ok(access)
}

// --- the wire shape ------------------------------------------------------------------------------------------------------------

/// The parts of a work item that `expand` swaps for objects, read once for a page: the states, the people and the labels it names.
#[derive(Default)]
struct Expansion {
    state: Option<HashMap<Uuid, StateBriefDto>>,
    assignees: Option<HashMap<UserId, UserBriefDto>>,
    labels: Option<HashMap<Uuid, LabelDto>>,
}

/// `Issue`'s body as the pin's `IssueSerializer` writes it, as an [`IssueDto`]: 29 keys. A work item that is read is live, so it is not
/// a draft, not archived and not deleted; it has no estimate point and no type. A person, a state or a label that `expand` asked for
/// and that is not among the ones read is its id, as for a request that did not ask.
fn work_item_dto(r: &WorkItemRow, x: &Expansion) -> IssueDto {
    let (i, a) = (&r.item, &r.attrs);
    let user = |u: Option<UserId>| u.map(|u| u.0);
    let state = a.state_id.map(|id| match x.state.as_ref().and_then(|m| m.get(&id)) {
        Some(brief) => StateRef::Expanded(brief.clone()),
        None => StateRef::Id(id),
    });
    let assignees = a
        .assignee_ids
        .iter()
        .map(|u| match x.assignees.as_ref().and_then(|m| m.get(u)) {
            Some(brief) => UserRef::Expanded(brief.clone()),
            None => UserRef::Id(u.0),
        })
        .collect();
    let labels = a
        .label_ids
        .iter()
        .map(|l| match x.labels.as_ref().and_then(|m| m.get(l)) {
            Some(label) => LabelRef::Expanded(label.clone()),
            None => LabelRef::Id(*l),
        })
        .collect();
    IssueDto {
        id: i.id.0,
        created_at: ts(i.created_at),
        updated_at: ts(a.updated_at),
        deleted_at: None,
        created_by: user(i.created_by),
        updated_by: user(a.updated_by),
        workspace: i.workspace_id.0,
        project: i.project_id.0,
        name: i.name.clone(),
        description_html: a.description_html.clone(),
        description_binary: None,
        priority: a.priority.into(),
        start_date: a.start_date.map(|d| d.to_string()),
        target_date: a.target_date.map(|d| d.to_string()),
        sequence_id: i.sequence_id,
        sort_order: i.sort_order as f64,
        completed_at: a.completed_at.map(ts),
        archived_at: None,
        is_draft: false,
        state,
        parent: a.parent_id,
        point: a.point,
        estimate_point: None,
        type_: None,
        type_id: None,
        external_source: i.external_source.clone(),
        external_id: i.external_id.clone(),
        assignees,
        labels,
    }
}

/// [`work_item_dto`] as the JSON object a read then cuts to the fields asked for.
fn work_item_body(r: &WorkItemRow, x: &Expansion) -> Result<Map<String, Value>, V1Error> {
    into_object(&work_item_dto(r, x))
}

/// A user as an expanded `assignees` shows them (`UserLiteSerializer`, as the oracle's corpus has it).
async fn user_brief(ctx: &Ctx, id: UserId) -> Result<UserBriefDto, V1Error> {
    let email = ctx.app.ports.db.user_by_id(id).await?.and_then(|u| u.email);
    let profile = ctx.app.ports.db.profile(id).await?;
    let (first, last, display, avatar) =
        profile.map(|p| (p.first_name, p.last_name, p.display_name, p.avatar)).unwrap_or_default();
    Ok(UserBriefDto {
        id: id.0,
        first_name: first,
        last_name: last,
        display_name: display,
        email,
        avatar_url: if avatar.is_empty() { None } else { Some(avatar.clone()) },
        avatar,
    })
}

/// What `expand` asks of these work items, looked up once: `state`, `assignees` and `labels`. The other names the base serializer
/// expands (`project`, `workspace`, `created_by`, `updated_by`, `parent`) are ignored (D-033).
async fn expansion(
    ctx: &Ctx,
    access: &IssueAccess,
    q: &HashMap<String, String>,
    rows: &[&WorkItemRow],
) -> Result<Expansion, V1Error> {
    let wanted: BTreeSet<&str> =
        q.get("expand").map(|e| e.split(',').filter(|s| !s.is_empty()).collect()).unwrap_or_default();
    let (scope, db) = (access.scope(), &ctx.app.ports.db);
    let mut x = Expansion::default();
    if wanted.contains("state") {
        let states = db.project_states(&scope).await?;
        x.state = Some(
            states
                .into_iter()
                .map(|s| (s.id, StateBriefDto { id: s.id, name: s.name, color: s.color, group: s.group.as_str() }))
                .collect(),
        );
    }
    if wanted.contains("labels") {
        let labels = db.project_labels(&scope).await?;
        x.labels = Some(labels.iter().map(|l| (l.id, label_dto(l))).collect());
    }
    if wanted.contains("assignees") {
        let people: BTreeSet<UserId> = rows.iter().flat_map(|r| r.attrs.assignee_ids.iter().copied()).collect();
        let mut briefs = HashMap::new();
        for user in people {
            briefs.insert(user, user_brief(ctx, user).await?);
        }
        x.assignees = Some(briefs);
    }
    Ok(x)
}

/// The body of one work item as a read answers it: expanded as asked, then cut to the fields asked for.
async fn shown(
    ctx: &Ctx,
    access: &IssueAccess,
    q: &HashMap<String, String>,
    row: &WorkItemRow,
) -> Result<Value, V1Error> {
    let x = expansion(ctx, access, q, &[row]).await?;
    Ok(pick_fields(work_item_body(row, &x)?, q))
}

// --- reading a body ------------------------------------------------------------------------------------------------------------

/// What a body says of a work item, read and checked as the pin's `IssueSerializer` does. Each field is `None` when the body does
/// not name it.
#[derive(Default)]
struct Parsed {
    /// The name, and the text it was sent as (the history quotes that, padding and all).
    name: Option<(String, String)>,
    description_html: Option<String>,
    priority: Option<Priority>,
    /// A date, with the text it was sent as (`None` for null).
    start: Option<(Option<String>, Option<NaiveDate>)>,
    target: Option<(Option<String>, Option<NaiveDate>)>,
    parent: Option<Option<Uuid>>,
    state: Option<Option<Uuid>>,
    point: Option<Option<i32>>,
    assignees: Option<Vec<UserId>>,
    labels: Option<Vec<Uuid>>,
    external_source: Option<Option<String>>,
    external_id: Option<Option<String>>,
    /// The body gave `deleted_at` a value: a creation then fails in the pin after it has made the work item (PQ-404).
    deleted_at: bool,
}

/// The pin's refusal when Django raises a `ValidationError` from the view itself, not from a serializer field (a query parameter that
/// names a project by a key it cannot read): the base view's bare 400, and nothing else. A BODY's key is not this: the serializer
/// catches that, per field (`PkRead::Refused`).
fn raised() -> V1Error {
    V1Error::error(StatusCode::BAD_REQUEST, "Please provide valid detail")
}

/// A list of keys as DRF's `ListField` of related fields reads it: the keys with the text each was sent as, or what is wrong, by
/// place.
fn read_key_list(v: &Value) -> Result<Vec<(Uuid, String)>, Value> {
    let Some(items) = v.as_array() else {
        return Err(match v {
            Value::Null => json!(["This field may not be null."]),
            other => json!([format!("Expected a list of items but got type \"{}\".", py_type_name(other))]),
        });
    };
    let (mut keys, mut bad) = (Vec::new(), Map::new());
    for (i, item) in items.iter().enumerate() {
        match pk_field(item) {
            PkRead::Key { id, shown } => keys.push((id, shown)),
            PkRead::Null => {
                bad.insert(i.to_string(), json!(["This field may not be null."]));
            }
            PkRead::Refused(m) => {
                bad.insert(i.to_string(), json!([m]));
            }
        }
    }
    if bad.is_empty() {
        Ok(keys)
    } else {
        Err(Value::Object(bad))
    }
}

/// Python's `repr` of the keys a refusal lists: `[UUID('...'), UUID('...')]`.
fn uuid_list(ids: &[Uuid]) -> String {
    let items: Vec<String> = ids.iter().map(|u| format!("UUID('{u}')")).collect();
    format!("[{}]", items.join(", "))
}

/// A list of related rows' keys (a work item's people and labels), read as DRF's `ListField` of related fields reads one: each item
/// is a key, and what is wrong with an item is filed by its place (`{"0": [message]}`). That a row has the key is the second stage's.
#[derive(Debug, Default)]
struct KeyList(Vec<Key>);

impl KeyList {
    fn ids<T>(self, make: impl Fn(Uuid) -> T) -> Vec<T> {
        self.0.into_iter().map(|key| make(key.id)).collect()
    }
}

impl Field for KeyList {
    type Refusal = Value;

    fn read(v: &Value) -> Result<Self, Value> {
        read_key_list(v).map(|keys| Self(keys.into_iter().map(|(id, shown)| Key { id, shown }).collect()))
    }
}

#[cfg(feature = "openapi")]
impl utoipa::PartialSchema for KeyList {
    fn schema() -> utoipa::openapi::RefOr<utoipa::openapi::Schema> {
        use utoipa::openapi::schema::{ArrayBuilder, KnownFormat, ObjectBuilder, SchemaFormat, Type};
        let uuid =
            ObjectBuilder::new().schema_type(Type::String).format(Some(SchemaFormat::KnownFormat(KnownFormat::Uuid)));
        utoipa::openapi::RefOr::T(utoipa::openapi::Schema::Array(ArrayBuilder::new().items(uuid).build()))
    }
}

#[cfg(feature = "openapi")]
impl utoipa::ToSchema for KeyList {
    fn name() -> std::borrow::Cow<'static, str> {
        std::borrow::Cow::Borrowed("KeyList")
    }
}

/// A work item's priority, read as DRF's choice field over the priorities `Priority` has: the list is `Priority::ALL`, for the parser and
/// for the document alike (written the pin's way, the enum by reference inside an `allOf`: `openapi::priority`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PriorityValue(Priority);

impl Field for PriorityValue {
    type Refusal = String;

    fn read(v: &Value) -> Result<Self, String> {
        choice_of(v, Priority::parse).map(Self)
    }
}

#[cfg(feature = "openapi")]
impl utoipa::PartialSchema for PriorityValue {
    fn schema() -> utoipa::openapi::RefOr<utoipa::openapi::Schema> {
        crate::openapi::priority()
    }
}

#[cfg(feature = "openapi")]
impl utoipa::ToSchema for PriorityValue {
    fn name() -> std::borrow::Cow<'static, str> {
        std::borrow::Cow::Borrowed("PriorityValue")
    }
}

request! {
    #[schema(description = "What a work item's change takes: the creation's keys, none required.")]
    /// What a body says of a work item, as the pin's `IssueSerializer` reads it: every field it knows is read into this, and what is wrong
    /// is one 400. The pin names a body whose keys may all be left out `Patched…`, and this is that: a change reads into it as it is, and a
    /// creation reads into it and must also name [`PatchedIssueRequest::REQUIRED`]. A key left out is `None`. Each field's type says what
    /// it takes, and the document is written from these types (the schema `PatchedIssueRequest` is this struct, and `Creations` in
    /// `openapi.rs` makes `IssueRequest` of it with `REQUIRED`), so the bounds below are the only place they are written. The route reads
    /// every key the serializer has and keeps some of them (D-033 (1)); the others are read, checked and not kept. `priority` and the lists
    /// are not `null`; a date, a key and the relations can be.
    pub(crate) struct PatchedIssueRequest {
        assignees: KeyList,
        labels: KeyList,
        type_ as "type": NullableKey,
        type_id: NullableKey,
        estimate_point: NullableKey,
        name: SentText<1, { thrust_core::MAX_NAME_CHARS }>,
        description_html: Prose,
        priority: PriorityValue,
        start_date: NullableDate,
        target_date: NullableDate,
        #[allow(dead_code, reason = "read, checked, and not kept (D-033 (1)): the document says it and the pin reads it")]
        sequence_id: Whole<{ i32::MIN }, { i32::MAX }>,
        #[allow(dead_code, reason = "read, checked, and not kept (D-033 (1)): the document says it and the pin reads it")]
        sort_order: f64,
        #[allow(dead_code, reason = "read, checked, and not kept (D-033 (1)): the document says it and the pin reads it")]
        archived_at: NullableDate,
        #[allow(dead_code, reason = "read, checked, and not kept (D-033 (1)): the document says it and the pin reads it")]
        is_draft: bool,
        external_source: NullableText<0, 255>,
        external_id: NullableText<0, 255>,
        point: NullableWhole<0, { thrust_contracts::MAX_WORK_ITEM_POINT }>,
        deleted_at: NullableDateTime,
        created_by: NullableKey,
        parent: NullableKey,
        state: NullableKey,
    }
}

impl PatchedIssueRequest {
    /// The keys a creation must name; a change names none of them. The document's creation is written from this list, so it is the
    /// parser's.
    pub(crate) const REQUIRED: [&'static str; 1] = ["name"];

    /// The first stage: read every key the body names, with no database, and file what is wrong. The errors are handed back and not
    /// refused, because the second stage ([`Self::check_rows`]) adds its own to the same 400, as the pin reports every field at once.
    fn read(b: &Map<String, Value>, partial: bool) -> (Self, FieldErrors) {
        let mut errors = FieldErrors::new();
        for field in Self::REQUIRED {
            if !partial && !b.contains_key(field) {
                fail(&mut errors, field, "This field is required.");
            }
        }
        let request = PatchedIssueRequest::read_fields(&mut errors, b);
        (request, errors)
    }

    /// The second stage: the keys that name a row. A person must be a user that is there, a label one of the project's, a parent a work
    /// item of the project and a state one of its states, "or not there", as for a row that does not exist (D-033: the pin tells the two
    /// apart, which tells a caller what exists in projects it cannot see); there are no types and no estimate points here, so a key
    /// for either names a row that is not there. A list is refused by the place of the item that is wrong.
    async fn check_rows(&self, ctx: &Ctx, access: &IssueAccess, errors: &mut FieldErrors) -> Result<(), V1Error> {
        let (scope, db) = (access.scope(), &ctx.app.ports.db);
        if let Some(list) = &self.assignees {
            let mut bad = Map::new();
            for (i, key) in list.0.iter().enumerate() {
                if db.user_by_id(UserId(key.id)).await?.is_none() {
                    bad.insert(i.to_string(), json!([no_such_pk_shown(&key.shown)]));
                }
            }
            if !bad.is_empty() {
                errors.insert("assignees".into(), Value::Object(bad));
            }
        }
        if let Some(list) = &self.labels {
            let known = db.project_labels(&scope).await?;
            let mut bad = Map::new();
            for (i, key) in list.0.iter().enumerate() {
                if !known.iter().any(|l| l.id == key.id) {
                    bad.insert(i.to_string(), json!([no_such_pk_shown(&key.shown)]));
                }
            }
            if !bad.is_empty() {
                errors.insert("labels".into(), Value::Object(bad));
            }
        }
        // the pin reads a type under two names, the model's own `type` and the serializer's `type_id` (both measured: each answers under
        // its own name)
        for (field, key) in
            [("type", &self.type_), ("type_id", &self.type_id), ("estimate_point", &self.estimate_point)]
        {
            if let Some(key) = key.as_ref().and_then(NullableKey::key) {
                fail(errors, field, no_such_pk_shown(&key.shown));
            }
        }
        if let Some(key) = self.created_by.as_ref().and_then(NullableKey::key) {
            if db.user_by_id(UserId(key.id)).await?.is_none() {
                fail(errors, "created_by", no_such_pk_shown(&key.shown));
            }
        }
        if let Some(key) = self.parent.as_ref().and_then(NullableKey::key) {
            if db.work_item_row(&scope, WorkItemId(key.id)).await?.is_none() {
                fail(errors, "parent", no_such_pk_shown(&key.shown));
            }
        }
        if let Some(key) = self.state.as_ref().and_then(NullableKey::key) {
            if !db.project_states(&scope).await?.iter().any(|s| s.id == key.id) {
                fail(errors, "state", no_such_pk_shown(&key.shown));
            }
        }
        Ok(())
    }

    /// The third stage, the serializer's `validate()`, which runs only when every field was fine and raises at its first refusal; then
    /// what the route keeps of the body.
    async fn validate(self, ctx: &Ctx, access: &IssueAccess) -> Result<Parsed, V1Error> {
        let (scope, db) = (access.scope(), &ctx.app.ports.db);
        let date = |d: &Option<NullableDate>| d.as_ref().and_then(NullableDate::date);
        if let (Some(start), Some(target)) = (date(&self.start_date), date(&self.target_date)) {
            if start > target {
                return Err(non_field("Start date cannot exceed target date"));
            }
        }
        let mut description_html = None;
        if let Some(prose) = &self.description_html {
            let sent = prose.as_str();
            // `html.fromstring("")` has no document to read
            if sent.is_empty() {
                return Err(non_field("Invalid HTML passed"));
            }
            match thrust_editor_doc::sanitize::validate_html_content(sent) {
                Ok(clean) => description_html = Some(clean.unwrap_or_else(|| sent.to_string())),
                Err(_) => {
                    let mut errors = FieldErrors::new();
                    fail(&mut errors, "error", "html content is not valid");
                    return Err(V1Error::Fields(errors));
                }
            }
        }
        if let Some(list) = &self.assignees {
            let members = db.project_members(&scope).await?;
            let invalid: Vec<Uuid> = list
                .0
                .iter()
                .map(|key| UserId(key.id))
                .filter(|u| !members.iter().any(|m| m.user_id == *u && m.role >= Role::Member))
                .map(|u| u.0)
                .collect();
            if !invalid.is_empty() {
                return Err(non_field(&format!(
                    "Assignees {} are not active members of this project",
                    uuid_list(&invalid)
                )));
            }
        }
        Ok(Parsed {
            name: self.name.map(SentText::into_parts),
            description_html,
            priority: self.priority.map(|p| p.0),
            start: self.start_date.map(NullableDate::into_parts),
            target: self.target_date.map(NullableDate::into_parts),
            parent: self.parent.map(NullableKey::into_id),
            state: self.state.map(NullableKey::into_id),
            point: self.point.map(NullableWhole::get),
            assignees: self.assignees.map(|list| list.ids(UserId)),
            labels: self.labels.map(|list| list.ids(|id| id)),
            external_source: self.external_source.map(NullableText::into_option),
            external_id: self.external_id.map(NullableText::into_option),
            deleted_at: self.deleted_at.is_some_and(NullableDateTime::is_set),
        })
    }
}

/// The creation's body in the document: `PatchedIssueRequest` with `name` required, which `Creations` (`openapi.rs`) writes from the
/// change's schema. The type is the name the path declarations use for it.
#[cfg(feature = "openapi")]
#[derive(utoipa::ToSchema)]
#[allow(dead_code, reason = "a name for the document: nothing builds it")]
pub(crate) struct IssueRequest;

/// Read a body the way the pin's `IssueSerializer` does: every field it knows is checked, and what is wrong is one 400 (the fields in
/// the first place, then the rows they name, then `validate()`'s refusals, one at a time); a key Django cannot read is
/// `“x” is not a valid UUID.` under its field, with the other fields' errors beside it (measured on the pin).
async fn read_item(ctx: &Ctx, access: &IssueAccess, b: &Map<String, Value>, partial: bool) -> Result<Parsed, V1Error> {
    let (request, mut errors) = PatchedIssueRequest::read(b, partial);
    request.check_rows(ctx, access, &mut errors).await?;
    refuse_if_any(errors)?;
    request.validate(ctx, access).await
}

/// The 409 for an integration key that another work item has, naming `id`.
fn key_taken(id: WorkItemId) -> V1Error {
    V1Error::Error(
        StatusCode::CONFLICT,
        json!({ "error": "Issue with the same external id and external source already exists", "id": id.0 }),
    )
}

/// A text the pin would call true: one that is not empty.
fn truthy(v: &Option<Option<String>>) -> Option<&str> {
    v.as_ref().and_then(|v| v.as_deref()).filter(|s| !s.is_empty())
}

// --- the routes ----------------------------------------------------------------------------------------------------------------

/// The order of a list (the pin's allowlist; a name outside it is the default, `-created_at`). The pin's `priority` runs from
/// `urgent` and `-priority` from `none`, which the adapters have the other way about, and its `state__name` and `state__group` both
/// order by the state's group; the orders by the least of a list (`labels__name`, `assignees__first_name`, the modules) are not built
/// and are the default (D-033). `archived_at` is in the allowlist and is not here: a list is of the work items that are not archived,
/// so it has nothing to put in another order, and the default is what is left when every value is null.
fn order_of(q: &HashMap<String, String>) -> WorkItemOrder {
    let default = WorkItemOrder { field: WorkItemOrderField::CreatedAt, descending: true };
    let raw = q.get("order_by").map_or("-created_at", String::as_str);
    let (descending, name) = raw.strip_prefix('-').map_or((false, raw), |n| (true, n));
    let plain = |field| WorkItemOrder { field, descending };
    match name {
        "created_at" => plain(WorkItemOrderField::CreatedAt),
        "updated_at" => plain(WorkItemOrderField::UpdatedAt),
        "sequence_id" => plain(WorkItemOrderField::SequenceId),
        "sort_order" => plain(WorkItemOrderField::SortOrder),
        "target_date" => plain(WorkItemOrderField::TargetDate),
        "start_date" => plain(WorkItemOrderField::StartDate),
        "completed_at" => plain(WorkItemOrderField::CompletedAt),
        "priority" => WorkItemOrder { field: WorkItemOrderField::Priority, descending: !descending },
        "state__name" | "state__group" => plain(WorkItemOrderField::StateGroup),
        _ => default,
    }
}

/// `GET .../{family}/`: a page of the project's work items; or, when the request names an integration key (both halves), the one
/// work item that has it, not a page.
///
/// The two families of paths (`issues/` and `work-items/`) share every handler here, so the attributes below describe the `issues/`
/// ones, and the document derives the `work-items/` ones from them (`openapi::WorkItemsTwice`, the pin's operation ids ending `_2`).
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        get,
        path = "/api/v1/workspaces/{slug}/projects/{project_id}/issues/",
        operation_id = "list_work_items",
        tag = "Work Items",
        summary = "List work items",
        params(("slug" = String, Path), ("project_id" = thrust_contracts::Uuid, Path), crate::openapi::WorkItemsQuery),
        responses(
            crate::openapi::Refused,
            (status = 200, description = "Paginated list of work items", body = Listing<IssueDto>),
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
) -> Result<Json<Listing<Value>>, V1Error> {
    let access = reader(&ctx, &slug, ProjectId(project.0)).await?;
    let q = query_map(raw.as_deref());
    let unsupported: Vec<&str> =
        ["pql", "filters"].into_iter().filter(|p| q.get(*p).is_some_and(|v| !v.is_empty())).collect();
    if !unsupported.is_empty() {
        return Err(V1Error::Error(
            StatusCode::BAD_REQUEST,
            json!({
                "pql": "PQL and structured filters are not supported on this Plane edition. Remove the pql/filters parameter and filter results client-side, or use a Plane edition that supports work item query filtering.",
                "unsupported_parameters": unsupported,
            }),
        ));
    }
    let given = |name: &str| q.get(name).map(String::as_str).filter(|s| !s.is_empty());
    if let (Some(id), Some(source)) = (given("external_id"), given("external_source")) {
        let row = thrust_core::issue_by_external(&ctx.app, &access, source, id).await.map_err(item_error)?;
        return Ok(Json(Listing::One(shown(&ctx, &access, &q, &row).await?)));
    }
    let plan =
        PageRequest::parse(q.get("per_page").map(String::as_str), q.get("cursor").map(String::as_str))?.plan()?;
    let query = IssueListQuery {
        order: order_of(&q),
        limit: u32::try_from(plan.limit).unwrap_or(u32::MAX),
        page: u32::try_from(plan.page).unwrap_or(u32::MAX),
        updated_after: None,
    };
    let page = thrust_core::list_issues(&ctx.app, &access, query).await.map_err(item_error)?;
    let rows: Vec<&WorkItemRow> = page.rows.iter().collect();
    let x = expansion(&ctx, &access, &q, &rows).await?;
    let results =
        page.rows.iter().map(|r| Ok(pick_fields(work_item_body(r, &x)?, &q))).collect::<Result<Vec<_>, V1Error>>()?;
    Ok(Json(Listing::Page(plan.page(results, usize::try_from(page.total).unwrap_or(usize::MAX)))))
}

/// `POST .../{family}/`: make a work item. The pin answers 201; a body that gives `deleted_at` a value makes the work item and then
/// fails (PQ-404), which here makes none.
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        post,
        path = "/api/v1/workspaces/{slug}/projects/{project_id}/issues/",
        operation_id = "create_work_item",
        tag = "Work Items",
        summary = "Create work item",
        params(("slug" = String, Path), ("project_id" = thrust_contracts::Uuid, Path)),
        request_body(content = IssueRequest, content_type = "application/json"),
        responses(
            crate::openapi::Refused,
            (status = 201, description = "Work Item created successfully", body = IssueDto),
            (status = 400, description = "Invalid request data provided"),
            (status = 404, description = "Project not found"),
            (status = 409, description = "Resource with same external ID already exists"),
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
    let access = writer(&ctx, &slug, ProjectId(project.0)).await?;
    let map = match json_body(&headers, &body)? {
        Body::Object(map) => map,
        Body::Array => return Err(not_a_dictionary()),
    };
    let p = read_item(&ctx, &access, &map, false).await?;
    if let (Some(source), Some(id)) = (truthy(&p.external_source), truthy(&p.external_id)) {
        if let Ok(taken) = thrust_core::issue_by_external(&ctx.app, &access, source, id).await {
            return Err(key_taken(taken.item.id));
        }
    }
    if p.deleted_at {
        return Err(V1Error::internal(
            "a deleted_at in the body: the pin creates the item soft-deleted, then raises (PQ-404)",
        ));
    }
    let draft = IssueDraft {
        name: p.name.map(|(_, name)| name).unwrap_or_default(),
        description_html: p.description_html,
        priority: p.priority,
        state_id: p.state.flatten(),
        parent_id: p.parent.flatten(),
        start_date: p.start.and_then(|(_, d)| d),
        target_date: p.target.and_then(|(_, d)| d),
        assignee_ids: p.assignees.unwrap_or_default(),
        label_ids: p.labels.unwrap_or_default(),
        point: p.point.flatten(),
        external_source: p.external_source.flatten().filter(|s| !s.is_empty()),
        external_id: p.external_id.flatten().filter(|s| !s.is_empty()),
    };
    let row = thrust_core::create_issue(&ctx.app, &access, draft).await.map_err(item_error)?;
    let x = expansion(&ctx, &access, &HashMap::new(), &[&row]).await?;
    Ok((StatusCode::CREATED, Json(Value::Object(work_item_body(&row, &x)?))))
}

/// `GET .../{family}/{pk}/`. The pin's schema lists `external_id`, `external_source` and `order_by` for it too, so the document does; this
/// route reads `fields` and `expand` only, and a request that sends the others is answered as one that does not (a test says so).
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        get,
        path = "/api/v1/workspaces/{slug}/projects/{project_id}/issues/{pk}/",
        operation_id = "retrieve_work_item",
        tag = "Work Items",
        summary = "Retrieve work item",
        params(
            ("slug" = String, Path),
            ("project_id" = thrust_contracts::Uuid, Path),
            ("pk" = thrust_contracts::Uuid, Path),
            crate::openapi::WorkItemQuery
        ),
        responses(
            crate::openapi::Refused,
            (status = 200, description = "List of issues or issue details", body = IssueDto),
            (status = 400, description = "Invalid request data provided"),
            (status = 404, description = "Work item not found"),
        ),
        security(("ApiKeyAuthentication" = []))
    )
)]
pub(crate) async fn retrieve(
    V1Path((slug, project, pk)): V1Path<(String, PlaneUuid, PlaneUuid)>,
    ctx: Ctx,
    RawQuery(raw): RawQuery,
) -> Result<Json<Value>, V1Error> {
    let access = reader(&ctx, &slug, ProjectId(project.0)).await?;
    let q = query_map(raw.as_deref());
    let row = thrust_core::get_issue(&ctx.app, &access, WorkItemId(pk.0)).await.map_err(item_error)?;
    Ok(Json(shown(&ctx, &access, &q, &row).await?))
}

/// `PATCH .../{family}/{pk}/`. The base view parses the body before the handler runs, so a body that cannot be parsed is refused
/// before the work item is looked for; the handler then finds it (as `Issue.objects` does, an archived project's too), and only then
/// does the serializer read what the parsed body says.
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        patch,
        path = "/api/v1/workspaces/{slug}/projects/{project_id}/issues/{pk}/",
        operation_id = "update_work_item",
        tag = "Work Items",
        summary = "Partially update work item",
        params(
            ("slug" = String, Path),
            ("project_id" = thrust_contracts::Uuid, Path),
            ("pk" = thrust_contracts::Uuid, Path)
        ),
        // not required: an empty body is an empty object (`json_body`), as DRF reads it
        request_body(content = Option<PatchedIssueRequest>, content_type = "application/json"),
        responses(
            crate::openapi::Refused,
            (status = 200, description = "Work Item patched successfully", body = IssueDto),
            (status = 400, description = "Invalid request data provided"),
            (status = 404, description = "Work item not found"),
            (status = 409, description = "Resource with same external ID already exists"),
        ),
        security(("ApiKeyAuthentication" = []))
    )
)]
pub(crate) async fn update(
    V1Path((slug, project, pk)): V1Path<(String, PlaneUuid, PlaneUuid)>,
    ctx: Ctx,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<Value>, V1Error> {
    let access = writer(&ctx, &slug, ProjectId(project.0)).await?;
    let parsed = json_body(&headers, &body)?;
    let before = thrust_core::issue_to_change(&ctx.app, &access, WorkItemId(pk.0)).await.map_err(item_error)?;
    let map = match parsed {
        Body::Object(map) => map,
        Body::Array => return Err(not_a_dictionary()),
    };
    let p = read_item(&ctx, &access, &map, true).await?;
    // a key the item does not have already, with the source named or else its own, that another has
    if let Some(id) = truthy(&p.external_id).filter(|id| before.item.external_id.as_deref() != Some(*id)) {
        let source = match &p.external_source {
            Some(named) => named.clone(),
            None => before.item.external_source.clone(),
        };
        if let Some(source) = source {
            if thrust_core::issue_by_external(&ctx.app, &access, &source, id).await.is_ok() {
                return Err(key_taken(before.item.id));
            }
        }
    }
    let mut asks = Vec::new();
    if let Some((sent, name)) = p.name {
        asks.push(Ask::Name { sent, name });
    }
    if let Some(html) = p.description_html {
        asks.push(Ask::Description(html));
    }
    if let Some(priority) = p.priority {
        asks.push(Ask::Priority(priority));
    }
    if let Some(state) = p.state {
        asks.push(Ask::State(state));
    }
    if let Some((sent, date)) = p.start {
        asks.push(Ask::Start { sent, date });
    }
    if let Some((sent, date)) = p.target {
        asks.push(Ask::Target { sent, date });
    }
    if let Some(parent) = p.parent {
        asks.push(Ask::Parent(parent));
    }
    if let Some(point) = p.point {
        asks.push(Ask::Point(point));
    }
    if let Some(source) = p.external_source {
        asks.push(Ask::ExternalSource(source.filter(|s| !s.is_empty())));
    }
    if let Some(id) = p.external_id {
        asks.push(Ask::ExternalId(id.filter(|s| !s.is_empty())));
    }
    if let Some(people) = p.assignees {
        asks.push(Ask::Assignees(people));
    }
    if let Some(labels) = p.labels {
        asks.push(Ask::Labels(labels));
    }
    let id = before.item.id;
    thrust_core::update_issue(&ctx.app, &access, before, IssueUpdate { asks, silent: false })
        .await
        .map_err(item_error)?;
    let row = thrust_core::issue_to_change(&ctx.app, &access, id).await.map_err(item_error)?;
    let x = expansion(&ctx, &access, &HashMap::new(), &[&row]).await?;
    Ok(Json(Value::Object(work_item_body(&row, &x)?)))
}

/// `DELETE .../{family}/{pk}/`: a project admin or the work item's maker.
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        delete,
        path = "/api/v1/workspaces/{slug}/projects/{project_id}/issues/{pk}/",
        operation_id = "delete_work_item",
        tag = "Work Items",
        summary = "Delete work item",
        params(
            ("slug" = String, Path),
            ("project_id" = thrust_contracts::Uuid, Path),
            ("pk" = thrust_contracts::Uuid, Path)
        ),
        responses(
            (status = 204, description = "Resource deleted successfully"),
            (status = 401, description = "Authentication credentials were not provided or are invalid."),
            (status = 403, description = "Only admin or creator can perform this action"),
            (status = 404, description = "Work item not found"),
        ),
        security(("ApiKeyAuthentication" = []))
    )
)]
pub(crate) async fn destroy(
    V1Path((slug, project, pk)): V1Path<(String, PlaneUuid, PlaneUuid)>,
    ctx: Ctx,
) -> Result<StatusCode, V1Error> {
    let access = writer(&ctx, &slug, ProjectId(project.0)).await?;
    thrust_core::delete_issue(&ctx.app, &access, WorkItemId(pk.0)).await.map_err(item_error)?;
    Ok(StatusCode::NO_CONTENT)
}

/// `GET /workspaces/{slug}/{family}/search/`: the work items of the caller's own projects that a name, a project identifier or a
/// number says. Any key may ask (the pin's permission is only that the caller is authenticated): a caller who is in no project of the
/// workspace gets no work items, not a refusal.
///
/// The pin's schema describes one hit as the whole answer (`IssueSearch`, with a `sequence_id` that is a string); what it answers is
/// `{"issues": [hit, ...]}` with a whole number (the oracle's corpus, `search`), which is what this document says.
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        get,
        path = "/api/v1/workspaces/{slug}/issues/search/",
        operation_id = "search_work_items",
        tag = "Work Items",
        params(("slug" = String, Path), crate::openapi::SearchQuery),
        responses(
            crate::openapi::Refused,
            (status = 200, description = "Work item search results", body = IssueSearchDto),
            (status = 400, description = "Bad request - invalid search parameters"),
            (status = 404, description = "Workspace not found"),
        ),
        security(("ApiKeyAuthentication" = []))
    )
)]
pub(crate) async fn search(
    V1Path(slug): V1Path<String>,
    ctx: Ctx,
    RawQuery(raw): RawQuery,
) -> Result<Json<IssueSearchDto>, V1Error> {
    ctx.throttle().await?;
    let q = query_map(raw.as_deref());
    let text = q.get("search").cloned().unwrap_or_default();
    let none = || Json(IssueSearchDto { issues: Vec::new() });
    if text.is_empty() {
        return Ok(none());
    }
    // `int(limit)` raises on text, and a negative slice is refused by the ORM: both are the pin's 500
    let limit = match q.get("limit") {
        None => 10,
        Some(s) => py_int(s).and_then(|n| u32::try_from(n).ok()).ok_or_else(|| {
            V1Error::internal(
                "search limit is not a non-negative integer: `int()` raises or the slice is negative (PQ-408)",
            )
        })?,
    };
    let workspace_search = q.get("workspace_search").map_or("false", String::as_str);
    let project = match q.get("project_id").filter(|p| !p.is_empty()) {
        Some(p) if workspace_search == "false" => Some(ProjectId(parse_uuid_text(p).ok_or_else(raised)?)),
        _ => None,
    };
    let scope = match thrust_core::authorize_workspace(&ctx.app, &ctx.principal, &WorkspaceSlug::new(&slug)).await {
        Ok(scope) => scope,
        Err(thrust_core::CoreError::Forbidden | thrust_core::CoreError::NotFound) => return Ok(none()),
        Err(other) => return Err(other.into()),
    };
    let search = WorkspaceSearch { text: text.clone(), numbers: whole_numbers(&text), project, limit };
    let hits = thrust_core::search_issues(&ctx.app, &scope, search).await.map_err(item_error)?;
    let issues = hits
        .into_iter()
        .map(|h| IssueSearchItemDto {
            name: h.name,
            id: h.id.0,
            sequence_id: h.sequence_id,
            project_identifier: h.project_identifier,
            project_id: h.project_id.0,
            workspace_slug: h.workspace_slug.0,
        })
        .collect();
    Ok(Json(IssueSearchDto { issues }))
}

/// `GET /workspaces/{slug}/{family}/{KEY}-{n}/`: a work item by the identifier of its project and its number. The route splits at the
/// last hyphen; the permission is a membership of a project of the workspace that has the identifier (a stranger, or an identifier
/// that is nobody's, is the refusal of a permission); a number that is not a whole number is a 404 of its own words. `fields` and
/// `expand` are read as on a read by id, though the pin's schema declares no query for it, so the document does.
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        get,
        path = "/api/v1/workspaces/{slug}/issues/{project_identifier}-{issue_identifier}/",
        operation_id = "get_workspace_work_item",
        tag = "Work Items",
        summary = "Retrieve work item by identifiers",
        params(crate::openapi::ByIdentifierPath, crate::openapi::ByIdentifierQuery),
        responses(
            crate::openapi::Refused,
            (status = 200, description = "Work item details", body = IssueDto),
            (status = 404, description = "Work item not found"),
        ),
        security(("ApiKeyAuthentication" = []))
    )
)]
pub(crate) async fn by_identifier(
    V1Path((slug, ident)): V1Path<(String, String)>,
    ctx: Ctx,
    RawQuery(raw): RawQuery,
) -> Result<Json<Value>, V1Error> {
    let Some((key, number)) = ident.rsplit_once('-') else { return Err(V1Error::PageNotFound) };
    if key.is_empty() || number.is_empty() {
        return Err(V1Error::PageNotFound);
    }
    let workspace = thrust_core::authorize_workspace(&ctx.app, &ctx.principal, &WorkspaceSlug::new(&slug))
        .await
        .map_err(|_| V1Error::PermissionDenied)?;
    let listings = ctx.app.ports.db.visible_projects(&workspace).await?;
    let project = listings
        .iter()
        .find(|l| l.project.identifier == key && l.role.is_some())
        .map(|l| l.project.id)
        .ok_or(V1Error::PermissionDenied)?;
    let access = reader(&ctx, &slug, project).await?;
    let q = query_map(raw.as_deref());
    if !number.bytes().all(|b| b.is_ascii_digit()) {
        return Err(V1Error::error(StatusCode::NOT_FOUND, "Work item not found"));
    }
    let sequence = number.parse::<i64>().map_err(|_| V1Error::NotFound)?;
    let row = thrust_core::issue_by_sequence(&ctx.app, &access, sequence).await.map_err(item_error)?;
    Ok(Json(shown(&ctx, &access, &q, &row).await?))
}
