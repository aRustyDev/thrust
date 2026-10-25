//! Work items: made (S48), listed a page at a time (S47) and opened (X01).
//!
//! The shapes are the pin's, field for field. A work item has three: the one a list holds (23 columns and the people, labels and
//! modules, with the group of its state), the one a creation answers (the same without the group, with `deleted_at`), and the one
//! it opens as (the create's, with its description and whether the caller subscribes, and what the request asks to be expanded).
//! Where a column has nothing behind it yet (the parent, the estimate, the cycle, the counts of attachments, links and sub work
//! items) the answer is what the pin gives a work item that has none: `null`, and `[]` for the modules.
//!
//! A creation is checked the way Django REST Framework checks it (`fields.rs`): every field at once, answered as
//! `{field: [message]}`, and a relation that names nothing in DRF's words. What the pin leaves to a database lookup is the core's
//! (`thrust_core::FieldProblem`), and is put in the same words here.
//!
//! A list is the pin's paginator: a cursor `size:page:previous` in which only the page counts, a page size that the request names
//! apart from it, twelve keys round the rows. Grouping, and every filter, are refused in words of Thrust's own (deviation D-028):
//! answering them with a flat list or an unfiltered one would be wrong data.

use std::collections::HashMap;

use axum::body::Bytes;
use axum::extract::{Path, Query};
use axum::http::StatusCode;
use axum::{Extension, Json};
use serde_json::{json, Map, Value};
use thrust_contracts::chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use thrust_contracts::*;
use thrust_core::{App, FieldProblem, IssueDraft, IssueError, IssueListQuery};
use thrust_ports::*;

use crate::fields::{char_field, choice_field, fail, FieldErrors};
use crate::projects::{key_field, missing_object, nothing_field, project_id};
use crate::users::{object_body, ts, ts_or_null, Caller};
use crate::workspace_load::refused_by_decorator;
use crate::workspaces::id_or_null;
use crate::PlaneError;

// --- the work item as the app shows it ------------------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum Shape {
    /// A row of a list: `issue_on_results`.
    List,
    /// The answer to a creation.
    Created,
    /// The work item the app opens.
    Detail,
}

fn date_or_null(d: Option<NaiveDate>) -> Value {
    d.map_or(Value::Null, |d| json!(d.format("%Y-%m-%d").to_string()))
}

/// A work item in one of the pin's shapes. `sort_order` is a float there (a column of floats), and the columns nothing is behind
/// yet are `null`: the counts the pin annotates with a subquery that finds no row, the estimate, the parent and the cycle.
fn issue_json(r: &WorkItemRow, shape: Shape) -> Value {
    let (i, a) = (&r.item, &r.attrs);
    let mut m = Map::new();
    let mut put = |k: &str, v: Value| {
        m.insert(k.to_string(), v);
    };
    put("id", json!(i.id.0));
    put("name", json!(i.name));
    put("state_id", json!(a.state_id));
    put("sort_order", json!(i.sort_order as f64));
    put("completed_at", ts_or_null(a.completed_at));
    put("estimate_point", Value::Null);
    put("priority", json!(a.priority.as_str()));
    put("start_date", date_or_null(a.start_date));
    put("target_date", date_or_null(a.target_date));
    put("sequence_id", json!(i.sequence_id));
    put("project_id", json!(i.project_id.0));
    put("parent_id", Value::Null);
    put("cycle_id", Value::Null);
    put("module_ids", json!([]));
    put("label_ids", json!(a.label_ids));
    put("assignee_ids", json!(a.assignee_ids.iter().map(|u| u.0).collect::<Vec<_>>()));
    put("sub_issues_count", Value::Null);
    put("created_at", json!(ts(i.created_at)));
    put("updated_at", json!(ts(a.updated_at)));
    put("created_by", json!(i.created_by.0));
    put("updated_by", id_or_null(a.updated_by));
    put("attachment_count", Value::Null);
    put("link_count", Value::Null);
    put("is_draft", json!(i.is_draft));
    put("archived_at", ts_or_null(i.archived_at));
    match shape {
        Shape::List => put("state__group", json!(r.state_group.map(|g| g.as_str()))),
        Shape::Created => put("deleted_at", Value::Null),
        Shape::Detail => {
            put("description_html", json!(a.description_html));
            // nobody subscribes to anything yet
            put("is_subscribed", json!(false));
        }
    }
    Value::Object(m)
}

