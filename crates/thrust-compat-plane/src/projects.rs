//! Projects: made, changed and opened (S30, S32, S34), and the reads the app makes once a project is open (S35 to S46).
//!
//! The shapes are the pin's, field for field. Where the pin is odd it is copied, because the fixtures show it and the React app was
//! written against it: a project's create and update answers are the long list shape (44 keys), a patch that changes nothing
//! still saves (the time and who changed it move), six of the routes refuse with `{"error"}` and three with `{"detail"}`
//! depending on how the pin guards them, and a member's own row is a 404 for anyone who has none.
//!
//! The request bodies are checked the way Django REST Framework checks them (`fields.rs`), every field at once, answered as
//! `{field: [message]}`. A name or key that is taken is a field error found by looking, and also by the database's unique index,
//! so two creations racing for one name end with one project and a 400 for the other.
//!
//! S42 to S46 (estimates, cycles and the two kinds of modules, views) are stubs (D-021): they answer the empty list, after asking
//! who the caller is.

use std::collections::HashMap;

use axum::body::Bytes;
use axum::extract::{Path, Query};
use axum::http::StatusCode;
use axum::{Extension, Json};
use serde_json::{json, Map, Value};
use thrust_contracts::*;
use thrust_core::{App, CoreError, Permission, ProjectError, ProjectPatch};
use thrust_ports::*;

use crate::fields::{bool_field, char_field, choice_field, fail, py_str, FieldErrors};
use crate::users::{non_empty, object_body, ts, ts_or_null, Caller};
use crate::workspace_load::{decorated, network_number, refused_by_class, refused_by_decorator, ALL_ROLES};
use crate::workspaces::{id_or_null, role_number, scope};
use crate::PlaneError;

/// The 404 of a view that fetches one object with `.get()` and does not find it.
pub(crate) fn missing_object() -> PlaneError {
    PlaneError::NotFound(json!({"error": "The required object does not exist."}))
}

/// A route's project id, or the 404 Django gives a path that no route matches: the pin's URLs take `<uuid:project_id>`.
pub(crate) fn project_id(raw: &str) -> Result<ProjectId, PlaneError> {
    raw.parse().map_err(|_| PlaneError::NotFound(json!({"detail": "Not found."})))
}

// --- the project as the app shows it ---------------------------------------------------------------------------------------

/// A project as S30, S32 and S34 answer it: the pin's `ProjectListSerializer`, 44 keys.
pub(crate) fn project_json(d: &ProjectDetail) -> Value {
    let (p, l) = (&d.listing.project, &d.listing);
    json!({
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
        "default_assignee": id_or_null(p.default_assignee),
        "project_lead": id_or_null(p.project_lead),
        "module_view": p.module_view,
        "cycle_view": p.cycle_view,
        "issue_views_view": p.issue_views_view,
        "page_view": p.page_view,
        "intake_view": p.intake_view,
        "inbox_view": p.intake_view,
        "is_time_tracking_enabled": p.is_time_tracking_enabled,
        "is_issue_type_enabled": p.is_issue_type_enabled,
        "guest_view_all_features": p.guest_view_all_features,
        "estimate": p.estimate,
        "archive_in": p.archive_in,
        "close_in": p.close_in,
        "default_state": p.default_state,
        "archived_at": ts_or_null(p.archived_at),
        "timezone": p.timezone,
        "external_source": p.external_source,
        "external_id": p.external_id,
        "created_at": ts(p.created_at),
        "updated_at": ts(p.updated_at),
        "created_by": id_or_null(p.created_by),
        "updated_by": id_or_null(p.updated_by),
        "deleted_at": null,
        // nothing can be starred yet (D-021), and no deploy board exists
        "is_favorite": false,
        "anchor": null,
        "member_role": l.role.map(role_number),
        "sort_order": l.sort_order,
        "members": d.members.iter().map(|u| u.0).collect::<Vec<_>>(),
        "next_work_item_sequence": d.next_sequence,
    })
}

