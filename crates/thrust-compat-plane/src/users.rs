//! `/api/users/me/` and the rows beside it: the signed-in user, their profile, the onboarding flags (S02, S07, S10, S11,
//! S27 and the `onboard` route the stock onboarding ends with).
//!
//! The shapes are the pin's, field for field (the fixtures are recorded from it). The user comes in two shapes: the short
//! one `GET /api/users/me/` answers, and the long one `PATCH /api/users/me/` answers (every column of the pin's `User`
//! except the password), which the React store merges into its user, so both matter.

use axum::body::Bytes;
use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::{Extension, Json};
use serde_json::{json, Map, Value};
use thrust_contracts::chrono::SecondsFormat;
use thrust_contracts::*;
use thrust_core::{App, Me};
use thrust_ports::ProfilePatch;

use crate::fields::{django_bool, profile_patch, user_patch};
use crate::PlaneError;

/// The signed-in caller, or Plane's 401. (`thrust_api::Authed` answers with `thrust-api`'s error body; Plane's is
/// `{"detail": ...}`.)
pub(crate) struct Caller(pub Principal);

impl<S: Send + Sync> FromRequestParts<S> for Caller {
    type Rejection = PlaneError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let thrust_api::Authed(who) = thrust_api::Authed::from_request_parts(parts, state).await?;
        Ok(Caller(who))
    }
}

/// A DRF datetime: UTC, microseconds, `Z`. (The pin renders in the user's own time zone, with an offset; a client that
/// parses either gets the same instant, and the parity rules treat the value as opaque.)
pub(crate) fn ts(t: Timestamp) -> String {
    t.to_rfc3339_opts(SecondsFormat::Micros, true)
}

pub(crate) fn ts_or_null(t: Option<Timestamp>) -> Value {
    t.map_or(Value::Null, |t| json!(ts(t)))
}

pub(crate) fn non_empty(s: &str) -> Value {
    if s.is_empty() {
        Value::Null
    } else {
        json!(s)
    }
}

/// The body of a request as a JSON object: an empty body is an empty object, as DRF reads it. A body that is not an object is
/// refused with a message of Thrust's own (the pin's DRF wording is not recorded; G-17).
pub(crate) fn object_body(bytes: &Bytes) -> Result<Map<String, Value>, PlaneError> {
    if bytes.iter().all(u8::is_ascii_whitespace) {
        return Ok(Map::new());
    }
    match serde_json::from_slice::<Value>(bytes) {
        Ok(Value::Object(m)) => Ok(m),
        _ => Err(PlaneError::BadRequest(json!({"detail": "The request body must be a JSON object."}))),
    }
}

/// S02's body: the short user.
fn user_me(me: &Me) -> Value {
    let (u, p) = (&me.user, &me.profile);
    json!({
        "id": u.id.0,
        "avatar": p.avatar,
        "cover_image": p.cover_image,
        "avatar_url": non_empty(&p.avatar),
        "cover_image_url": p.cover_image,
        "date_joined": ts(p.date_joined),
        "display_name": p.display_name,
        "email": u.email.clone().unwrap_or_default(),
        "first_name": p.first_name,
        "last_name": p.last_name,
        "is_active": u.is_active,
        "is_bot": false,
        "is_email_verified": false,
        "is_password_autoset": false,
        "last_login_medium": "email",
        "last_login_time": ts_or_null(p.last_login_at),
        "user_timezone": p.timezone,
        "username": p.username,
    })
}

/// S10's body: the long user, every column of the pin's `User` but the password. `token` is a string of the pin's length
/// and shape and is NOT a credential here (the pin rotates a real one on every save); the columns Thrust has no use for
/// (`last_location`, the IPs, the user agent, the staff and superuser flags) are what a fresh pin account has.
fn user_full(me: &Me) -> Value {
    let (u, p) = (&me.user, &me.profile);
    let last_seen = p.last_login_at.unwrap_or(p.date_joined);
    let token = format!("{}{}", u.id.0.simple(), u.id.0.simple());
    json!({
        "id": u.id.0,
        "last_login": ts_or_null(p.last_login_at),
        "is_superuser": false,
        "username": p.username,
        "mobile_number": null,
        "email": u.email.clone().unwrap_or_default(),
        "display_name": p.display_name,
        "first_name": p.first_name,
        "last_name": p.last_name,
        "avatar": p.avatar,
        "avatar_asset": null,
        "cover_image": p.cover_image,
        "cover_image_asset": null,
        "date_joined": ts(p.date_joined),
        "created_at": ts(p.created_at),
        "updated_at": ts(p.updated_at),
        "last_location": "",
        "created_location": "",
        "is_managed": false,
        "is_password_expired": false,
        "is_active": u.is_active,
        "is_staff": false,
        "is_email_verified": false,
        "is_password_autoset": false,
        "is_password_reset_required": false,
        "token": token,
        "last_active": ts(last_seen),
        "last_login_time": ts_or_null(p.last_login_at),
        "last_logout_time": null,
        "last_login_ip": "",
        "last_logout_ip": "",
        "last_login_medium": "email",
        "last_login_uagent": "",
        "token_updated_at": ts(last_seen),
        "is_bot": false,
        "bot_type": null,
        "user_timezone": p.timezone,
        "is_email_valid": false,
        "masked_at": null,
    })
}