// --- who may, and what a refusal says ---------------------------------------------------------------------------------------

/// A refusal of a work item route, in the words of the pin's guard.
fn refusal(e: IssueError) -> PlaneError {
    match e {
        IssueError::Forbidden => refused_by_decorator(),
        IssueError::NotFound => missing_object(),
        IssueError::NotAllowedToView => PlaneError::Forbidden(json!({"error": "You are not allowed to view this issue"})),
        IssueError::StartAfterTarget => {
            let mut errors = FieldErrors::new();
            fail(&mut errors, "non_field_errors", "Start date cannot exceed target date");
            PlaneError::Fields(errors)
        }
        // the problems of a creation are put in DRF's words by `create`, which has the texts the request used
        IssueError::Fields(_) => PlaneError::BadRequest(json!({"error": "The payload is not valid"})),
        IssueError::Core(e) => e.into(),
    }
}

// --- reading a creation the way DRF does -----------------------------------------------------------------------------------

/// Python's name for the type of a JSON value (what DRF's "Expected a list" message prints).
fn py_type_name(v: &Value) -> &'static str {
    match v {
        Value::Null => "NoneType",
        Value::Bool(_) => "bool",
        Value::Number(n) if n.is_f64() => "float",
        Value::Number(_) => "int",
        Value::String(_) => "str",
        Value::Array(_) => "list",
        Value::Object(_) => "dict",
    }
}

/// A date as Django's `parse_date` reads it (what DRF's `DateField` calls), in the three forms probed against the pin and recorded
/// (fixture S48): `YYYY-M-D` with one or two digits for the month and the day, `YYYYMMDD`, and the ISO week `YYYY-Www-D`. No space
/// around it, no time after it, a day that exists, a year from 1 to 9999. (Python 3.11's `date.fromisoformat` reads more spellings
/// of a week, and digits of other scripts; neither is read here.)
fn parse_date(s: &str) -> Option<NaiveDate> {
    let digits = |t: &str, min: usize, max: usize| (min..=max).contains(&t.len()) && t.bytes().all(|b| b.is_ascii_digit());
    let number = |t: &str| t.parse::<u32>().ok();
    let ymd = |y: u32, m: u32, d: u32| NaiveDate::from_ymd_opt(i32::try_from(y).ok().filter(|y| (1..=9999).contains(y))?, m, d);
    let parts: Vec<&str> = s.split('-').collect();
    match parts.as_slice() {
        [y, m, d] if digits(y, 4, 4) && digits(m, 1, 2) && digits(d, 1, 2) => ymd(number(y)?, number(m)?, number(d)?),
        [y, w, d] if digits(y, 4, 4) && digits(d, 1, 1) && w.strip_prefix('W').is_some_and(|n| digits(n, 2, 2)) => {
            let year = i32::try_from(number(y)?).ok().filter(|y| (1..=9999).contains(y))?;
            let weekday = thrust_contracts::chrono::Weekday::try_from(u8::try_from(number(d)?.checked_sub(1)?).ok()?).ok()?;
            NaiveDate::from_isoywd_opt(year, number(&w[1..])?, weekday)
        }
        [whole] if digits(whole, 8, 8) => ymd(number(&whole[..4])?, number(&whole[4..6])?, number(&whole[6..])?),
        _ => None,
    }
}

/// DRF `DateField` on a column that allows null: null, or a date as [`parse_date`] reads one (a time after it is refused).
fn date_field(v: &Value) -> Result<Option<NaiveDate>, String> {
    match v {
        Value::Null => Ok(None),
        Value::String(s) => parse_date(s).map(Some).ok_or_else(|| "Date has wrong format. Use one of these formats instead: YYYY-MM-DD.".to_string()),
        _ => Err("Date has wrong format. Use one of these formats instead: YYYY-MM-DD.".into()),
    }
}

