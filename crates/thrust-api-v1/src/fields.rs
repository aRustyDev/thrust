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
use thrust_contracts::chrono::{self, NaiveDate};
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

/// Python's `str()` of a JSON value, which DRF puts into "is not a valid choice" (and Django into "is not a valid UUID"). A list and a
/// dict print as Python's `repr` of them (`['a', 1]`, `{'a': 1}`), as `str()` of those does.
pub fn py_str(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Bool(true) => "True".into(),
        Value::Bool(false) => "False".into(),
        Value::Null => "None".into(),
        Value::Array(_) | Value::Object(_) => py_repr(v),
        Value::Number(n) if n.is_f64() => py_float(n.as_f64().unwrap_or_default()),
        other => other.to_string(),
    }
}

/// Python's `repr()` of a JSON value read as Python reads it: strings in quotes, `True`, `None`, floats as Python prints them.
pub fn py_repr(v: &Value) -> String {
    match v {
        Value::String(s) => py_string_repr(s),
        Value::Array(items) => format!("[{}]", items.iter().map(py_repr).collect::<Vec<_>>().join(", ")),
        Value::Object(map) => {
            let items: Vec<String> =
                map.iter().map(|(k, v)| format!("{}: {}", py_string_repr(k), py_repr(v))).collect();
            format!("{{{}}}", items.join(", "))
        }
        other => py_str(other),
    }
}

/// A string as Python's `repr` writes it: single quotes unless the text has one and no double quote, a backslash before the quote and
/// the backslash, `\n`, `\r`, `\t` and `\xNN` for the other control characters.
fn py_string_repr(s: &str) -> String {
    let quote = if s.contains('\'') && !s.contains('"') { '"' } else { '\'' };
    let mut out = String::with_capacity(s.len() + 2);
    out.push(quote);
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
            c if u32::from(c) < 0x20 || u32::from(c) == 0x7f => out.push_str(&format!("\\x{:02x}", u32::from(c))),
            c => out.push(c),
        }
    }
    out.push(quote);
    out
}