/// What a setting is before anyone sets it: the pin's column defaults.
fn profile_defaults() -> Map<String, Value> {
    let Value::Object(m) = json!({
        "theme": {},
        "is_app_rail_docked": true,
        "use_case": null,
        "role": null,
        "billing_address_country": "INDIA",
        "billing_address": null,
        "has_billing_address": false,
        "company_name": "",
        "notification_view_mode": "full",
        "is_smooth_cursor_enabled": false,
        "is_mobile_onboarded": false,
        "mobile_onboarding_step": {"profile_complete": false, "workspace_create": false, "workspace_join": false},
        "mobile_timezone_auto_set": false,
        "language": "en",
        "start_of_the_week": 0,
        "goals": {},
        "is_navigation_tour_completed": false,
        "has_marketing_email_consent": false,
        "is_subscribed_to_changelog": false,
        "product_tour": {"work_items": false, "cycles": false, "modules": false, "intake": false, "pages": false},
    }) else {
        unreachable!("a JSON object literal")
    };
    m
}

/// S07 and S11's body: the profile, 29 keys. The settings the user has set over the defaults; `background_color` is the
/// pin's random default, here derived from the profile's id so that it is stable.
fn profile_json(me: &Me) -> Value {
    let p = &me.profile;
    let mut m = profile_defaults();
    m.insert("background_color".into(), json!(format!("#{}", &p.id.simple().to_string()[..6])));
    if let Value::Object(set) = &p.preferences {
        for (k, v) in set {
            m.insert(k.clone(), v.clone());
        }
    }
    m.insert("id".into(), json!(p.id));
    m.insert("user".into(), json!(p.user_id.0));
    m.insert("created_at".into(), json!(ts(p.created_at)));
    m.insert("updated_at".into(), json!(ts(p.updated_at)));
    m.insert("is_onboarded".into(), json!(p.is_onboarded));
    m.insert("is_tour_completed".into(), json!(p.is_tour_completed));
    m.insert("onboarding_step".into(), p.onboarding_step.clone());
    m.insert("last_workspace_id".into(), json!(p.last_workspace_id.map(|w| w.0)));
    Value::Object(m)
}

/// S02, signed in. (Signed out is the extractor's 401, which is S02's other recorded answer.)
pub(crate) async fn get_me(Caller(who): Caller, Extension(app): Extension<App>) -> Result<Json<Value>, PlaneError> {
    Ok(Json(user_me(&thrust_core::me(&app, &who).await?)))
}

/// S10. The fields of `UserSerializer` that Thrust keeps are changed, the rest are ignored (including every read-only
/// field the pin names), and the answer is the long user.
pub(crate) async fn patch_me(Caller(who): Caller, Extension(app): Extension<App>, body: Bytes) -> Result<Json<Value>, PlaneError> {
    let patch = user_patch(&object_body(&body)?).map_err(PlaneError::Fields)?;
    Ok(Json(user_full(&thrust_core::update_me(&app, &who, patch).await?)))
}

/// S07.
pub(crate) async fn get_profile(Caller(who): Caller, Extension(app): Extension<App>) -> Result<Json<Value>, PlaneError> {
    Ok(Json(profile_json(&thrust_core::me(&app, &who).await?)))
}

/// S11. Every writable column of `ProfileSerializer`; the keys the body does not name keep their values.
pub(crate) async fn patch_profile(Caller(who): Caller, Extension(app): Extension<App>, body: Bytes) -> Result<Json<Value>, PlaneError> {
    let patch = profile_patch(&object_body(&body)?).map_err(PlaneError::Fields)?;
    Ok(Json(profile_json(&thrust_core::update_me(&app, &who, patch).await?)))
}

/// One flag of the profile set from a body key the way the pin does it: `request.data.get(key, False)` straight onto the
/// column, so a missing key is `false`, Django's booleans are booleans and anything else (`null` too) is a 400 with the
/// pin's one message.
async fn set_flag(app: &App, who: &Principal, body: &Bytes, key: &str, patch: impl FnOnce(bool) -> ProfilePatch) -> Result<Json<Value>, PlaneError> {
    let body = object_body(body)?;
    let value = django_bool(body.get(key)).map_err(|()| PlaneError::BadRequest(json!({"error": "Please provide valid detail"})))?;
    thrust_core::update_me(app, who, patch(value)).await?;
    Ok(Json(json!({"message": "Updated successfully"})))
}

/// S27: the tour on the home page is done.
pub(crate) async fn patch_tour_completed(Caller(who): Caller, Extension(app): Extension<App>, body: Bytes) -> Result<Json<Value>, PlaneError> {
    set_flag(&app, &who, &body, "is_tour_completed", |b| ProfilePatch { is_tour_completed: Some(b), ..ProfilePatch::default() }).await
}

/// X04: the last request of the stock onboarding. Not one of the inventory's 48 rows, but `finishUserOnboarding` awaits it,
/// and a 404 would stop the page from finishing.
pub(crate) async fn patch_onboard(Caller(who): Caller, Extension(app): Extension<App>, body: Bytes) -> Result<Json<Value>, PlaneError> {
    set_flag(&app, &who, &body, "is_onboarded", |b| ProfilePatch { is_onboarded: Some(b), ..ProfilePatch::default() }).await
}