/// DRF `ListField` of related keys (the assignees, the labels): a list, each of whose members is the key of a row. A member that
/// is no key is an error at its place; whether the row exists is the core's to find. Returns the keys with the text each was
/// sent as, which the messages of the core's findings quote.
fn key_list(v: &Value) -> Result<Vec<(String, Uuid)>, Value> {
    let Some(items) = v.as_array() else {
        return Err(match v {
            Value::Null => json!(["This field may not be null."]),
            other => json!([format!("Expected a list of items but got type \"{}\".", py_type_name(other))]),
        });
    };
    let mut keys = Vec::new();
    let mut bad = Map::new();
    for (i, item) in items.iter().enumerate() {
        match key_field(item) {
            Ok(Some((text, id))) => keys.push((text, id)),
            // null, or an empty string (DRF reads it as null): the child field does not allow null
            Ok(None) => {
                bad.insert(i.to_string(), json!(["This field may not be null."]));
            }
            Err(m) => {
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

/// The texts a request named its relations with, for the messages of what the core did not find.
#[derive(Default)]
struct Texts {
    state: String,
    parent: String,
    assignees: Vec<String>,
    labels: Vec<String>,
}

/// What `IssueCreateSerializer` makes of a request body, every field at once.
fn issue_draft(b: &Map<String, Value>) -> Result<(IssueDraft, Texts), FieldErrors> {
    let mut errors = FieldErrors::new();
    let mut draft = IssueDraft::default();
    let mut texts = Texts::default();

    match b.get("name") {
        None => fail(&mut errors, "name", "This field is required."),
        Some(v) => match char_field(v, Some(255), false, false) {
            Ok(name) => draft.name = name.unwrap_or_default(),
            Err(m) => fail(&mut errors, "name", m),
        },
    }
    if let Some(v) = b.get("description_html") {
        match char_field(v, None, true, false) {
            // the pin cleans what it stores; an empty text stays empty (the cleaner answers none for it)
            Ok(text) => match thrust_editor_doc::sanitize::validate_html_content(text.as_deref().unwrap_or_default()) {
                Ok(clean) => draft.description_html = clean.or(text),
                Err(_) => fail(&mut errors, "error", "html content is not valid"),
            },
            Err(m) => fail(&mut errors, "description_html", m),
        }
    }
    if let Some(v) = b.get("priority") {
        match choice_field(v, &["urgent", "high", "medium", "low", "none"]) {
            Ok(p) => draft.priority = Priority::parse(p),
            Err(m) => fail(&mut errors, "priority", m),
        }
    }
    for (field, keep_text, slot) in [("state_id", true, &mut draft.state_id), ("parent_id", false, &mut draft.parent_id)] {
        if let Some(v) = b.get(field) {
            match key_field(v) {
                Ok(Some((text, id))) => {
                    *slot = Some(id);
                    if keep_text {
                        texts.state = text;
                    } else {
                        texts.parent = text;
                    }
                }
                Ok(None) => {}
                Err(m) => fail(&mut errors, field, m),
            }
        }
    }
    if let Some(v) = b.get("estimate_point") {
        if let Err(m) = nothing_field(v) {
            fail(&mut errors, "estimate_point", m);
        }
    }
    for field in ["start_date", "target_date"] {
        if let Some(v) = b.get(field) {
            match date_field(v) {
                Ok(d) if field == "start_date" => draft.start_date = d,
                Ok(d) => draft.target_date = d,
                Err(m) => fail(&mut errors, field, m),
            }
        }
    }
    if let Some(v) = b.get("assignee_ids") {
        match key_list(v) {
            Ok(keys) => {
                draft.assignee_ids = keys.iter().map(|(_, id)| UserId(*id)).collect();
                texts.assignees = keys.into_iter().map(|(t, _)| t).collect();
            }
            Err(e) => {
                errors.insert("assignee_ids".into(), e);
            }
        }
    }
    if let Some(v) = b.get("label_ids") {
        match key_list(v) {
            Ok(keys) => {
                draft.label_ids = keys.iter().map(|(_, id)| *id).collect();
                texts.labels = keys.into_iter().map(|(t, _)| t).collect();
            }
            Err(e) => {
                errors.insert("label_ids".into(), e);
            }
        }
    }
    if errors.is_empty() {
        Ok((draft, texts))
    } else {
        Err(errors)
    }
}

/// What the core did not find, in DRF's words for a relation that names nothing.
fn problems_to_errors(problems: &[FieldProblem], texts: &Texts) -> FieldErrors {
    let none = |text: &str| format!("Invalid pk \"{text}\" - object does not exist.");
    let mut errors = FieldErrors::new();
    let mut nested: HashMap<&str, Map<String, Value>> = HashMap::new();
    for p in problems {
        match p {
            FieldProblem::State => fail(&mut errors, "state_id", none(&texts.state)),
            FieldProblem::Parent => fail(&mut errors, "parent_id", none(&texts.parent)),
            FieldProblem::ParentNotSupported => fail(&mut errors, "parent_id", "Sub-work items are not supported yet."),
            FieldProblem::Assignee(i) => {
                nested.entry("assignee_ids").or_default().insert(i.to_string(), json!([none(&texts.assignees[*i])]));
            }
            FieldProblem::Label(i) => {
                nested.entry("label_ids").or_default().insert(i.to_string(), json!([none(&texts.labels[*i])]));
            }
        }
    }
    for (field, by_place) in nested {
        errors.insert(field.to_string(), Value::Object(by_place));
    }
    errors
}

// --- S48 ----------------------------------------------------------------------------------------------------------------

/// S48. Make a work item. An admin or a member of the project may; anyone else, the project missing, a stranger to the workspace
/// are the decorator's 403.
pub(crate) async fn create(
    Caller(who): Caller,
    Extension(app): Extension<App>,
    Path((slug, project)): Path<(String, String)>,
    body: Bytes,
) -> Result<(StatusCode, Json<Value>), PlaneError> {
    let project = project_id(&project)?;
    let slug = WorkspaceSlug(slug);
    // the pin's decorator answers before it reads the body
    let access = thrust_core::issue_writer(&app, &who, &slug, project).await.map_err(refusal)?;
    let b = object_body(&body)?;
    let (draft, texts) = issue_draft(&b).map_err(PlaneError::Fields)?;
    match thrust_core::create_issue(&app, &access, draft).await {
        Ok(row) => Ok((StatusCode::CREATED, Json(issue_json(&row, Shape::Created)))),
        Err(IssueError::Fields(problems)) => Err(PlaneError::Fields(problems_to_errors(&problems, &texts))),
        Err(e) => Err(refusal(e)),
    }
}

// --- S47 ----------------------------------------------------------------------------------------------------------------

const DEFAULT_PER_PAGE: u32 = 1000;

fn invalid_detail(message: &str) -> PlaneError {
    PlaneError::BadRequest(json!({"detail": message}))
}

/// The page size: the request's `per_page`, or 1000; not more than 1000. (The pin answers 500 for a size of 0 or less; Thrust
/// answers 400, as for a size that is no number.)
fn per_page(q: &HashMap<String, String>) -> Result<u32, PlaneError> {
    let Some(raw) = q.get("per_page") else { return Ok(DEFAULT_PER_PAGE) };
    let n: i64 = raw.trim().parse().map_err(|_| invalid_detail("Invalid per_page parameter."))?;
    if n > i64::from(DEFAULT_PER_PAGE) {
        return Err(invalid_detail("Invalid per_page value. Cannot exceed 1000."));
    }
    u32::try_from(n).ok().filter(|n| *n > 0).ok_or_else(|| invalid_detail("Invalid per_page parameter."))
}

/// The page asked for: a cursor is `size:page:previous`, all three numbers (the size may be a float), and only the page counts. No
/// cursor is the first page.
fn page(q: &HashMap<String, String>) -> Result<u32, PlaneError> {
    let Some(raw) = q.get("cursor") else { return Ok(0) };
    let parts: Vec<&str> = raw.split(':').collect();
    let invalid = || invalid_detail("Invalid cursor parameter.");
    if parts.len() != 3 {
        return Err(invalid());
    }
    let size_ok = parts[0].trim().parse::<i64>().is_ok() || parts[0].trim().parse::<f64>().is_ok_and(f64::is_finite);
    let page: i64 = parts[1].trim().parse().map_err(|_| invalid())?;
    let _previous: i64 = parts[2].trim().parse().map_err(|_| invalid())?;
    if !size_ok {
        return Err(invalid());
    }
    if page < 0 {
        return Err(invalid_detail("Error in parsing"));
    }
    Ok(u32::try_from(page).unwrap_or(u32::MAX))
}

/// The order asked for, as the pin sanitises it: an unknown value, and one with two minus signs, is `-created_at`. The three it
/// can order by the least of a list (labels, assignees, modules) are not offered here and are the default too.
fn order(q: &HashMap<String, String>) -> WorkItemOrder {
    let default = WorkItemOrder { field: WorkItemOrderField::CreatedAt, descending: true };
    let raw = q.get("order_by").map(String::as_str).unwrap_or_default();
    let (descending, bare) = match raw.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, raw),
    };
    let field = match bare {
        "created_at" => WorkItemOrderField::CreatedAt,
        "updated_at" => WorkItemOrderField::UpdatedAt,
        "sequence_id" => WorkItemOrderField::SequenceId,
        "sort_order" => WorkItemOrderField::SortOrder,
        "target_date" => WorkItemOrderField::TargetDate,
        "start_date" => WorkItemOrderField::StartDate,
        "completed_at" => WorkItemOrderField::CompletedAt,
        // every work item listed is unarchived, so this order is all ties, which run newest first: the default's answer. The arm
        // names an order the pin offers; no test can tell it from the fall-through (an equivalent mutant, see G-21).
        "archived_at" => WorkItemOrderField::ArchivedAt,
        "priority" => WorkItemOrderField::Priority,
        "state__name" => WorkItemOrderField::StateName,
        "state__group" => WorkItemOrderField::StateGroup,
        _ => return default,
    };
    WorkItemOrder { field, descending }
}

/// A time as Django's `DateTimeField` reads it from a query: RFC 3339, the same with a space for the `T` and no zone (UTC), or a
/// date (midnight).
fn datetime(raw: &str) -> Option<Timestamp> {
    let raw = raw.trim();
    if let Ok(t) = DateTime::parse_from_rfc3339(raw) {
        return Some(t.with_timezone(&Utc));
    }
    for format in ["%Y-%m-%dT%H:%M:%S%.f", "%Y-%m-%d %H:%M:%S%.f"] {
        if let Ok(t) = NaiveDateTime::parse_from_str(raw, format) {
            return Some(t.and_utc());
        }
    }
    NaiveDate::parse_from_str(raw, "%Y-%m-%d").ok().and_then(|d| d.and_hms_opt(0, 0, 0)).map(|t| t.and_utc())
}

/// The query parameters of a list that this one does not answer (deviation D-028): grouping, a filter of the pin's 25, and a rich
/// filter that says something. The two the app sends with every request, `sub_issue` and `layout`, are not among them (a work item
/// has no parent here, so `sub_issue=false` leaves out nothing).
fn unsupported(q: &HashMap<String, String>) -> Result<(), PlaneError> {
    const FILTERS: [&str; 23] = [
        "assignees", "completed_at", "created_at", "created_by", "cycle", "estimate_point", "inbox_status", "intake_status", "labels",
        "logged_by", "mentions", "module", "name", "parent", "priority", "project", "start_date", "start_target_date", "state",
        "state_group", "subscriber", "target_date", "type",
    ];
    let given = |k: &str| q.get(k).is_some_and(|v| !v.is_empty());
    if given("group_by") || given("sub_group_by") {
        return Err(PlaneError::BadRequest(json!({"error": "Grouping work items is not supported yet."})));
    }
    if FILTERS.iter().any(|k| given(k)) {
        return Err(PlaneError::BadRequest(json!({"error": "Filtering work items is not supported yet."})));
    }
    if let Some(raw) = q.get("filters").filter(|v| !v.is_empty()) {
        match serde_json::from_str::<Value>(raw) {
            Ok(Value::Object(m)) if m.is_empty() => {}
            Ok(Value::Object(_)) => return Err(PlaneError::BadRequest(json!({"error": "Filtering work items is not supported yet."}))),
            _ => {
                return Err(PlaneError::BadRequest(json!({"message": "Invalid JSON for 'filter'. Expected a valid JSON object.", "code": "invalid_json"})))
            }
        }
    }
    Ok(())
}

/// S47. A page of the project's work items, flat. Any member of the project may ask (a guest of a project that does not show
/// guests everything sees what they made); the page size, the cursor, the order and `updated_at__gt` are the pin's.
pub(crate) async fn list(
    Caller(who): Caller,
    Extension(app): Extension<App>,
    Path((slug, project)): Path<(String, String)>,
    Query(q): Query<HashMap<String, String>>,
) -> Result<Json<Value>, PlaneError> {
    let project = project_id(&project)?;
    let slug = WorkspaceSlug(slug);
    // the pin's decorator answers before it reads a parameter
    let access = thrust_core::issue_reader(&app, &who, &slug, project).await.map_err(refusal)?;
    let limit = per_page(&q)?;
    let page = page(&q)?;
    let updated_after = match q.get("updated_at__gt") {
        Some(raw) => Some(datetime(raw).ok_or_else(|| PlaneError::BadRequest(json!({"error": "Please provide valid detail"})))?),
        None => None,
    };
    unsupported(&q)?;
    let got = thrust_core::list_issues(&app, &access, IssueListQuery { order: order(&q), limit, page, updated_after }).await.map_err(refusal)?;
    let total = got.total;
    Ok(Json(json!({
        "grouped_by": null,
        "sub_grouped_by": null,
        "total_count": total,
        "next_cursor": format!("{limit}:{}:0", i64::from(page) + 1),
        "prev_cursor": format!("{limit}:{}:1", i64::from(page) - 1),
        "next_page_results": got.has_next,
        "prev_page_results": page > 0,
        "count": got.rows.len(),
        "total_pages": total.div_ceil(u64::from(limit)),
        "total_results": total,
        "extra_stats": null,
        "results": got.rows.iter().map(|r| issue_json(r, Shape::List)).collect::<Vec<_>>(),
    })))
}

// --- X01 ----------------------------------------------------------------------------------------------------------------

/// X01. A work item, as the app opens it. `expand` names what the pin adds to it: reactions, attachments and links (none exist),
/// and the parent (it has none).
pub(crate) async fn retrieve(
    Caller(who): Caller,
    Extension(app): Extension<App>,
    Path((slug, project, issue)): Path<(String, String, String)>,
    Query(q): Query<HashMap<String, String>>,
) -> Result<Json<Value>, PlaneError> {
    let project = project_id(&project)?;
    let issue: WorkItemId = issue.parse().map_err(|_| PlaneError::NotFound(json!({"error": "Page not found."})))?;
    let access = thrust_core::issue_reader(&app, &who, &WorkspaceSlug(slug), project).await.map_err(refusal)?;
    let row = thrust_core::get_issue(&app, &access, issue).await.map_err(refusal)?;
    let mut out = issue_json(&row, Shape::Detail);
    let m = out.as_object_mut().expect("an object");
    for name in q.get("expand").map(String::as_str).unwrap_or_default().split(',') {
        match name {
            "issue_reactions" | "issue_attachments" | "issue_link" => {
                m.insert(name.to_string(), json!([]));
            }
            "parent" => {
                m.insert("parent".to_string(), Value::Null);
            }
            _ => {}
        }
    }
    Ok(Json(out))
}