/// What a project refusal is on the pin, for the routes that ask who the caller is with a decorator.
fn refusal(e: ProjectError) -> PlaneError {
    match e {
        ProjectError::NotFound => PlaneError::NotFound(json!({"error": "Project does not exist"})),
        ProjectError::Forbidden => refused_by_decorator(),
        ProjectError::Secret => PlaneError::Forbidden(json!({"error": "You do not have permission"})),
        ProjectError::NotAMember => PlaneError::Conflict(json!({"error": "You are not a member of this project"})),
        ProjectError::Archived => PlaneError::BadRequest(json!({"error": "Archived projects cannot be updated"})),
        ProjectError::Core(e) => e.into(),
    }
}

// --- reading the request the way DRF's ModelSerializer does ---------------------------------------------------------------

/// A JSON column: any value, null where the column allows it.
fn json_field(v: &Value, allow_null: bool) -> Result<Option<Value>, String> {
    match v {
        Value::Null if allow_null => Ok(None),
        Value::Null => Err("This field may not be null.".into()),
        other => Ok(Some(other.clone())),
    }
}

/// How Python writes a string literal in a `repr`: in single quotes, or in double quotes when the string holds a single quote and
/// no double one; the quote it uses, the backslash and the control characters escaped.
fn py_string(s: &str) -> String {
    let quote = if s.contains('\'') && !s.contains('"') { '"' } else { '\'' };
    let mut out = String::from(quote);
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c == quote => {
                out.push('\\');
                out.push(c);
            }
            c if c < ' ' || c == '\u{7f}' => out.push_str(&format!("\\x{:02x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push(quote);
    out
}

/// How Python writes a JSON value that was read into a `dict`, a `list`, a `str`, a `float` or one of the three constants (what
/// Django's message for a value that is no UUID shows of it): `True`, `False` and `None`, strings as [`py_string`], and `, ` and
/// `: ` between the members.
fn py_repr(v: &Value) -> String {
    match v {
        Value::String(s) => py_string(s),
        Value::Array(a) => format!("[{}]", a.iter().map(py_repr).collect::<Vec<_>>().join(", ")),
        Value::Object(m) => format!("{{{}}}", m.iter().map(|(k, v)| format!("{}: {}", py_string(k), py_repr(v))).collect::<Vec<_>>().join(", ")),
        other => py_str(other),
    }
}

/// What the pin makes of `description_html` of each kind of value (probed against it, and recorded in fixture S32): it keeps null,
/// a string and an empty list or object, and writes anything else as Python's `str()` of it (`5` is `"5"`, `true` is `"True"`,
/// `{"a": 1}` is `"{'a': 1}"`).
fn html_text(v: &Value) -> Value {
    match v {
        Value::Null | Value::String(_) => v.clone(),
        Value::Array(a) if a.is_empty() => v.clone(),
        Value::Object(m) if m.is_empty() => v.clone(),
        other => Value::String(py_repr(other)),
    }
}

/// What DRF's `IntegerField` makes of a string: it cuts a trailing `.0` (any number of zeros) and then Python's `int()` reads the
/// rest, which allows a sign and spaces around it.
fn decimal_text(s: &str) -> Option<i128> {
    let t = s.trim_end();
    let t = match t.find('.') {
        Some(dot) if t[dot + 1..].bytes().all(|b| b == b'0') => &t[..dot],
        _ => t,
    };
    t.trim().parse().ok()
}

/// DRF `IntegerField` bounded 0 to 12 (the `MinValueValidator` and `MaxValueValidator` of the pin's `archive_in` and `close_in`).
/// A float is read through its text, so one with no fraction is an integer, and one that Python writes with an exponent (from
/// 1e16 up) is not.
fn months_field(v: &Value) -> Result<i32, String> {
    let n = match v {
        Value::Number(n) => n
            .as_i64()
            .map(i128::from)
            .or_else(|| n.as_u64().map(i128::from))
            .or_else(|| n.as_f64().filter(|f| f.fract() == 0.0 && f.abs() < 1e16).map(|f| f as i128)),
        Value::String(s) => decimal_text(s),
        Value::Null => return Err("This field may not be null.".into()),
        _ => None,
    }
    .ok_or_else(|| "A valid integer is required.".to_string())?;
    match n {
        n if n > 12 => Err("Ensure this value is less than or equal to 12.".into()),
        n if n < 0 => Err("Ensure this value is greater than or equal to 0.".into()),
        n => Ok(n as i32),
    }
}

/// DRF `PrimaryKeyRelatedField.to_internal_value` for the key of a row, as the pin answers it: no row for null or an empty string (a
/// field that allows null reads `""` as null); otherwise the text its messages quote and the UUID. A bool is refused by DRF itself;
/// an integer is read as a UUID's number (so a negative one is no UUID), a string as the text of a UUID (Python's `uuid.UUID` takes
/// braces, hyphens anywhere, `urn:` and `uuid:`), and anything else (a float, a list, an object) is no UUID, written as Python
/// writes it. The messages are DRF's and Django's. An integer above 2^64 reaches this as a float (`serde_json` keeps no more), so it
/// is told as one.
pub(crate) fn key_field(v: &Value) -> Result<Option<(String, Uuid)>, String> {
    let not_a_uuid = |text: &str| format!("\u{201c}{text}\u{201d} is not a valid UUID.");
    match v {
        Value::Null => Ok(None),
        Value::String(s) if s.is_empty() => Ok(None),
        Value::Bool(_) => Err("Incorrect type. Expected pk value, received bool.".into()),
        Value::String(s) => {
            let hex: String = s.replace("urn:", "").replace("uuid:", "").trim_matches(['{', '}']).replace('-', "");
            Uuid::parse_str(&hex).map(|id| Some((s.clone(), id))).map_err(|_| not_a_uuid(s))
        }
        Value::Number(n) => match n.as_u64() {
            Some(n) => Ok(Some((n.to_string(), Uuid::from_u128(u128::from(n))))),
            None => Err(not_a_uuid(&n.to_string())),
        },
        other => Err(not_a_uuid(&py_repr(other))),
    }
}

/// DRF `PrimaryKeyRelatedField` to a user (a project's lead and default assignee): no user, or the user the key names.
async fn user_field(app: &App, v: &Value) -> Result<Option<UserId>, String> {
    let Some((text, id)) = key_field(v)? else { return Ok(None) };
    match app.ports.db.user_by_id(UserId(id)).await {
        Ok(Some(_)) => Ok(Some(UserId(id))),
        Ok(None) => Err(format!("Invalid pk \"{text}\" - object does not exist.")),
        Err(_) => Err("Invalid pk - object does not exist.".into()),
    }
}

/// DRF `PrimaryKeyRelatedField` to a table that has no rows the app can name yet (the estimate, the default state, the cover
/// image's asset): no row (null, or an empty string), or a key that names nothing. A `default_state` that names one of the
/// project's own states is refused here, which the pin accepts (D-027).
pub(crate) fn nothing_field(v: &Value) -> Result<(), String> {
    match key_field(v)? {
        None => Ok(()),
        Some((text, _)) => Err(format!("Invalid pk \"{text}\" - object does not exist.")),
    }
}

/// The fields of the pin's `ProjectSerializer` (`fields = "__all__"`) that a request may set, checked all at once. On creation the
/// name and the key are required; on a patch only what is named is checked. A field the serializer does not know, or knows as
/// read-only (`id`, `workspace`, the times, who made it, `deleted_at`), is ignored, `cover_image_url` among them: the pin's is a
/// property. The name and the key are each checked for the forbidden characters and then for being taken (excluding `except`, the
/// project being changed), as `validate_name` and `validate_identifier` do.
pub(crate) async fn project_patch(
    app: &App,
    scope: &WorkspaceScope,
    b: &Map<String, Value>,
    except: Option<ProjectId>,
) -> Result<ProjectPatch, FieldErrors> {
    let partial = except.is_some();
    let mut errors = FieldErrors::new();
    let mut p = ProjectPatch::default();

    // the two that are required, and whose own validators come after the field's
    let mut checked = |field: &str, max: usize, special: &str| -> Option<String> {
        match b.get(field) {
            None if !partial => {
                fail(&mut errors, field, "This field is required.");
                None
            }
            None => None,
            Some(v) => match char_field(v, Some(max), false, false) {
                Ok(Some(text)) if thrust_core::has_forbidden_project_chars(&text) => {
                    fail(&mut errors, field, special);
                    None
                }
                Ok(text) => text,
                Err(m) => {
                    fail(&mut errors, field, m);
                    None
                }
            },
        }
    };
    let name = checked("name", thrust_core::MAX_PROJECT_NAME, "PROJECT_NAME_CANNOT_CONTAIN_SPECIAL_CHARACTERS");
    let identifier = checked("identifier", thrust_core::MAX_PROJECT_IDENTIFIER, "PROJECT_IDENTIFIER_CANNOT_CONTAIN_SPECIAL_CHARACTERS");
    if name.is_some() || identifier.is_some() {
        match thrust_core::project_clashes(app, scope, name.as_deref(), identifier.as_deref(), except).await {
            Ok(c) => {
                if c.name {
                    fail(&mut errors, "name", "PROJECT_NAME_ALREADY_EXIST");
                }
                if c.identifier {
                    fail(&mut errors, "identifier", "PROJECT_IDENTIFIER_ALREADY_EXIST");
                }
            }
            Err(e) => {
                fail(&mut errors, "name", format!("{e}"));
            }
        }
    }
    p.name = name;
    p.identifier = identifier;

    macro_rules! text {
        ($field:literal, $slot:expr, $max:expr, $null:expr) => {
            if let Some(v) = b.get($field) {
                match char_field(v, $max, true, $null) {
                    Ok(text) => $slot = Some(text),
                    Err(m) => fail(&mut errors, $field, m),
                }
            }
        };
    }
    if let Some(v) = b.get("description") {
        match char_field(v, None, true, false) {
            Ok(text) => p.description = text,
            Err(m) => fail(&mut errors, "description", m),
        }
    }
    text!("emoji", p.emoji, Some(255), true);
    text!("cover_image", p.cover_image, None, true);
    text!("external_source", p.external_source, Some(255), true);
    text!("external_id", p.external_id, Some(255), true);

    for (field, slot) in [("description_text", &mut p.description_text), ("description_html", &mut p.description_html), ("icon_prop", &mut p.icon_prop)] {
        if let Some(v) = b.get(field) {
            let v = if field == "description_html" { html_text(v) } else { v.clone() };
            match json_field(&v, true) {
                Ok(value) => *slot = Some(value),
                Err(m) => fail(&mut errors, field, m),
            }
        }
    }
    if let Some(v) = b.get("logo_props") {
        match json_field(v, false) {
            // the pin stores any value; the column here holds an object, which is all the app ever sends (D-027)
            Ok(Some(value)) if !value.is_object() => fail(&mut errors, "logo_props", "The logo of a project must be a JSON object."),
            Ok(value) => p.logo_props = value,
            Err(m) => fail(&mut errors, "logo_props", m),
        }
    }
    if let Some(v) = b.get("network") {
        match choice_field(v, &["0", "2"]) {
            Ok(n) => p.network = Some(if n == "0" { ProjectNetwork::Secret } else { ProjectNetwork::Public }),
            Err(m) => fail(&mut errors, "network", m),
        }
    }
    for (field, slot) in [("project_lead", &mut p.project_lead), ("default_assignee", &mut p.default_assignee)] {
        if let Some(v) = b.get(field) {
            match user_field(app, v).await {
                // D-026: the pin lets anyone lead a project; Thrust's project members are workspace members, so the lead must be one
                Ok(Some(user)) if field == "project_lead" && !thrust_core::is_workspace_member(app, scope, user).await.unwrap_or(false) => {
                    fail(&mut errors, field, "The lead of a project must be a member of its workspace.")
                }
                Ok(user) => *slot = Some(user),
                Err(m) => fail(&mut errors, field, m),
            }
        }
    }
    for field in ["estimate", "default_state", "cover_image_asset"] {
        if let Some(Err(m)) = b.get(field).map(nothing_field) {
            fail(&mut errors, field, m);
        }
    }
    for (field, slot) in [
        ("cycle_view", &mut p.cycle_view),
        ("issue_views_view", &mut p.issue_views_view),
        ("module_view", &mut p.module_view),
        ("page_view", &mut p.page_view),
        ("is_time_tracking_enabled", &mut p.is_time_tracking_enabled),
        ("is_issue_type_enabled", &mut p.is_issue_type_enabled),
        ("guest_view_all_features", &mut p.guest_view_all_features),
    ] {
        if let Some(v) = b.get(field) {
            match bool_field(v) {
                Ok(on) => *slot = Some(on),
                Err(m) => fail(&mut errors, field, m),
            }
        }
    }
    // `intake_view` is a writable column on creation; a patch takes it from `inbox_view`, whatever it says itself
    let intake = if partial { b.get("inbox_view") } else { b.get("intake_view") };
    if let Some(v) = intake {
        match bool_field(v) {
            Ok(on) => p.intake_view = Some(on),
            Err(m) => fail(&mut errors, "intake_view", m),
        }
    }
    for (field, slot) in [("archive_in", &mut p.archive_in), ("close_in", &mut p.close_in)] {
        if let Some(v) = b.get(field) {
            match months_field(v) {
                Ok(n) => *slot = Some(n),
                Err(m) => fail(&mut errors, field, m),
            }
        }
    }
    if let Some(v) = b.get("timezone") {
        match v {
            Value::Null => fail(&mut errors, "timezone", "This field may not be null."),
            v if thrust_core::is_known_timezone(&py_str(v)) => p.timezone = Some(py_str(v)),
            v => fail(&mut errors, "timezone", format!("\"{}\" is not a valid choice.", py_str(v))),
        }
    }

    if errors.is_empty() {
        Ok(p)
    } else {
        Err(errors)
    }
}

/// A project creation or change that the database refused for a name or key that is taken: another request got there first.
fn taken(constraint: &str) -> PlaneError {
    let (field, message) = if constraint.contains("identifier") {
        ("identifier", "PROJECT_IDENTIFIER_ALREADY_EXIST")
    } else {
        ("name", "PROJECT_NAME_ALREADY_EXIST")
    };
    let mut errors = FieldErrors::new();
    fail(&mut errors, field, message);
    PlaneError::Fields(errors)
}

pub(crate) fn saved(e: ProjectError) -> PlaneError {
    match e {
        ProjectError::Core(CoreError::Conflict(constraint)) => taken(&constraint),
        other => refusal(other),
    }
}

/// S30. Make a project; the caller becomes its admin. An admin or a member of the workspace may (a guest and a stranger are
/// refused by the pin's decorator).
pub(crate) async fn create(
    Caller(who): Caller,
    Extension(app): Extension<App>,
    Path(slug): Path<String>,
    body: Bytes,
) -> Result<(StatusCode, Json<Value>), PlaneError> {
    let scope = decorated(&app, &who, slug, &[Role::Member, Role::Admin]).await?;
    let b = object_body(&body)?;
    let patch = project_patch(&app, &scope, &b, None).await.map_err(PlaneError::Fields)?;
    let detail = thrust_core::create_project(&app, &scope, patch).await.map_err(saved)?;
    Ok((StatusCode::CREATED, Json(project_json(&detail))))
}

/// S32. Change a project. The workspace's admins and the project's own admins may; the checks come before the request's fields are
/// read, as on the pin.
pub(crate) async fn update(
    Caller(who): Caller,
    Extension(app): Extension<App>,
    Path((slug, project)): Path<(String, String)>,
    body: Bytes,
) -> Result<Json<Value>, PlaneError> {
    let Some(scope) = scope(&app, &who, slug).await? else { return Err(refused_by_decorator()) };
    let id = project_id(&project)?;
    thrust_core::updatable_project(&app, &scope, id).await.map_err(|e| match e {
        ProjectError::NotFound => missing_object(),
        other => refusal(other),
    })?;
    let b = object_body(&body)?;
    let patch = project_patch(&app, &scope, &b, Some(id)).await.map_err(PlaneError::Fields)?;
    let detail = thrust_core::update_project(&app, &scope, id, patch).await.map_err(saved)?;
    Ok(Json(project_json(&detail)))
}

/// S34. Open a project.
pub(crate) async fn retrieve(
    Caller(who): Caller,
    Extension(app): Extension<App>,
    Path((slug, project)): Path<(String, String)>,
) -> Result<Json<Value>, PlaneError> {
    let scope = decorated(&app, &who, slug, &ALL_ROLES).await?;
    let detail = thrust_core::open_project(&app, &scope, project_id(&project)?).await.map_err(refusal)?;
    Ok(Json(project_json(&detail)))
}

/// What `fields` and `project_ids` ask `project-stats` for, and the counts there are.
const STAT_FIELDS: [&str; 5] = ["total_issues", "completed_issues", "total_members", "total_cycles", "total_modules"];

/// S35. Counts for the projects of the workspace, all of them whoever asks (the pin does not filter by membership), or those named
/// by `project_ids`, and the counts named by `fields` (all five when none is a count). Nothing completes, cycles or modules yet
/// (D-021), so those say 0.
pub(crate) async fn stats(
    Caller(who): Caller,
    Extension(app): Extension<App>,
    Path(slug): Path<String>,
    Query(q): Query<HashMap<String, String>>,
) -> Result<Json<Value>, PlaneError> {
    let scope = decorated(&app, &who, slug, &ALL_ROLES).await?;
    let asked: Vec<&str> = q.get("fields").map(|f| f.split(',').collect()).unwrap_or_default();
    let mut fields: Vec<&str> = STAT_FIELDS.iter().copied().filter(|f| asked.contains(f)).collect();
    if fields.is_empty() {
        fields = STAT_FIELDS.to_vec();
    }
    let only: Option<Vec<ProjectId>> = match q.get("project_ids").filter(|s| !s.is_empty()) {
        None => None,
        Some(list) => Some(
            list.split(',')
                // the pin's `ValidationError` for a value that is no id is answered with its own words (fixture S35)
                .map(|id| id.parse::<ProjectId>().map_err(|_| PlaneError::BadRequest(json!({"error": "Please provide valid detail"}))))
                .collect::<Result<_, _>>()?,
        ),
    };
    let stats = thrust_core::project_stats(&app, &scope, only.as_deref()).await?;
    Ok(Json(Value::Array(
        stats
            .iter()
            .map(|s| {
                let mut row = Map::new();
                row.insert("id".into(), json!(s.id.0));
                for f in &fields {
                    let n = match *f {
                        "total_issues" => s.total_issues,
                        "total_members" => s.total_members,
                        _ => 0,
                    };
                    row.insert((*f).to_string(), json!(n));
                }
                Value::Object(row)
            })
            .collect(),
    )))
}

// --- who is in a project and what they have set -----------------------------------------------------------------------------

fn default_view_props() -> Value {
    json!({
        "filters": {
            "priority": null, "state": null, "state_group": null, "assignees": null, "created_by": null,
            "labels": null, "start_date": null, "target_date": null, "subscriber": null,
        },
        "display_filters": {
            "group_by": null, "order_by": "-created_at", "type": null, "sub_issue": true, "show_empty_groups": true,
            "layout": "list", "calendar_date_range": "",
        },
    })
}

fn default_preferences() -> Value {
    json!({"pages": {"block_display": true}, "navigation": {"default_tab": "work_items", "hide_in_more_menu": []}})
}

/// The pin's default settings with what the row has set laid over them, key by key.
fn filled(stored: &Value, default: Value) -> Value {
    let mut out = default.as_object().cloned().unwrap_or_default();
    for (k, v) in stored.as_object().into_iter().flatten() {
        out.insert(k.clone(), v.clone());
    }
    Value::Object(out)
}

/// S36. The caller's own row in a project, with who they are, the project and the workspace. A caller who has none (a stranger, a
/// workspace member who is not in it, a project that is not there) is told the pin's 404 for a missing object.
pub(crate) async fn member_me(
    Caller(who): Caller,
    Extension(app): Extension<App>,
    Path((slug, project)): Path<(String, String)>,
) -> Result<Json<Value>, PlaneError> {
    let Some(scope) = scope(&app, &who, slug).await? else { return Err(missing_object()) };
    let Some(m) = thrust_core::my_project_member(&app, &scope, project_id(&project)?).await? else { return Err(missing_object()) };
    let (row, u, p, w) = (&m.member, &m.user, &m.project, &m.workspace);
    Ok(Json(json!({
        "id": row.id,
        "workspace": {"id": w.id.0, "name": w.name, "slug": w.slug.as_str(), "logo_url": null},
        "project": {
            "id": p.id.0, "identifier": p.identifier, "name": p.name, "cover_image": p.cover_image, "cover_image_url": p.cover_image,
            "logo_props": p.logo_props, "description": p.description,
        },
        "member": {
            "id": u.id.0, "first_name": u.first_name, "last_name": u.last_name, "avatar": u.avatar, "avatar_url": non_empty(&u.avatar),
            "display_name": u.display_name, "is_bot": false,
        },
        "comment": row.comment,
        "role": role_number(row.role),
        "view_props": filled(&row.view_props, default_view_props()),
        "default_props": filled(&row.default_props, default_view_props()),
        "preferences": filled(&row.preferences, default_preferences()),
        "sort_order": row.sort_order,
        "is_active": true,
        "created_at": ts(row.created_at),
        "updated_at": ts(row.updated_at),
        "created_by": id_or_null(row.created_by),
        "updated_by": null,
        "deleted_at": null,
    })))
}

/// The caller's way into a project for a view guarded by the decorator: any member of the project.
async fn project_scope(app: &App, who: &Principal, slug: String, project: &str) -> Result<Scope, PlaneError> {
    let id = project_id(project)?;
    match thrust_core::authorize(app, who, &WorkspaceSlug::new(slug), id, Permission::ReadProject).await {
        Ok(scope) => Ok(scope),
        Err(CoreError::NotFound | CoreError::Forbidden) => Err(refused_by_decorator()),
        Err(e) => Err(e.into()),
    }
}

/// S37. The caller's work-item list settings for the project; made on the first ask if joining the project did not.
pub(crate) async fn user_properties(
    Caller(who): Caller,
    Extension(app): Extension<App>,
    Path((slug, project)): Path<(String, String)>,
) -> Result<Json<Value>, PlaneError> {
    let scope = project_scope(&app, &who, slug, &project).await?;
    let p = thrust_core::project_properties(&app, &scope).await?;
    Ok(Json(json!({
        "id": p.id,
        "workspace": p.workspace_id.0,
        "project": p.project_id.0,
        "user": p.user_id.0,
        "filters": p.filters,
        "display_filters": p.display_filters,
        "display_properties": p.display_properties,
        "rich_filters": p.rich_filters,
        "preferences": p.preferences,
        "sort_order": p.sort_order,
        "created_at": ts(p.created_at),
        "updated_at": ts(p.updated_at),
        "created_by": p.user_id.0,
        "updated_by": null,
        "deleted_at": null,
    })))
}

/// S38. The project's labels, by `sort_order`. The pin guards it with a permission class that lets in any member of the workspace
/// and lists only the projects the caller is in, so a member who is not in the project, and a project that is not there, are `[]`.
pub(crate) async fn labels(
    Caller(who): Caller,
    Extension(app): Extension<App>,
    Path((slug, project)): Path<(String, String)>,
) -> Result<Json<Value>, PlaneError> {
    if scope(&app, &who, slug.clone()).await?.is_none() {
        return Err(refused_by_class());
    }
    let Ok(scope) = project_scope(&app, &who, slug, &project).await else { return Ok(Json(json!([]))) };
    let labels = thrust_core::project_labels(&app, &scope).await?;
    Ok(Json(Value::Array(
        labels
            .iter()
            .map(|l| {
                json!({
                    "id": l.id, "project_id": l.project_id.0, "workspace_id": l.workspace_id.0, "parent": l.parent_id, "name": l.name,
                    "color": l.color, "sort_order": l.sort_order,
                })
            })
            .collect(),
    )))
}

/// S39. The people in the project, newest row first.
pub(crate) async fn members(
    Caller(who): Caller,
    Extension(app): Extension<App>,
    Path((slug, project)): Path<(String, String)>,
) -> Result<Json<Value>, PlaneError> {
    let scope = project_scope(&app, &who, slug, &project).await?;
    let rows = thrust_core::project_members(&app, &scope).await?;
    Ok(Json(Value::Array(
        rows.iter()
            .map(|m| {
                json!({
                    "id": m.id, "member": m.user_id.0, "role": role_number(m.role), "original_role": role_number(m.role),
                    "project": m.project_id.0, "created_at": ts(m.created_at),
                })
            })
            .collect(),
    )))
}

// --- states ------------------------------------------------------------------------------------------------------------------

/// A state as the pin's `StateSerializer` shows it. `order` is only there when the route works one out.
pub(crate) fn state_json(s: &StateRow, order: Option<f64>) -> Value {
    let mut row = json!({
        "id": s.id,
        "project_id": s.project_id.0,
        "workspace_id": s.workspace_id.0,
        "name": s.name,
        "color": s.color,
        "group": s.group.as_str(),
        "default": s.is_default,
        "description": s.description,
        "sequence": s.sequence,
    });
    if let Some(order) = order {
        row["order"] = json!(order);
    }
    row
}

/// The pin's `order` of each state of a list: its place among the states of its group, over how many there are (`index / count`,
/// from 1), in the order the list has them.
pub(crate) fn state_orders(states: &[StateRow]) -> Vec<f64> {
    let mut count: HashMap<StateGroup, usize> = HashMap::new();
    for s in states {
        *count.entry(s.group).or_default() += 1;
    }
    let mut seen: HashMap<StateGroup, usize> = HashMap::new();
    states
        .iter()
        .map(|s| {
            let i = seen.entry(s.group).or_default();
            *i += 1;
            *i as f64 / count[&s.group] as f64
        })
        .collect()
}

/// S40. The project's states, the intake one left out.
pub(crate) async fn states(
    Caller(who): Caller,
    Extension(app): Extension<App>,
    Path((slug, project)): Path<(String, String)>,
) -> Result<Json<Value>, PlaneError> {
    let scope = project_scope(&app, &who, slug, &project).await?;
    let states = thrust_core::project_states(&app, &scope).await?;
    let orders = state_orders(&states);
    Ok(Json(Value::Array(states.iter().zip(orders).map(|(s, o)| state_json(s, Some(o))).collect())))
}

/// S41. The project's intake state: the one the lists leave out.
pub(crate) async fn intake_state(
    Caller(who): Caller,
    Extension(app): Extension<App>,
    Path((slug, project)): Path<(String, String)>,
) -> Result<Json<Value>, PlaneError> {
    let scope = project_scope(&app, &who, slug, &project).await?;
    match thrust_core::intake_state(&app, &scope).await {
        Ok(state) => Ok(Json(state_json(&state, None))),
        Err(CoreError::NotFound) => Err(PlaneError::NotFound(json!({"error": "Triage state not found"}))),
        Err(e) => Err(e.into()),
    }
}

// --- stubs -------------------------------------------------------------------------------------------------------------------

/// S42. The project's estimates. A stub (D-021): there are none. The pin guards it with a permission class that lets in only
/// members of the project.
pub(crate) async fn estimates(
    Caller(who): Caller,
    Extension(app): Extension<App>,
    Path((slug, project)): Path<(String, String)>,
) -> Result<Json<Value>, PlaneError> {
    project_scope(&app, &who, slug, &project).await.map_err(|_| refused_by_class())?;
    Ok(Json(json!([])))
}

/// S43 (cycles), S45 (the project's modules) and S46 (views). Stubs (D-021): there are none.
pub(crate) async fn project_stub(
    Caller(who): Caller,
    Extension(app): Extension<App>,
    Path((slug, project)): Path<(String, String)>,
) -> Result<Json<Value>, PlaneError> {
    project_scope(&app, &who, slug, &project).await?;
    Ok(Json(json!([])))
}

/// S44. The workspace's modules. A stub (D-021): there are none. Any active member of the workspace may ask.
pub(crate) async fn workspace_modules(Caller(who): Caller, Extension(app): Extension<App>, Path(slug): Path<String>) -> Result<Json<Value>, PlaneError> {
    scope(&app, &who, slug).await?.ok_or_else(refused_by_class)?;
    Ok(Json(json!([])))
}
