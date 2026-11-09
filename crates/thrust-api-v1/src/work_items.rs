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

use crate::fields::{
    bool_field, char_field, choice_field, date_field, datetime_field, fail, float_field, int_field, no_such_pk_shown,
    not_a_dictionary, parse_uuid_text, pk_field, py_int, py_str, py_type_name, refuse_if_any, whole_numbers,
    FieldErrors, PkRead,
};
use crate::pagination::PageRequest;
use crate::projects::ts;
use crate::request::{json_body, pick_fields, query_map, Body};
use crate::{Ctx, PlaneUuid, V1Error, V1Path};

const PRIORITIES: [&str; 5] = ["urgent", "high", "medium", "low", "none"];

pub(crate) fn routes() -> Router {
    let mut r = Router::new();
    for family in ["issues", "work-items"] {
        r = r
            .route(&format!("/api/v1/workspaces/{{slug}}/projects/{{project_id}}/{family}/"), get(list).post(create))
            .route(
                &format!("/api/v1/workspaces/{{slug}}/projects/{{project_id}}/{family}/{{pk}}/"),
                get(retrieve).patch(update).delete(destroy),
            )
            // `search/` is a literal and wins over the identifier below, as Django's route order makes it
            .route(&format!("/api/v1/workspaces/{{slug}}/{family}/search/"), get(search))
            .route(&format!("/api/v1/workspaces/{{slug}}/{family}/{{ident}}/"), get(by_identifier));
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
    state: Option<HashMap<Uuid, Value>>,
    assignees: Option<HashMap<UserId, Value>>,
    labels: Option<HashMap<Uuid, Value>>,
}

/// `Issue`'s body as the pin's `IssueSerializer` writes it: 29 keys. A work item that is read is live, so it is not a draft, not
/// archived and not deleted; it has no estimate point and no type.
fn work_item_body(r: &WorkItemRow, x: &Expansion) -> Map<String, Value> {
    let (i, a) = (&r.item, &r.attrs);
    let user = |u: Option<UserId>| u.map_or(Value::Null, |u| json!(u.0));
    let state = match (a.state_id, &x.state) {
        (Some(id), Some(map)) => map.get(&id).cloned().unwrap_or_else(|| json!(id)),
        (state, _) => json!(state),
    };
    let assignees: Vec<Value> = a
        .assignee_ids
        .iter()
        .map(|u| x.assignees.as_ref().and_then(|m| m.get(u).cloned()).unwrap_or_else(|| json!(u.0)))
        .collect();
    let labels: Vec<Value> = a
        .label_ids
        .iter()
        .map(|l| x.labels.as_ref().and_then(|m| m.get(l).cloned()).unwrap_or_else(|| json!(l)))
        .collect();
    let body = json!({
        "id": i.id.0,
        "created_at": ts(i.created_at),
        "updated_at": ts(a.updated_at),
        "deleted_at": null,
        "created_by": user(i.created_by),
        "updated_by": user(a.updated_by),
        "workspace": i.workspace_id.0,
        "project": i.project_id.0,
        "name": i.name,
        "description_html": a.description_html,
        "description_binary": null,
        "priority": a.priority.as_str(),
        "start_date": a.start_date.map(|d| d.to_string()),
        "target_date": a.target_date.map(|d| d.to_string()),
        "sequence_id": i.sequence_id,
        "sort_order": i.sort_order as f64,
        "completed_at": a.completed_at.map(ts),
        "archived_at": null,
        "is_draft": false,
        "state": state,
        "parent": a.parent_id,
        "point": a.point,
        "estimate_point": null,
        "type": null,
        "type_id": null,
        "external_source": i.external_source,
        "external_id": i.external_id,
        "assignees": assignees,
        "labels": labels,
    });
    match body {
        Value::Object(map) => map,
        _ => Map::new(),
    }
}

/// A user as an expanded `assignees` shows them (`UserLiteSerializer`, as the oracle's corpus has it).
async fn user_brief(ctx: &Ctx, id: UserId) -> Result<Value, V1Error> {
    let email = ctx.app.ports.db.user_by_id(id).await?.and_then(|u| u.email);
    let profile = ctx.app.ports.db.profile(id).await?;
    let (first, last, display, avatar) =
        profile.map(|p| (p.first_name, p.last_name, p.display_name, p.avatar)).unwrap_or_default();
    Ok(json!({
        "id": id.0,
        "first_name": first,
        "last_name": last,
        "display_name": display,
        "email": email,
        "avatar": avatar,
        "avatar_url": if avatar.is_empty() { Value::Null } else { json!(avatar) },
    }))
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
                .map(|s| (s.id, json!({ "id": s.id, "name": s.name, "color": s.color, "group": s.group.as_str() })))
                .collect(),
        );
    }
    if wanted.contains("labels") {
        let labels = db.project_labels(&scope).await?;
        x.labels = Some(labels.iter().map(|l| (l.id, Value::Object(crate::labels::label_body(l)))).collect());
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
    Ok(pick_fields(work_item_body(row, &x), q))
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

/// The pin's refusal when Django raises from inside a relation field (a key it cannot read): a bare 400 and nothing else.
fn raised() -> V1Error {
    V1Error::error(StatusCode::BAD_REQUEST, "Please provide valid detail")
}

/// A list of keys as DRF's `ListField` of related fields reads it: the keys with the text each was sent as, or what is wrong, by
/// place. `Err(None)` is a key Django raises on.
fn read_key_list(v: &Value) -> Result<Vec<(Uuid, String)>, Option<Value>> {
    let Some(items) = v.as_array() else {
        return Err(Some(match v {
            Value::Null => json!(["This field may not be null."]),
            other => json!([format!("Expected a list of items but got type \"{}\".", py_type_name(other))]),
        }));
    };
    let (mut keys, mut bad, mut raises) = (Vec::new(), Map::new(), false);
    for (i, item) in items.iter().enumerate() {
        match pk_field(item) {
            PkRead::Key { id, shown } => keys.push((id, shown)),
            PkRead::Null => {
                bad.insert(i.to_string(), json!(["This field may not be null."]));
            }
            PkRead::Refused(m) => {
                bad.insert(i.to_string(), json!([m]));
            }
            PkRead::Raises => raises = true,
        }
    }
    match (raises, bad.is_empty()) {
        (true, _) => Err(None),
        (false, true) => Ok(keys),
        (false, false) => Err(Some(Value::Object(bad))),
    }
}

/// Python's `repr` of the keys a refusal lists: `[UUID('...'), UUID('...')]`.
fn uuid_list(ids: &[Uuid]) -> String {
    let items: Vec<String> = ids.iter().map(|u| format!("UUID('{u}')")).collect();
    format!("[{}]", items.join(", "))
}

/// Read a body the way the pin's `IssueSerializer` does: every field it knows is checked, and what is wrong is one 400 (the fields in
/// the first place, then `validate()`'s refusals, one at a time); a key Django cannot read makes the whole answer a bare 400. A key
/// that names a state, a label or a parent that is not one of the project's is "not there", as for a row that does not exist
/// (D-033: the pin tells the two apart, which tells a caller what exists in projects it cannot see).
async fn read_item(ctx: &Ctx, access: &IssueAccess, b: &Map<String, Value>, partial: bool) -> Result<Parsed, V1Error> {
    let (scope, db) = (access.scope(), &ctx.app.ports.db);
    let mut errors = FieldErrors::new();
    let mut raises = false;
    let mut p = Parsed::default();
    let text = |max: Option<usize>, blank: bool, null: bool| move |v: &Value| char_field(v, max, blank, null);

    // the declared fields: the people, the labels and the type
    if let Some(v) = b.get("assignees") {
        match read_key_list(v) {
            Ok(keys) => {
                let mut bad = Map::new();
                let mut people = Vec::new();
                for (i, (id, shown)) in keys.iter().enumerate() {
                    if db.user_by_id(UserId(*id)).await?.is_none() {
                        bad.insert(i.to_string(), json!([no_such_pk_shown(shown)]));
                    } else {
                        people.push(UserId(*id));
                    }
                }
                if bad.is_empty() {
                    p.assignees = Some(people);
                } else {
                    errors.insert("assignees".into(), Value::Object(bad));
                }
            }
            Err(Some(e)) => {
                errors.insert("assignees".into(), e);
            }
            Err(None) => raises = true,
        }
    }
    if let Some(v) = b.get("labels") {
        match read_key_list(v) {
            Ok(keys) => {
                let known = db.project_labels(&scope).await?;
                let mut bad = Map::new();
                let mut labels = Vec::new();
                for (i, (id, shown)) in keys.iter().enumerate() {
                    if known.iter().any(|l| l.id == *id) {
                        labels.push(*id);
                    } else {
                        bad.insert(i.to_string(), json!([no_such_pk_shown(shown)]));
                    }
                }
                if bad.is_empty() {
                    p.labels = Some(labels);
                } else {
                    errors.insert("labels".into(), Value::Object(bad));
                }
            }
            Err(Some(e)) => {
                errors.insert("labels".into(), e);
            }
            Err(None) => raises = true,
        }
    }
    // there are no types and no estimate points here: a key names a row that is not there
    for field in ["type_id", "estimate_point"] {
        if let Some(v) = b.get(field) {
            match pk_field(v) {
                PkRead::Null => {}
                PkRead::Key { shown, .. } => fail(&mut errors, field, no_such_pk_shown(&shown)),
                PkRead::Refused(m) => fail(&mut errors, field, m),
                PkRead::Raises => raises = true,
            }
        }
    }

    // the model's fields
    if !partial && !b.contains_key("name") {
        fail(&mut errors, "name", "This field is required.");
    }
    if let Some(v) = b.get("name") {
        match char_field(v, Some(255), false, false) {
            Ok(name) => p.name = Some((py_str(v), name.unwrap_or_default())),
            Err(m) => fail(&mut errors, "name", m),
        }
    }
    let html = b.get("description_html").map(|v| char_field(v, None, true, false));
    match &html {
        Some(Ok(t)) => p.description_html = Some(t.clone().unwrap_or_default()),
        Some(Err(m)) => fail(&mut errors, "description_html", m.clone()),
        None => {}
    }
    if let Some(v) = b.get("priority") {
        match choice_field(v, &PRIORITIES) {
            Ok(priority) => p.priority = Priority::parse(priority),
            Err(m) => fail(&mut errors, "priority", m),
        }
    }
    for (field, slot) in [("start_date", &mut p.start), ("target_date", &mut p.target)] {
        if let Some(v) = b.get(field) {
            match date_field(v) {
                Ok(date) => *slot = Some((v.as_str().map(str::to_string), date)),
                Err(m) => fail(&mut errors, field, m),
            }
        }
    }
    // read, checked, and not kept
    if let Some(v) = b.get("sequence_id") {
        if let Err(m) = int_field(v, i64::from(i32::MIN), i64::from(i32::MAX)) {
            fail(&mut errors, "sequence_id", m);
        }
    }
    if let Some(v) = b.get("sort_order") {
        if let Err(m) = float_field(v) {
            fail(&mut errors, "sort_order", m);
        }
    }
    if let Some(v) = b.get("archived_at") {
        if let Err(m) = date_field(v) {
            fail(&mut errors, "archived_at", m);
        }
    }
    if let Some(v) = b.get("is_draft") {
        if let Err(m) = bool_field(v) {
            fail(&mut errors, "is_draft", m);
        }
    }
    for (field, slot) in [("external_source", &mut p.external_source), ("external_id", &mut p.external_id)] {
        if let Some(v) = b.get(field) {
            match text(Some(255), true, true)(v) {
                Ok(t) => *slot = Some(t),
                Err(m) => fail(&mut errors, field, m),
            }
        }
    }
    if let Some(v) = b.get("point") {
        if v.is_null() {
            p.point = Some(None);
        } else {
            match int_field(v, 0, 12).and_then(|n| i32::try_from(n).map_err(|e| e.to_string())) {
                Ok(n) => p.point = Some(Some(n)),
                Err(m) => fail(&mut errors, "point", m),
            }
        }
    }
    if let Some(v) = b.get("deleted_at") {
        match datetime_field(v) {
            Ok(set) => p.deleted_at = set,
            Err(m) => fail(&mut errors, "deleted_at", m),
        }
    }
    if let Some(v) = b.get("created_by") {
        match pk_field(v) {
            PkRead::Null => {}
            PkRead::Key { id, shown } => {
                if db.user_by_id(UserId(id)).await?.is_none() {
                    fail(&mut errors, "created_by", no_such_pk_shown(&shown));
                }
            }
            PkRead::Refused(m) => fail(&mut errors, "created_by", m),
            PkRead::Raises => raises = true,
        }
    }
    // the work items and the states: of this project, or not there
    if let Some(v) = b.get("parent") {
        match pk_field(v) {
            PkRead::Null => p.parent = Some(None),
            PkRead::Key { id, shown } => {
                if db.work_item_row(&scope, WorkItemId(id)).await?.is_some() {
                    p.parent = Some(Some(id));
                } else {
                    fail(&mut errors, "parent", no_such_pk_shown(&shown));
                }
            }
            PkRead::Refused(m) => fail(&mut errors, "parent", m),
            PkRead::Raises => raises = true,
        }
    }
    if let Some(v) = b.get("state") {
        match pk_field(v) {
            PkRead::Null => p.state = Some(None),
            PkRead::Key { id, shown } => {
                if db.project_states(&scope).await?.iter().any(|s| s.id == id) {
                    p.state = Some(Some(id));
                } else {
                    fail(&mut errors, "state", no_such_pk_shown(&shown));
                }
            }
            PkRead::Refused(m) => fail(&mut errors, "state", m),
            PkRead::Raises => raises = true,
        }
    }
    if raises {
        return Err(raised());
    }
    refuse_if_any(errors)?;

    // `validate()`, which raises at its first refusal
    if let (Some((_, Some(start))), Some((_, Some(target)))) = (&p.start, &p.target) {
        if start > target {
            return Err(non_field("Start date cannot exceed target date"));
        }
    }
    if let Some(Ok(sent)) = &html {
        let sent = sent.as_deref().unwrap_or_default();
        // `html.fromstring("")` has no document to read
        if sent.is_empty() {
            return Err(non_field("Invalid HTML passed"));
        }
        match thrust_editor_doc::sanitize::validate_html_content(sent) {
            Ok(clean) => p.description_html = Some(clean.unwrap_or_else(|| sent.to_string())),
            Err(_) => {
                let mut errors = FieldErrors::new();
                fail(&mut errors, "error", "html content is not valid");
                return Err(V1Error::Fields(errors));
            }
        }
    }
    if let Some(people) = &p.assignees {
        let members = db.project_members(&scope).await?;
        let invalid: Vec<Uuid> = people
            .iter()
            .filter(|u| !members.iter().any(|m| m.user_id == **u && m.role >= Role::Member))
            .map(|u| u.0)
            .collect();
        if !invalid.is_empty() {
            return Err(non_field(&format!(
                "Assignees {} are not active members of this project",
                uuid_list(&invalid)
            )));
        }
    }
    Ok(p)
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
async fn list(
    V1Path((slug, project)): V1Path<(String, PlaneUuid)>,
    ctx: Ctx,
    RawQuery(raw): RawQuery,
) -> Result<Json<Value>, V1Error> {
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
        return Ok(Json(shown(&ctx, &access, &q, &row).await?));
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
    let results = page.rows.iter().map(|r| pick_fields(work_item_body(r, &x), &q)).collect();
    Ok(Json(plan.envelope(results, usize::try_from(page.total).unwrap_or(usize::MAX))))
}

/// `POST .../{family}/`: make a work item. The pin answers 201; a body that gives `deleted_at` a value makes the work item and then
/// fails (PQ-404), which here makes none.
async fn create(
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
        return Err(crate::plane_500());
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
    Ok((StatusCode::CREATED, Json(Value::Object(work_item_body(&row, &x)))))
}

/// `GET .../{family}/{pk}/`.
async fn retrieve(
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
async fn update(
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
    Ok(Json(Value::Object(work_item_body(&row, &x))))
}

/// `DELETE .../{family}/{pk}/`: a project admin or the work item's maker.
async fn destroy(
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
async fn search(V1Path(slug): V1Path<String>, ctx: Ctx, RawQuery(raw): RawQuery) -> Result<Json<Value>, V1Error> {
    ctx.throttle().await?;
    let q = query_map(raw.as_deref());
    let text = q.get("search").cloned().unwrap_or_default();
    if text.is_empty() {
        return Ok(Json(json!({ "issues": [] })));
    }
    // `int(limit)` raises on text, and a negative slice is refused by the ORM: both are the pin's 500
    let limit = match q.get("limit") {
        None => 10,
        Some(s) => py_int(s).and_then(|n| u32::try_from(n).ok()).ok_or_else(crate::plane_500)?,
    };
    let workspace_search = q.get("workspace_search").map_or("false", String::as_str);
    let project = match q.get("project_id").filter(|p| !p.is_empty()) {
        Some(p) if workspace_search == "false" => Some(ProjectId(parse_uuid_text(p).ok_or_else(raised)?)),
        _ => None,
    };
    let scope = match thrust_core::authorize_workspace(&ctx.app, &ctx.principal, &WorkspaceSlug::new(&slug)).await {
        Ok(scope) => scope,
        Err(thrust_core::CoreError::Forbidden | thrust_core::CoreError::NotFound) => {
            return Ok(Json(json!({ "issues": [] })))
        }
        Err(other) => return Err(other.into()),
    };
    let search = WorkspaceSearch { text: text.clone(), numbers: whole_numbers(&text), project, limit };
    let hits = thrust_core::search_issues(&ctx.app, &scope, search).await.map_err(item_error)?;
    let issues: Vec<Value> = hits
        .into_iter()
        .map(|h| {
            json!({
                "name": h.name,
                "id": h.id.0,
                "sequence_id": h.sequence_id,
                "project__identifier": h.project_identifier,
                "project_id": h.project_id.0,
                "workspace__slug": h.workspace_slug.0,
            })
        })
        .collect();
    Ok(Json(json!({ "issues": issues })))
}

/// `GET /workspaces/{slug}/{family}/{KEY}-{n}/`: a work item by the identifier of its project and its number. The route splits at the
/// last hyphen; the permission is a membership of a project of the workspace that has the identifier (a stranger, or an identifier
/// that is nobody's, is the refusal of a permission); a number that is not a whole number is a 404 of its own words.
async fn by_identifier(
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
