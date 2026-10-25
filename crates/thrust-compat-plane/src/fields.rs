//! Field validation the way Django REST Framework does it, for the request bodies of the user and profile routes.
//!
//! DRF checks every field it knows, collects what is wrong into `{field: [message]}`, and answers a 400 with that. A field it
//! does not know is ignored. Each kind below is the DRF field the pinned serializer builds for the model column, with DRF's
//! default messages. The messages the fixtures record are exact; the others (marked) are DRF's defaults as read from its
//! source, not recorded from the pin.

use serde_json::{json, Map, Value};
use thrust_contracts::*;
use thrust_ports::ProfilePatch;

/// `{field: [message]}`, the body of a DRF 400.
pub(crate) type FieldErrors = Map<String, Value>;

pub(crate) fn fail(errors: &mut FieldErrors, field: &str, message: impl Into<String>) {
    errors.insert(field.to_string(), json!([message.into()]));
}

/// Python's `str()` of a JSON value, which DRF puts into "is not a valid choice" and "is not a valid boolean".
pub(crate) fn py_str(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Bool(true) => "True".into(),
        Value::Bool(false) => "False".into(),
        Value::Null => "None".into(),
        other => other.to_string(),
    }
}

/// DRF `CharField` (and `TextField`): trims, refuses blank and null unless the column allows them, accepts a number as its
/// text, refuses a bool, a list and an object, and counts the length in characters after trimming.
pub(crate) fn char_field(v: &Value, max: Option<usize>, allow_blank: bool, allow_null: bool) -> Result<Option<String>, String> {
    let text = match v {
        Value::Null if allow_null => return Ok(None),
        Value::Null => return Err("This field may not be null.".into()),
        Value::Bool(_) | Value::Array(_) | Value::Object(_) => return Err("Not a valid string.".into()),
        Value::String(s) => s.trim().to_string(),
        n => n.to_string().trim().to_string(),
    };
    if text.is_empty() && !allow_blank {
        return Err("This field may not be blank.".into());
    }
    if let Some(max) = max {
        if text.chars().count() > max {
            return Err(format!("Ensure this field has no more than {max} characters."));
        }
    }
    Ok(Some(text))
}

/// DRF `BooleanField`: a real bool, or one of its spellings (`"yes"`, `"off"`, `1`, ...). Anything else is refused.
fn bool_field(v: &Value) -> Result<bool, String> {
    const TRUE: &[&str] = &["t", "T", "y", "Y", "yes", "Yes", "YES", "true", "True", "TRUE", "on", "On", "ON", "1"];
    const FALSE: &[&str] = &["f", "F", "n", "N", "no", "No", "NO", "false", "False", "FALSE", "off", "Off", "OFF", "0"];
    match v {
        Value::Bool(b) => Ok(*b),
        Value::Null => Err("This field may not be null.".into()),
        Value::String(s) if TRUE.contains(&s.as_str()) => Ok(true),
        Value::String(s) if FALSE.contains(&s.as_str()) => Ok(false),
        Value::Number(n) if n.as_f64() == Some(1.0) => Ok(true),
        Value::Number(n) if n.as_f64() == Some(0.0) => Ok(false),
        // recorded from the pin (S11): this DRF's message does not quote the value, unlike its choice message
        _ => Err("Must be a valid boolean.".into()),
    }
}

