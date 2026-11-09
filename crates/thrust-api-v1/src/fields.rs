//! Field validation the way Django REST Framework does it, for the request bodies of the v1 routes.
//!
//! DRF checks every field it knows, collects what is wrong into `{field: [message]}`, and answers a 400 with that. A field it does
//! not know is ignored (which is how v1 ignores a `network` on a project: the serializer has no such field). Each kind below is the
//! DRF field the pinned serializer builds for the model column, with DRF's default messages.
//!
//! The kinds and their messages are the ones `thrust-compat-plane` has for the internal API's bodies (`fields.rs`, recorded from the
//! pin there); they are copied, not shared, so that the two Plane-shaped crates stay independent (#52). The messages the corpus
//! records are exact; the rest are DRF's defaults, read from its source.

use serde_json::{json, Map, Value};
use thrust_contracts::Uuid;

use crate::V1Error;

/// `{field: [message]}`, the body of a DRF 400.
pub type FieldErrors = Map<String, Value>;

pub fn fail(errors: &mut FieldErrors, field: &str, message: impl Into<String>) {
    errors.insert(field.to_string(), json!([message.into()]));
}

/// Read a field if the body names it, and file what is wrong with it under its name. `None` is "not named", or "named and wrong".
pub fn read<T>(
    errors: &mut FieldErrors,
    body: &Map<String, Value>,
    field: &str,
    f: impl FnOnce(&Value) -> Result<T, String>,
) -> Option<T> {
    let v = body.get(field)?;
    match f(v) {
        Ok(x) => Some(x),
        Err(message) => {
            fail(errors, field, message);
            None
        }
    }
}

/// What DRF answers when a body that should be an object is a list.
pub fn not_a_dictionary() -> V1Error {
    let mut errors = FieldErrors::new();
    errors.insert("non_field_errors".into(), json!(["Invalid data. Expected a dictionary, but got list."]));
    V1Error::Fields(errors)
}

/// A 400 for the fields that failed, or nothing when none did.
pub fn refuse_if_any(errors: FieldErrors) -> Result<(), V1Error> {
    if errors.is_empty() {
        Ok(())
    } else {
        Err(V1Error::Fields(errors))
    }
}

/// DRF `CharField` (and `TextField`): trims, refuses blank and null unless the column allows them, accepts a number as its text,
/// refuses a bool, a list and an object, and counts the length in characters after trimming.
pub fn char_field(
    v: &Value,
    max: Option<usize>,
    allow_blank: bool,
    allow_null: bool,
) -> Result<Option<String>, String> {
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

/// Whether the body's `name` is text that DRF trimmed to something else (`validated` is what it made of it). The pin's creations look
/// the row that has a taken name up by `request.data.get("name")`, the text as it was sent, so a name with spaces round it clashes
/// in the database and then finds no row to name in the 409: the view raises, and the base view answers 500.
pub fn name_was_padded(body: &Map<String, Value>, validated: Option<&str>) -> bool {
    matches!((body.get("name"), validated), (Some(Value::String(sent)), Some(kept)) if sent != kept)
}

/// DRF `BooleanField`: a real bool, or one of its spellings (`"yes"`, `"off"`, `1`, ...). Anything else is refused.
pub fn bool_field(v: &Value) -> Result<bool, String> {
    const TRUE: &[&str] = &["t", "T", "y", "Y", "yes", "Yes", "YES", "true", "True", "TRUE", "on", "On", "ON", "1"];
    const FALSE: &[&str] = &["f", "F", "n", "N", "no", "No", "NO", "false", "False", "FALSE", "off", "Off", "OFF", "0"];
    match v {
        Value::Bool(b) => Ok(*b),
        Value::Null => Err("This field may not be null.".into()),
        Value::String(s) if TRUE.contains(&s.as_str()) => Ok(true),
        Value::String(s) if FALSE.contains(&s.as_str()) => Ok(false),
        Value::Number(n) if n.as_f64() == Some(1.0) => Ok(true),
        Value::Number(n) if n.as_f64() == Some(0.0) => Ok(false),
        _ => Err("Must be a valid boolean.".into()),
    }
}

/// What DRF's `IntegerField` makes of a string: it cuts a trailing `.0` (any number of zeros), and then Python's `int()` reads the
/// rest, which allows a sign and spaces around it.
fn decimal_text(s: &str) -> Option<i128> {
    let t = s.trim_end();
    let whole = match t.split_once('.') {
        Some((whole, zeros)) if zeros.bytes().all(|b| b == b'0') => whole,
        _ => t,
    };
    whole.trim().parse().ok()
}

/// DRF `IntegerField` between `min` and `max` (the validators of the pin's model fields): a float is read through its text, so one
/// with no fraction is an integer.
pub fn int_field(v: &Value, min: i64, max: i64) -> Result<i64, String> {
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
    if n > i128::from(max) {
        Err(format!("Ensure this value is less than or equal to {max}."))
    } else if n < i128::from(min) {
        Err(format!("Ensure this value is greater than or equal to {min}."))
    } else {
        i64::try_from(n).map_err(|_| "A valid integer is required.".to_string())
    }
}

/// Python's `str()` of a JSON value, which DRF puts into "is not a valid choice".
pub fn py_str(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Bool(true) => "True".into(),
        Value::Bool(false) => "False".into(),
        Value::Null => "None".into(),
        other => other.to_string(),
    }
}