/// A float as Python's `repr` writes it: a point and a digit after it (`5.0`), an exponent with a sign and two digits from 1e16 up and
/// below 1e-4 (`1e+16`, `1.5e-05`).
fn py_float(f: f64) -> String {
    if f == 0.0 {
        return if f.is_sign_negative() { "-0.0".into() } else { "0.0".into() };
    }
    let magnitude = f.abs();
    if !(1e-4..1e16).contains(&magnitude) {
        let text = format!("{f:e}");
        return match text.split_once('e') {
            Some((mantissa, exponent)) => {
                let (sign, digits) = exponent.strip_prefix('-').map_or(('+', exponent), |d| ('-', d));
                format!("{mantissa}e{sign}{digits:0>2}")
            }
            None => text,
        };
    }
    let text = format!("{f}");
    if text.contains('.') {
        text
    } else {
        format!("{text}.0")
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
/// `queryset.get(pk=value)`, and that is where the kinds part: a missing row, a value that is no kind of key (a bool), and a value the
/// key column cannot read, for which Django raises its own `ValidationError` that the serializer then files under the field's name.
#[derive(Debug, PartialEq, Eq)]
pub enum PkRead {
    /// `null`, or the empty string, which a relation reads as none.
    Null,
    /// A key a row may have. `shown` is the value as DRF prints it: `Invalid pk "<shown>" - object does not exist.`
    Key { id: Uuid, shown: String },
    /// A refusal filed under the field's name: DRF's own (a bool is no key), or Django's `ValidationError` from reading the value as
    /// a UUID (`“nope” is not a valid UUID.`). The serializer catches the second, per field, as it does the first, so the other fields
    /// are read and reported too. (Measured on the pinned Plane; reading the source suggested the error escapes the field and makes a
    /// bare 400, which it does not.)
    Refused(String),
}

/// Django's message for a value that is no UUID, with the value as Python's `str()` prints it.
fn not_a_uuid(shown: &str) -> PkRead {
    PkRead::Refused(format!("\u{201c}{shown}\u{201d} is not a valid UUID."))
}

pub fn pk_field(v: &Value) -> PkRead {
    match v {
        Value::Null => PkRead::Null,
        Value::String(s) if s.is_empty() => PkRead::Null,
        // `get(pk=True)` is a TypeError in DRF's own check
        Value::Bool(_) => PkRead::Refused("Incorrect type. Expected pk value, received bool.".into()),
        Value::String(s) => uuid_hex(s).map_or_else(|| not_a_uuid(s), |id| PkRead::Key { id, shown: s.clone() }),
        // an integer is read as the number of the key (`UUID(int=n)`); a negative one, a float, a list and an object are no key at all
        Value::Number(n) => match n.as_u64() {
            Some(n) => PkRead::Key { id: Uuid::from_u128(u128::from(n)), shown: n.to_string() },
            None => not_a_uuid(&py_str(v)),
        },
        Value::Array(_) | Value::Object(_) => not_a_uuid(&py_str(v)),
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

/// A UUID as Python's `uuid.UUID(hex=...)` reads one, which is how Django reads a query parameter that names a row's key: `None` is
/// what makes Django raise its `ValidationError` (a bare 400, `Please provide valid detail`).
pub fn parse_uuid_text(s: &str) -> Option<Uuid> {
    uuid_hex(s)
}

/// The name Python gives the type of a JSON value, as DRF prints it in "got type".
pub fn py_type_name(v: &Value) -> &'static str {
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

const BAD_DATE: &str = "Date has wrong format. Use one of these formats instead: YYYY-MM-DD.";
const BAD_DATETIME: &str =
    "Datetime has wrong format. Use one of these formats instead: YYYY-MM-DDThh:mm[:ss[.uuuuuu]][+HH:MM|-HH:MM|Z].";

/// A date as Django's `parse_date` reads it (what DRF's `DateField` calls), in the three forms probed against the pin and recorded
/// (fixture S48): `YYYY-M-D` with one or two digits for the month and the day, `YYYYMMDD`, and the ISO week `YYYY-Www-D`. No space
/// around it, no time after it, a day that exists, a year from 1 to 9999. (Python 3.11's `date.fromisoformat` reads more spellings
/// of a week, and digits of other scripts; neither is read here.)
fn parse_date(s: &str) -> Option<NaiveDate> {
    let digits =
        |t: &str, min: usize, max: usize| (min..=max).contains(&t.len()) && t.bytes().all(|b| b.is_ascii_digit());
    let number = |t: &str| t.parse::<u32>().ok();
    let ymd = |y: u32, m: u32, d: u32| {
        NaiveDate::from_ymd_opt(i32::try_from(y).ok().filter(|y| (1..=9999).contains(y))?, m, d)
    };
    let parts: Vec<&str> = s.split('-').collect();
    match parts.as_slice() {
        [y, m, d] if digits(y, 4, 4) && digits(m, 1, 2) && digits(d, 1, 2) => ymd(number(y)?, number(m)?, number(d)?),
        [y, w, d] if digits(y, 4, 4) && digits(d, 1, 1) && w.strip_prefix('W').is_some_and(|n| digits(n, 2, 2)) => {
            let year = i32::try_from(number(y)?).ok().filter(|y| (1..=9999).contains(y))?;
            let weekday = chrono::Weekday::try_from(u8::try_from(number(d)?.checked_sub(1)?).ok()?).ok()?;
            NaiveDate::from_isoywd_opt(year, number(w.get(1..)?)?, weekday)
        }
        [whole] if digits(whole, 8, 8) => {
            ymd(number(whole.get(..4)?)?, number(whole.get(4..6)?)?, number(whole.get(6..)?)?)
        }
        _ => None,
    }
}

/// DRF `DateField` on a column that allows null: null, or a date as [`parse_date`] reads one (a time after it is refused).
pub fn date_field(v: &Value) -> Result<Option<NaiveDate>, String> {
    match v {
        Value::Null => Ok(None),
        Value::String(s) => parse_date(s).map(Some).ok_or_else(|| BAD_DATE.to_string()),
        _ => Err(BAD_DATE.into()),
    }
}

/// A date and time as Django's `parse_datetime` reads one: `YYYY-MM-DD`, `T` or a space, `HH:MM`, optionally `:SS` and a fraction of
/// up to six digits, then optionally `Z` or an offset of `+HH`, `+HHMM` or `+HH:MM`.
fn parse_datetime(s: &str) -> Option<()> {
    let (date, rest) = s.split_once(['T', ' '])?;
    parse_date(date).filter(|_| date.split('-').count() == 3)?;
    let (clock, zone) = match rest.find(['Z', '+', '-']) {
        Some(i) => (rest.get(..i)?, rest.get(i..)?),
        None => (rest, ""),
    };
    let zone_ok = match zone {
        "" | "Z" => true,
        z => {
            let digits = z.get(1..)?.replace(':', "");
            (digits.len() == 2 || digits.len() == 4) && digits.bytes().all(|b| b.is_ascii_digit())
        }
    };
    let (hm_s, fraction) = match clock.split_once(['.', ',']) {
        Some((c, f)) => (c, Some(f)),
        None => (clock, None),
    };
    let parts: Vec<&str> = hm_s.split(':').collect();
    let (h, m, sec) = match parts.as_slice() {
        [h, m] => (*h, *m, None),
        [h, m, sec] => (*h, *m, Some(*sec)),
        _ => return None,
    };
    let num = |t: &str, max: u32| {
        (1..=2).contains(&t.len()) && t.bytes().all(|b| b.is_ascii_digit()) && t.parse::<u32>().is_ok_and(|n| n <= max)
    };
    let fraction_ok = fraction.is_none_or(|f| sec.is_some() && !f.is_empty() && f.bytes().all(|b| b.is_ascii_digit()));
    (zone_ok && num(h, 23) && num(m, 59) && sec.is_none_or(|t| num(t, 59)) && fraction_ok).then_some(())
}

/// DRF `DateTimeField` on a column that allows null: null, or a date and time as [`parse_datetime`] reads one. Only whether it is
/// one is told: the work item's `deleted_at` and the like are read and checked, not stored.
pub fn datetime_field(v: &Value) -> Result<bool, String> {
    match v {
        Value::Null => Ok(false),
        Value::String(s) if parse_datetime(s).is_some() => Ok(true),
        _ => Err(BAD_DATETIME.into()),
    }
}

/// The whole numbers in a text, as Python's `re.findall(r"\b\d+\b", text)` finds them: a run of digits with no letter, digit or
/// underscore touching it on either side (a number too big to hold is not one).
pub fn whole_numbers(text: &str) -> Vec<i64> {
    let chars: Vec<char> = text.chars().collect();
    let word = |c: &char| c.is_alphanumeric() || *c == '_';
    // The text as runs of ASCII digits and runs of anything else: a run of digits is a number when what is on either side of it (the
    // end of a run of other characters, which is no digit) is no word character.
    let runs: Vec<&[char]> = chars.chunk_by(|a, b| a.is_ascii_digit() == b.is_ascii_digit()).collect();
    runs.iter()
        .enumerate()
        .filter(|(_, run)| run.first().is_some_and(char::is_ascii_digit))
        .filter(|(at, _)| {
            let before = at.checked_sub(1).and_then(|j| runs.get(j)).and_then(|r| r.last());
            let after = runs.get(at + 1).and_then(|r| r.first());
            !before.is_some_and(word) && !after.is_some_and(word)
        })
        .filter_map(|(_, run)| run.iter().collect::<String>().parse::<i64>().ok())
        .collect()
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