/// DRF `ChoiceField`: the value's text must be one of the choices; the choice is returned.
fn choice_field<'a>(v: &Value, choices: &'a [&'a str]) -> Result<&'a str, String> {
    if v.is_null() {
        return Err("This field may not be null.".into());
    }
    let text = py_str(v);
    choices.iter().find(|c| **c == text).copied().ok_or_else(|| format!("\"{text}\" is not a valid choice."))
}

/// DRF `UUIDField`: Python's `uuid.UUID(hex=...)` after DRF strips `urn:`, `uuid:` and braces and drops the hyphens, so
/// 32 hex digits in any hyphenation are one.
fn uuid_field(v: &Value) -> Result<Option<Uuid>, String> {
    match v {
        Value::Null => Ok(None),
        Value::String(s) => {
            // with the hyphens gone only exactly 32 hex digits parse, which is Python's check too
            let hex: String = s.replace("urn:", "").replace("uuid:", "").trim_matches(['{', '}']).replace('-', "");
            Uuid::parse_str(&hex).map(Some).map_err(|_| "Must be a valid UUID.".to_string())
        }
        _ => Err("Must be a valid UUID.".into()),
    }
}

/// DRF `URLField` (not recorded from the pin): an `http`, `https`, `ftp` or `ftps` URL with a host, no whitespace, within
/// the length. Django's own validator is stricter about hosts; this refuses what is plainly not a URL and no more.
fn url_field(v: &Value, max: usize) -> Result<Option<String>, String> {
    let Some(text) = char_field(v, Some(max), false, true)? else { return Ok(None) };
    let rest = ["http://", "https://", "ftp://", "ftps://"].iter().find_map(|s| text.strip_prefix(s));
    let host = rest.map(|r| r.split(['/', '?', '#']).next().unwrap_or_default()).unwrap_or_default();
    if host.is_empty() || text.chars().any(char::is_whitespace) {
        return Err("Enter a valid URL.".into());
    }
    Ok(Some(text))
}

/// The writable fields of `UserSerializer` that Thrust keeps: names, avatar, cover image, time zone. The rest of the pin's
/// writable fields (`avatar_asset`, `cover_image_asset`, and the system fields it leaves writable by mistake, such as
/// `last_login` and `bot_type`) are ignored.
pub(crate) fn user_patch(body: &Map<String, Value>) -> Result<ProfilePatch, FieldErrors> {
    let mut errors = FieldErrors::new();
    let mut patch = ProfilePatch::default();
    for (field, max, blank) in [("display_name", Some(255), false), ("first_name", Some(255), true), ("last_name", Some(255), true), ("avatar", None, true)] {
        let Some(v) = body.get(field) else { continue };
        match char_field(v, max, blank, false) {
            Err(m) => fail(&mut errors, field, m),
            Ok(s) => {
                let s = s.unwrap_or_default();
                // the pin's `validate_first_name` / `validate_last_name`
                let url_message = match field {
                    "first_name" => Some("First name cannot contain a URL."),
                    "last_name" => Some("Last name cannot contain a URL."),
                    _ => None,
                };
                if let Some(m) = url_message.filter(|_| thrust_core::contains_url(&s)) {
                    fail(&mut errors, field, m);
                    continue;
                }
                match field {
                    "display_name" => patch.display_name = Some(s),
                    "first_name" => patch.first_name = Some(s),
                    "last_name" => patch.last_name = Some(s),
                    _ => patch.avatar = Some(s),
                }
            }
        }
    }
    if let Some(v) = body.get("cover_image") {
        match url_field(v, 800) {
            Ok(url) => patch.cover_image = Some(url),
            Err(m) => fail(&mut errors, "cover_image", m),
        }
    }
    if let Some(v) = body.get("user_timezone") {
        // a choice field: the value's text must be one of the zones, and a refusal quotes that text
        if v.is_null() {
            fail(&mut errors, "user_timezone", "This field may not be null.");
        } else {
            let text = py_str(v);
            if thrust_core::is_known_timezone(&text) {
                patch.timezone = Some(text);
            } else {
                fail(&mut errors, "user_timezone", format!("\"{text}\" is not a valid choice."));
            }
        }
    }
    if errors.is_empty() {
        Ok(patch)
    } else {
        Err(errors)
    }
}

/// How a profile setting is validated.
enum Kind {
    Bool,
    /// Any JSON value, but not null (`theme`, `goals`, ...).
    Json,
    /// Any JSON value, null allowed (`billing_address`).
    JsonOrNull,
    /// `max` characters; blank and null as the column allows.
    Text { max: Option<usize>, blank: bool, null: bool },
    Choice(&'static [&'static str]),
}