/// DRF `ChoiceField`: the value's text must be one of the choices; the choice is returned.
pub fn choice_field<'a>(v: &Value, choices: &'a [&'a str]) -> Result<&'a str, String> {
    if v.is_null() {
        return Err("This field may not be null.".into());
    }
    let text = py_str(v);
    choices.iter().find(|c| **c == text).copied().ok_or_else(|| format!("\"{text}\" is not a valid choice."))
}

/// DRF `FloatField`: Python's `float()` of the value. A number is itself, a bool is 0 or 1 (Python's `float(True)`), a string is read
/// after trimming, and anything else is no number. A value that is not finite is refused here (the pin keeps a NaN, which no JSON
/// can say back).
pub fn float_field(v: &Value) -> Result<f64, String> {
    let invalid = || "A valid number is required.".to_string();
    let n = match v {
        Value::Null => return Err("This field may not be null.".into()),
        Value::Number(n) => n.as_f64().ok_or_else(invalid)?,
        Value::Bool(b) => f64::from(u8::from(*b)),
        Value::String(s) => s.trim().parse::<f64>().map_err(|_| invalid())?,
        _ => return Err(invalid()),
    };
    if n.is_finite() {
        Ok(n)
    } else {
        Err(invalid())
    }
}

/// DRF `UUIDField` for the key of a row: null is none, and 32 hex digits in any hyphenation are a UUID (DRF strips `urn:`, `uuid:`
/// and braces and drops the hyphens, as Python's `uuid.UUID(hex=...)` does).
pub fn uuid_field(v: &Value) -> Result<Option<Uuid>, String> {
    match v {
        Value::Null => Ok(None),
        Value::String(s) => uuid_hex(s).map(Some).ok_or_else(|| "Must be a valid UUID.".to_string()),
        _ => Err("Must be a valid UUID.".into()),
    }
}

/// Python's `uuid.UUID(hex=s)`: `urn:` and `uuid:` and the braces and hyphens are dropped, and what is left must be 32 hex digits.
fn uuid_hex(s: &str) -> Option<Uuid> {
    let hex: String = s.replace("urn:", "").replace("uuid:", "").trim_matches(['{', '}']).replace('-', "");
    Uuid::parse_str(&hex).ok()
}

/// What DRF's `PrimaryKeyRelatedField` makes of a body's value for a model whose key is a UUID. It looks the row up with
/// `queryset.get(pk=value)`, and that is where the kinds part: a value the key column cannot read makes Django raise its own
/// `ValidationError`, which the field does not catch (it catches only a missing row, a `TypeError` and a `ValueError`).
#[derive(Debug, PartialEq, Eq)]
pub enum PkRead {
    /// `null`, or the empty string, which a relation reads as none.
    Null,
    /// A key a row may have. `shown` is the value as DRF prints it: `Invalid pk "<shown>" - object does not exist.`
    Key { id: Uuid, shown: String },
    /// DRF's own refusal, filed under the field's name.
    Refused(String),
    /// Django's `ValidationError`, which no serializer field answers: the base view makes it a 400 `{"error": "Please provide valid
    /// detail"}`, and the fields after this one are not read.
    Raises,
}

pub fn pk_field(v: &Value) -> PkRead {
    match v {
        Value::Null => PkRead::Null,
        Value::String(s) if s.is_empty() => PkRead::Null,
        // `get(pk=True)` is a TypeError in DRF's own check
        Value::Bool(_) => PkRead::Refused("Incorrect type. Expected pk value, received bool.".into()),
        Value::String(s) => uuid_hex(s).map_or(PkRead::Raises, |id| PkRead::Key { id, shown: s.clone() }),
        // an integer is read as the number of the key (`UUID(int=n)`); a float, a list and an object are no key at all
        Value::Number(n) => n
            .as_u64()
            .map_or(PkRead::Raises, |n| PkRead::Key { id: Uuid::from_u128(u128::from(n)), shown: n.to_string() }),
        Value::Array(_) | Value::Object(_) => PkRead::Raises,
    }
}

/// DRF `PrimaryKeyRelatedField`'s message for a key that names no row.
pub fn no_such_pk(id: Uuid) -> String {
    no_such_pk_shown(&id.to_string())
}

/// The same message for a key as the request spelled it (DRF prints the value it was given, not the UUID it made of it).
pub fn no_such_pk_shown(shown: &str) -> String {
    format!("Invalid pk \"{shown}\" - object does not exist.")
}

/// Python's `int()` of a query parameter: optional spaces, an optional sign, digits with single underscores between them.
pub fn py_int(s: &str) -> Option<i64> {
    let t = s.trim();
    let (sign, digits) = match t.strip_prefix('-') {
        Some(rest) => (-1i64, rest),
        None => (1, t.strip_prefix('+').unwrap_or(t)),
    };
    let well_formed = !digits.is_empty()
        && !digits.starts_with('_')
        && !digits.ends_with('_')
        && !digits.contains("__")
        && digits.bytes().all(|b| b.is_ascii_digit() || b == b'_');
    if !well_formed {
        return None;
    }
    let n: i64 = digits.replace('_', "").parse().ok()?;
    sign.checked_mul(n)
}
