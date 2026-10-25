//! The caller's own profile (issue #44): who they say they are, and how they have set the app up.
//!
//! A profile is made when an account is: at sign-up, or (for an account older than profiles, or one a test seeded) the
//! first time it is read. Every change goes through [`update_me`], which applies a [`ProfilePatch`] in one statement, so
//! two requests that change different fields cannot overwrite each other.
//!
//! The rules here are the ones that must hold whoever asks: a name is not a URL, a time zone is a real one. The wording
//! of a refusal is the caller's: the compat layer words them as Plane does.

use thrust_contracts::*;
use thrust_ports::*;

use crate::timezones::COMMON_TIMEZONES;
use crate::{App, CoreError};

/// A user and the profile that goes with them: what `GET /api/users/me/` is made of.
#[derive(Debug, Clone, PartialEq)]
pub struct Me {
    pub user: UserRecord,
    pub profile: UserProfile,
}

/// Whether `tz` is one of the names Plane accepts for `user_timezone` (`pytz.common_timezones`, pytz 2024.1). Exact and
/// case-sensitive, as the pin is.
pub fn is_known_timezone(tz: &str) -> bool {
    COMMON_TIMEZONES.binary_search(&tz).is_ok()
}

/// Whether `value` contains something that reads as a URL: Plane's `URL_PATTERN` (`apps/api/plane/utils/url.py`), which it
/// refuses in names. Written out by hand because core carries no regex engine; the compat layer's tests compare it with
/// the pattern itself over generated ASCII text. What the pattern finds, anywhere in a line, case-insensitively:
///
/// * `http://` or `https://` followed by at least one non-space character;
/// * `www.` followed by a letter or digit;
/// * a dot with a letter or digit before it and at least two letters after it (`example.com`, but also `Jane.Doe`: the
///   pin refuses a name like that too);
/// * an IPv4 address (four dot-separated numbers up to 255).
///
/// As on the pin, a value of more than 1000 characters is never a URL, and only the first 500 characters of a line are
/// looked at. Non-ASCII letters never match, where Python's case-insensitive match also folds `K` and `ſ` onto ASCII.
pub fn contains_url(value: &str) -> bool {
    if value.chars().count() > 1000 {
        return false;
    }
    value.split('\n').any(|line| {
        let line: Vec<char> = line.chars().take(500).map(|c| c.to_ascii_lowercase()).collect();
        line_has_url(&line)
    })
}

fn line_has_url(s: &[char]) -> bool {
    let at = |i: usize| s.get(i).copied();
    let alnum = |c: Option<char>| c.is_some_and(|c| c.is_ascii_alphanumeric());
    let letter = |c: Option<char>| c.is_some_and(|c| c.is_ascii_lowercase());
    let starts_with = |i: usize, pat: &str| pat.chars().enumerate().all(|(k, p)| at(i + k) == Some(p));
    (0..s.len()).any(|i| {
        // http:// or https:// then one non-space
        let scheme = if starts_with(i, "https://") {
            Some(8)
        } else if starts_with(i, "http://") {
            Some(7)
        } else {
            None
        };
        let http = scheme.is_some_and(|n| at(i + n).is_some_and(|c| !c.is_whitespace()));
        // www. then a letter or digit
        let www = starts_with(i, "www.") && alnum(at(i + 4));
        // a dot after a letter or digit and before two letters
        let dotted = s[i] == '.' && i > 0 && alnum(at(i - 1)) && letter(at(i + 1)) && letter(at(i + 2));
        http || www || dotted
    }) || has_ipv4(s)
}

/// `d.N.N.d` where `d` is a digit and each `N` is a whole run of one to three digits with a value of at most 255: the first
/// and last octets only need their one digit next to a dot, because the pattern is not anchored.
fn has_ipv4(s: &[char]) -> bool {
    let text: String = s.iter().collect();
    let parts: Vec<&str> = text.split('.').collect();
    let octet = |p: &str| (1..=3).contains(&p.len()) && p.chars().all(|c| c.is_ascii_digit()) && p.parse::<u16>().is_ok_and(|n| n <= 255);
    parts.windows(4).any(|w| {
        w[0].chars().last().is_some_and(|c| c.is_ascii_digit())
            && octet(w[1])
            && octet(w[2])
            && w[3].chars().next().is_some_and(|c| c.is_ascii_digit())
    })
}

/// A profile for a new account, with the defaults every account starts from. The display name is the local part of the
/// email (what Plane's `User.save` fills in), the username 32 hex digits from the id source (never shown as a name).
pub(crate) fn new_profile(app: &App, user: UserId, email: Option<&str>) -> UserProfile {
    let now = app.ports.clock.now();
    UserProfile {
        id: app.ports.ids.new_uuid(),
        user_id: user,
        username: app.ports.ids.new_uuid().simple().to_string(),
        first_name: String::new(),
        last_name: String::new(),
        display_name: email.and_then(|e| e.split('@').next()).unwrap_or_default().to_string(),
        avatar: String::new(),
        cover_image: None,
        timezone: "UTC".to_string(),
        date_joined: now,
        last_login_at: None,
        is_onboarded: false,
        is_tour_completed: false,
        last_workspace_id: None,
        onboarding_step: serde_json::json!({
            "profile_complete": false,
            "workspace_create": false,
            "workspace_invite": false,
            "workspace_join": false,
        }),
        preferences: serde_json::json!({}),
        created_at: now,
        updated_at: now,
    }
}

/// The profile of `user`, made now if the account has none yet (an account that predates profiles, or one a test seeded).
async fn profile_of(app: &App, user: &UserRecord) -> Result<UserProfile, CoreError> {
    if let Some(p) = app.ports.db.profile(user.id).await? {
        return Ok(p);
    }
    app.ports.db.insert_profile(&new_profile(app, user.id, user.email.as_deref())).await?;
    app.ports.db.profile(user.id).await?.ok_or_else(|| CoreError::Internal("a profile that was just made is not there".into()))
}

async fn user_of(app: &App, who: &Principal) -> Result<UserRecord, CoreError> {
    // an authenticated principal whose user is gone (deleted since the request began) is simply not signed in
    app.ports.db.user_by_id(who.user_id).await?.ok_or(CoreError::Unauthenticated)
}

/// The caller and their profile.
pub async fn me(app: &App, who: &Principal) -> Result<Me, CoreError> {
    let user = user_of(app, who).await?;
    let profile = profile_of(app, &user).await?;
    Ok(Me { user, profile })
}

/// Change the caller's profile and return it as it is afterwards. `Invalid` for a name that is a URL or a time zone that
/// is not one; nothing is written then.
pub async fn update_me(app: &App, who: &Principal, patch: ProfilePatch) -> Result<Me, CoreError> {
    for (what, name) in [("first name", &patch.first_name), ("last name", &patch.last_name)] {
        if name.as_deref().is_some_and(contains_url) {
            return Err(CoreError::Invalid(format!("{what} cannot contain a URL")));
        }
    }
    if patch.timezone.as_deref().is_some_and(|tz| !is_known_timezone(tz)) {
        return Err(CoreError::Invalid("not a known time zone".into()));
    }
    let user = user_of(app, who).await?;
    profile_of(app, &user).await?; // makes it if there is none, so the update has a row to change
    let profile = app
        .ports
        .db
        .update_profile(who.user_id, &patch, app.ports.clock.now())
        .await?
        .ok_or_else(|| CoreError::Internal("a profile that was just made is not there".into()))?;
    Ok(Me { user, profile })
}