/// The settings kept in `UserProfile::preferences`, by the name the client uses.
const PREFERENCES: &[(&str, Kind)] = &[
    ("theme", Kind::Json),
    ("is_app_rail_docked", Kind::Bool),
    ("use_case", Kind::Text { max: None, blank: true, null: true }),
    ("role", Kind::Text { max: Some(300), blank: true, null: true }),
    ("billing_address_country", Kind::Text { max: Some(255), blank: false, null: false }),
    ("billing_address", Kind::JsonOrNull),
    ("has_billing_address", Kind::Bool),
    ("company_name", Kind::Text { max: Some(255), blank: true, null: false }),
    ("notification_view_mode", Kind::Choice(&["full", "compact"])),
    ("is_smooth_cursor_enabled", Kind::Bool),
    ("is_mobile_onboarded", Kind::Bool),
    ("mobile_onboarding_step", Kind::Json),
    ("mobile_timezone_auto_set", Kind::Bool),
    ("language", Kind::Text { max: Some(255), blank: false, null: false }),
    ("start_of_the_week", Kind::Choice(&["0", "1", "2", "3", "4", "5", "6"])),
    ("goals", Kind::Json),
    ("background_color", Kind::Text { max: Some(255), blank: false, null: false }),
    ("is_navigation_tour_completed", Kind::Bool),
    ("has_marketing_email_consent", Kind::Bool),
    ("is_subscribed_to_changelog", Kind::Bool),
    ("product_tour", Kind::Json),
];

/// The writable fields of `ProfileSerializer` (every column except `user` and `id`, which are read only).
pub(crate) fn profile_patch(body: &Map<String, Value>) -> Result<ProfilePatch, FieldErrors> {
    let mut errors = FieldErrors::new();
    let mut patch = ProfilePatch::default();
    for (field, kind) in PREFERENCES {
        let Some(v) = body.get(*field) else { continue };
        let stored: Result<Value, String> = match kind {
            Kind::Bool => bool_field(v).map(Value::Bool),
            Kind::Json if v.is_null() => Err("This field may not be null.".into()),
            Kind::Json | Kind::JsonOrNull => Ok(v.clone()),
            Kind::Text { max, blank, null } => char_field(v, *max, *blank, *null).map(|s| s.map_or(Value::Null, Value::String)),
            Kind::Choice(choices) => choice_field(v, choices).map(|c| match *field {
                // an integer choice is stored as the integer
                "start_of_the_week" => json!(c.parse::<u8>().unwrap_or(0)),
                _ => json!(c),
            }),
        };
        match stored {
            Ok(value) => {
                patch.preferences.insert((*field).to_string(), value);
            }
            Err(m) => fail(&mut errors, field, m),
        }
    }
    for field in ["is_onboarded", "is_tour_completed"] {
        let Some(v) = body.get(field) else { continue };
        match bool_field(v) {
            Ok(b) if field == "is_onboarded" => patch.is_onboarded = Some(b),
            Ok(b) => patch.is_tour_completed = Some(b),
            Err(m) => fail(&mut errors, field, m),
        }
    }
    if let Some(v) = body.get("last_workspace_id") {
        match uuid_field(v) {
            Ok(id) => patch.last_workspace_id = Some(id.map(WorkspaceId)),
            Err(m) => fail(&mut errors, "last_workspace_id", m),
        }
    }
    if let Some(v) = body.get("onboarding_step") {
        match v {
            Value::Null => fail(&mut errors, "onboarding_step", "This field may not be null."),
            Value::Object(_) => patch.onboarding_step = Some(v.clone()),
            // the pin stores any JSON; Thrust keeps an object (a deviation, G-17)
            _ => fail(&mut errors, "onboarding_step", "Must be a JSON object."),
        }
    }
    if errors.is_empty() {
        Ok(patch)
    } else {
        Err(errors)
    }
}

/// Django's `BooleanField.to_python`, which a plain `profile.is_tour_completed = request.data.get(..., False)` then
/// `profile.save()` goes through: absent is `False`; `True`/`False`, `1`/`0`, `"t"`/`"True"`/`"1"` and `"f"`/`"False"`/`"0"`
/// are booleans; everything else, `null` included, is refused.
pub(crate) fn django_bool(v: Option<&Value>) -> Result<bool, ()> {
    match v {
        None => Ok(false),
        Some(Value::Bool(b)) => Ok(*b),
        Some(Value::Number(n)) if n.as_f64() == Some(1.0) => Ok(true),
        Some(Value::Number(n)) if n.as_f64() == Some(0.0) => Ok(false),
        Some(Value::String(s)) if ["t", "True", "1"].contains(&s.as_str()) => Ok(true),
        Some(Value::String(s)) if ["f", "False", "0"].contains(&s.as_str()) => Ok(false),
        _ => Err(()),
    }
}
