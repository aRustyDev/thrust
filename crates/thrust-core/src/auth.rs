//! Identity use-cases (issue #38): argon2id sign-in, the session lifecycle, CSRF, API keys and the OIDC login.
//!
//! Nothing here knows how a request arrived or how a cookie is spelled: `thrust-api` parses headers into
//! [`Credentials`] and formats the answer. Everything here takes its randomness, its clock and its hashing from the
//! ports, so a test can repeat a run and count hash work.

use argon2::{Algorithm, Argon2, Params, PasswordHasher as _, PasswordVerifier as _, Version};
use thrust_contracts::*;
use thrust_ports::*;

use crate::{App, CoreConfig, CoreError};

/// Argon2id cost parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Argon2Params {
    /// Memory in KiB.
    pub m_cost_kib: u32,
    /// Passes.
    pub t_cost: u32,
    /// Lanes.
    pub p_cost: u32,
}

impl Argon2Params {
    /// OWASP's minimum (Password Storage Cheat Sheet): 19 MiB, two passes, one lane.
    pub const OWASP_MINIMUM: Argon2Params = Argon2Params { m_cost_kib: 19 * 1024, t_cost: 2, p_cost: 1 };
}

/// argon2id through the RustCrypto `argon2` crate: pure Rust, so one implementation serves the native server and the
/// Worker.
pub struct Argon2idHasher {
    params: Argon2Params,
}

impl Argon2idHasher {
    pub fn new(params: Argon2Params) -> Self {
        Self { params }
    }

    fn argon2(&self) -> Result<Argon2<'static>, PasswordError> {
        let p = &self.params;
        let params = Params::new(p.m_cost_kib, p.t_cost, p.p_cost, None).map_err(|e| PasswordError(e.to_string()))?;
        Ok(Argon2::new(Algorithm::Argon2id, Version::V0x13, params))
    }
}

impl PasswordHasher for Argon2idHasher {
    fn hash(&self, password: &str, salt: &[u8]) -> Result<String, PasswordError> {
        let phc = self
            .argon2()?
            .hash_password_with_salt(password.as_bytes(), salt)
            .map_err(|e| PasswordError(e.to_string()))?;
        Ok(phc.to_string())
    }

    fn verify(&self, password: &str, phc: &str) -> bool {
        // The cost comes from the string; `self.params` is only for new hashes.
        self.argon2().is_ok_and(|a| a.verify_password(password.as_bytes(), phc).is_ok())
    }
}

/// What a successful sign-in hands back. `token` and `csrf_token` are secrets: the caller puts them in cookies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignedIn {
    pub user_id: UserId,
    pub token: String,
    pub csrf_token: String,
    pub expires_at: Timestamp,
}

/// Longest accepted email (RFC 5321 gives a path 256 octets; 320 is the usual generous bound) and password. Both
/// bounds apply to every attempt, known email or not, so they tell a caller nothing about an account.
const MAX_EMAIL_LEN: usize = 320;
const MAX_PASSWORD_LEN: usize = 1024;
const SALT_LEN: usize = 16;

/// The one spelling an email is stored and looked up under: trimmed and lower case. (Full Unicode case folding and
/// IDNA are not P0's business; Plane lowercases too.)
pub fn normalize_email(raw: &str) -> String {
    raw.trim().to_lowercase()
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        // lower case, two digits a byte; writing to a `String` cannot fail
        let _ = write!(out, "{b:02x}");
    }
    out
}

/// `bytes` of CSPRNG output, hex-encoded. A source that fails is an internal error: no request ever proceeds on a
/// short or predictable token.
pub(crate) fn random_hex(app: &App, bytes: usize) -> Result<String, CoreError> {
    let mut buf = vec![0u8; bytes];
    app.ports.rng.fill(&mut buf).map_err(|e| CoreError::Internal(e.to_string()))?;
    Ok(hex(&buf))
}

pub(crate) fn random_salt(app: &App) -> Result<[u8; SALT_LEN], CoreError> {
    let mut salt = [0u8; SALT_LEN];
    app.ports.rng.fill(&mut salt).map_err(|e| CoreError::Internal(e.to_string()))?;
    Ok(salt)
}

/// Make a session for `user`: 256 bits of token, 128 of CSRF token, one row holding only the token's digest, expiring
/// after `session_ttl_secs`. Always a fresh token, so signing in never reuses a session id (no fixation).
pub(crate) async fn mint_session(app: &App, user: UserId) -> Result<SignedIn, CoreError> {
    let token = random_hex(app, 32)?;
    let csrf_token = random_hex(app, 16)?;
    let now = app.ports.clock.now();
    let expires_at = now + thrust_contracts::chrono::TimeDelta::seconds(app.cfg.session_ttl_secs);
    app.ports
        .db
        .insert_session(&NewSession {
            token_hash: crate::hash_token(&token),
            user_id: user,
            created_at: now,
            expires_at,
        })
        .await?;
    // When they last signed in is bookkeeping, like a session's `last_seen_at`: failing to write it, or an account with no
    // profile yet, must not fail a sign-in that is authentic.
    let seen = ProfilePatch { last_login_at: Some(now), ..ProfilePatch::default() };
    let _ = app.ports.db.update_profile(user, &seen, now).await;
    Ok(SignedIn { user_id: user, token, csrf_token, expires_at })
}

/// Email and password to a session. Order matters and each step is the same for every caller: input bounds, the
/// per-email limiter, then the lookup, then exactly one unit of hash work whether or not the email exists (a wrong
/// password verifies against the stored hash; an unknown email, or a user with no password, hashes the attempt and
/// throws the result away), then one answer for every refusal.
pub async fn sign_in(app: &App, email: &str, password: &str) -> Result<SignedIn, CoreError> {
    let email = normalize_email(email);
    if email.is_empty() || email.len() > MAX_EMAIL_LEN || password.is_empty() || password.len() > MAX_PASSWORD_LEN {
        return Err(CoreError::Invalid("an email and a password are required".into()));
    }
    let decision = app
        .ports
        .limiter
        .hit(&format!("signin:{email}"), app.cfg.sign_in_rate_limit, app.cfg.sign_in_rate_window_secs)
        .await?;
    if let RateDecision::Limited { retry_after_secs } = decision {
        return Err(CoreError::RateLimited { retry_after_secs });
    }
    let found = app.ports.db.user_by_email(&email).await?;
    let user = match found {
        Some(u) if u.password_hash.is_some() => {
            let phc = u.password_hash.as_deref().unwrap_or_default();
            app.ports.hasher.verify(password, phc).then_some(u)
        }
        _ => {
            // Nobody to verify against: do the work anyway, so the time and cost of a refusal do not depend on it.
            app.ports.hasher.hash(password, &random_salt(app)?).map_err(|e| CoreError::Internal(e.to_string()))?;
            None
        }
    };
    match user {
        Some(u) if u.is_active => mint_session(app, u.id).await,
        _ => Err(CoreError::Unauthenticated),
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// Sign-up (issue #44). The stock Plane app signs a person up with an email and a password: one new user, signed in.
// ---------------------------------------------------------------------------------------------------------------------

/// Whether `email` (already [`normalize_email`]d) is something a mailbox could be: an ASCII address of the usual shape.
/// This is Django's `validate_email` in outline, not in full: one `@`, a local part of at most 64 characters made of
/// the usual atom characters and single dots, and a domain of at least two dot-separated labels (letters, digits and
/// inner hyphens) ending in an alphabetic top-level domain or an `xn--` one. It refuses what Django accepts but no one
/// signs up with (quoted local parts, domain literals, `user@localhost`, domains in Unicode rather than punycode).
pub fn is_valid_email(email: &str) -> bool {
    if email.is_empty() || email.len() > MAX_EMAIL_LEN {
        return false;
    }
    let Some((local, domain)) = email.rsplit_once('@') else { return false };
    let atom = |c: char| c.is_ascii_alphanumeric() || "!#$%&'*+/=?^_`{|}~-".contains(c);
    let local_ok = !local.is_empty()
        && local.len() <= 64
        && local.split('.').all(|part| !part.is_empty() && part.chars().all(atom));
    let labels: Vec<&str> = domain.split('.').collect();
    let label_ok = |l: &str| {
        !l.is_empty()
            && l.len() <= 63
            && !l.starts_with('-')
            && !l.ends_with('-')
            && l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
    };
    // an `xn--` label that is only `xn--` ends in a hyphen, which `label_ok` already refuses, so nothing more is needed
    let tld_ok = labels
        .last()
        .is_some_and(|t| (t.len() >= 2 && t.chars().all(|c| c.is_ascii_alphabetic())) || t.starts_with("xn--"));
    local_ok && labels.len() >= 2 && labels.iter().all(|l| label_ok(l)) && tld_ok
}

/// The shortest password sign-up accepts, in characters.
pub const MIN_PASSWORD_LEN: usize = 10;
/// Fewest distinct characters a password may have: `aaaaaaaaaa` and `abababababab` are not passwords.
const MIN_DISTINCT_CHARS: usize = 5;

/// The passwords people pick most, compared lower case with the digits and symbols around the letters stripped (so
/// `Password123!` is `password`).
const COMMON_PASSWORDS: &[&str] = &[
    "password",
    "passw0rd",
    "qwerty",
    "qwertyuiop",
    "letmein",
    "welcome",
    "admin",
    "administrator",
    "iloveyou",
    "monkey",
    "dragon",
    "football",
    "baseball",
    "master",
    "login",
    "trustno",
    "sunshine",
    "princess",
    "starwars",
    "whatever",
    "freedom",
    "shadow",
    "superman",
    "michael",
    "jordan",
    "hello",
    "charlie",
    "donald",
    "hunter",
    "abc",
    "abcdef",
    "asdfgh",
    "asdfghjkl",
    "zxcvbn",
    "zxcvbnm",
    "plane",
    "thrust",
    "changeme",
    "secret",
    "default",
    "test",
    "guest",
];

/// Whether `password` is acceptable for a new account. A deliberately modest rule in place of Plane's zxcvbn score of
/// at least 3, which would put a large frequency dictionary into the Worker bundle (its size was not measured): at
/// least [`MIN_PASSWORD_LEN`] characters, at least [`MIN_DISTINCT_CHARS`] different ones, and not one of the most
/// common passwords once its decoration is stripped. It is stricter than zxcvbn about length and weaker about
/// everything else, and the difference is deviation D-023.
pub fn password_is_strong(password: &str) -> bool {
    let chars = password.chars().count();
    if chars < MIN_PASSWORD_LEN || password.len() > MAX_PASSWORD_LEN {
        return false;
    }
    let distinct: std::collections::BTreeSet<char> = password.chars().collect();
    if distinct.len() < MIN_DISTINCT_CHARS {
        return false;
    }
    let letters = password.trim_matches(|c: char| !c.is_alphabetic()).to_lowercase();
    !COMMON_PASSWORDS.contains(&letters.as_str())
}

/// Whether an account has this email: what Plane's `email-check` answers, and what its sign-in asks before it checks the
/// password (so it can say "no such user" apart from "wrong password"). Plane answers it to anyone, so this does not
/// widen what a caller can learn there; the answer is `false` for an email that is not valid, without a lookup.
pub async fn email_exists(app: &App, email: &str) -> Result<bool, CoreError> {
    let email = normalize_email(email);
    if !is_valid_email(&email) {
        return Ok(false);
    }
    Ok(app.ports.db.user_by_email(&email).await?.is_some())
}

/// A new account with an email and a password, signed in. In this order, with no hash work until the last decision:
/// the bounds and the email's shape and the password's strength (`Invalid`, nothing stored), then whether the email is
/// taken (`Conflict`), then one hash, one insert (a race for the same email is the unique index's `Conflict`), one
/// session. The user is active and has no name yet: the profile is the onboarding step's.
///
/// There is no limiter here, unlike [`sign_in`]'s per-email one: a per-address limit needs the caller's address, which
/// core never sees, and a global one would let anyone lock everyone else out. Plane limits sign-up per IP; see gap G-16.
pub async fn sign_up(app: &App, email: &str, password: &str) -> Result<SignedIn, CoreError> {
    let id = create_account(app, email, password).await?;
    mint_session(app, id).await
}

/// The part of making an account that sign-up and the operator's create-user share, so that the two cannot drift: the
/// refusals, in order and with no hash work before the last one (a malformed email, a weak password: `Invalid`; an email
/// that is taken: `Conflict`), then one hash, one user row (a race for the same email is the unique index's `Conflict`) and
/// its profile. Returns the new user. No session: that is sign-up's step.
async fn create_account(app: &App, email: &str, password: &str) -> Result<UserId, CoreError> {
    let email = normalize_email(email);
    if !is_valid_email(&email) {
        return Err(CoreError::Invalid("not a valid email".into()));
    }
    if !password_is_strong(password) {
        return Err(CoreError::Invalid("password too weak".into()));
    }
    if app.ports.db.user_by_email(&email).await?.is_some() {
        return Err(CoreError::Conflict("a user with this email exists".into()));
    }
    let password_hash =
        app.ports.hasher.hash(password, &random_salt(app)?).map_err(|e| CoreError::Internal(e.to_string()))?;
    let id = UserId(app.ports.ids.new_uuid());
    app.ports
        .db
        .insert_user(&UserRecord {
            id,
            email: Some(email.clone()),
            password_hash: Some(password_hash),
            is_active: true,
        })
        .await?;
    app.ports.db.insert_profile(&crate::profile::new_profile(app, id, Some(&email))).await?;
    Ok(id)
}

// ---------------------------------------------------------------------------------------------------------------------
// The operator's way in (Q-F7, issue #46). There is no email to send a reset link with (ADR-0073's pipeline is after M2), so
// the person who runs the deployment makes the account, or sets a new password for it, and tells the owner of the account
// out of band. Both are behind `OperatorAdmin` in `thrust-api`, which is a door of its own and not the test routes.
// ---------------------------------------------------------------------------------------------------------------------

/// The operator makes an account with a temporary password: the same rules as [`sign_up`] (same checks, same refusals, the
/// same single hash), but nobody is signed in, and the user is returned so the caller can say which account it made.
pub async fn admin_create_user(app: &App, email: &str, password: &str) -> Result<UserId, CoreError> {
    create_account(app, email, password).await
}

/// The operator sets a new password for the account with this email. In this order, with no hash work until the last
/// decision: the password's strength (`Invalid`, nothing changed), whether the account exists (`NotFound`), then one hash,
/// one write, and every session of that user ended (so a session that someone else holds stops working with the old
/// password). The account's state is not touched: a deactivated user stays deactivated, and a password makes nobody active.
/// Returns the user.
pub async fn admin_set_password(app: &App, email: &str, password: &str) -> Result<UserId, CoreError> {
    let email = normalize_email(email);
    // MUTANT (control run only): no strength check on a reset. The contract suite's operator test must catch it.
    #[cfg(not(feature = "mutant_operator_skips_strength"))]
    if !password_is_strong(password) {
        return Err(CoreError::Invalid("password too weak".into()));
    }
    let user = app.ports.db.user_by_email(&email).await?.ok_or(CoreError::NotFound)?;
    let password_hash =
        app.ports.hasher.hash(password, &random_salt(app)?).map_err(|e| CoreError::Internal(e.to_string()))?;
    if !app.ports.db.set_password_hash(&app.system, user.id, &password_hash).await? {
        // the account was removed between the lookup and the write
        return Err(CoreError::NotFound);
    }
    // MUTANT (control run only): leave the user's sessions alive. The contract suite's operator test must catch it.
    #[cfg(not(feature = "mutant_operator_keeps_sessions"))]
    app.ports.db.revoke_user_sessions(user.id, app.ports.clock.now()).await?;
    Ok(user.id)
}

// ---------------------------------------------------------------------------------------------------------------------
// A person changes their own password (Q-F7, issue #46): what Profile -> Security does, and what a person given a temporary
// password by the operator does first.
// ---------------------------------------------------------------------------------------------------------------------

/// What a change of password came to. A refusal is an answer, not a failure: the caller says which one, and nothing was changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PasswordChange {
    /// The account has the new password, every session it had is over, and this is the one made in their place.
    Changed(SignedIn),
    /// The old password is not the account's (or the account has none to match, or the guess is longer than a password may be).
    WrongOldPassword,
    /// The new password is refused by [`password_is_strong`] (D-023).
    TooWeak,
}

/// The caller changes their own password. In this order, the order of Plane's `ChangePasswordEndpoint` (a wrong old password is
/// answered before a weak new one, so the one with two faults gets the first code):
///
///  1. a password longer than a password may be is a wrong one, with no work and no limiter (it costs nothing and could be nothing);
///  2. the per-email limiter, the same bucket [`sign_in`] counts in, because a guess at the old password is a guess at the password
///     and a person should have one allowance for the two, counted before any hashing (a limited caller gets
///     `RateLimited`); Plane does not limit a signed-in caller at all (D-039);
///  3. the old password, with exactly one unit of hash work whether the account has a password or not (an account that single
///     sign-on made has none, and its attempt is hashed and thrown away, as in [`sign_in`]);
///  4. the new password's strength, which needs no work;
///  5. one hash, the write, every session of the account ended (the one that asked included: a session held by someone who knew
///     the old password must not outlive it, and Plane ends them all too), and one new session for the caller, so they stay in.
///
/// The old password is asked for whatever the account is: Thrust reports `is_password_autoset` as false for everyone, so the page
/// always asks, and an account with no password cannot answer (the operator's reset gives it one, D-038).
pub async fn change_password(app: &App, who: &Principal, old: &str, new: &str) -> Result<PasswordChange, CoreError> {
    if old.len() > MAX_PASSWORD_LEN {
        return Ok(PasswordChange::WrongOldPassword);
    }
    let user = app.ports.db.user_by_id(who.user_id).await?.ok_or(CoreError::Unauthenticated)?;
    // a session of a deactivated user does not authenticate, so this is the window between that check and this read
    if !user.is_active {
        return Err(CoreError::Unauthenticated);
    }
    let bucket = user.email.clone().unwrap_or_else(|| user.id.0.to_string());
    let decision = app
        .ports
        .limiter
        .hit(&format!("signin:{bucket}"), app.cfg.sign_in_rate_limit, app.cfg.sign_in_rate_window_secs)
        .await?;
    if let RateDecision::Limited { retry_after_secs } = decision {
        return Err(CoreError::RateLimited { retry_after_secs });
    }
    let old_is_right = match user.password_hash.as_deref() {
        Some(phc) => app.ports.hasher.verify(old, phc),
        None => {
            app.ports.hasher.hash(old, &random_salt(app)?).map_err(|e| CoreError::Internal(e.to_string()))?;
            false
        }
    };
    // MUTANT (control run only): the old password is not checked. The contract suite's change-password test must catch it.
    #[cfg(feature = "mutant_change_password_skips_old")]
    let old_is_right = {
        let _ = old_is_right;
        true
    };
    if !old_is_right {
        return Ok(PasswordChange::WrongOldPassword);
    }
    // MUTANT (control run only): no strength check on a change. The contract suite's change-password test must catch it.
    #[cfg(not(feature = "mutant_change_password_skips_strength"))]
    if !password_is_strong(new) {
        return Ok(PasswordChange::TooWeak);
    }
    let password_hash =
        app.ports.hasher.hash(new, &random_salt(app)?).map_err(|e| CoreError::Internal(e.to_string()))?;
    if !app.ports.db.set_password_hash(&app.system, user.id, &password_hash).await? {
        // the account was removed between the lookup and the write
        return Err(CoreError::Unauthenticated);
    }
    // MUTANT (control run only): the account's other sessions are left alive. The contract suite's change-password test must catch it.
    #[cfg(not(feature = "mutant_change_password_keeps_sessions"))]
    app.ports.db.revoke_user_sessions(user.id, app.ports.clock.now()).await?;
    Ok(PasswordChange::Changed(mint_session(app, user.id).await?))
}

/// Sign-out of one session, by the digest of its token. Idempotent.
pub async fn sign_out(app: &App, session: &TokenHash) -> Result<(), CoreError> {
    Ok(app.ports.db.revoke_session(session, app.ports.clock.now()).await?)
}

/// Sign-out everywhere: every session of the caller's user, this one included. Returns how many it ended.
pub async fn sign_out_everywhere(app: &App, who: &Principal) -> Result<u64, CoreError> {
    Ok(app.ports.db.revoke_user_sessions(who.user_id, app.ports.clock.now()).await?)
}

/// Deactivate or reactivate a user. Test support behind `/internal/*` until ADR-0053 provides the real thing; a
/// deactivated user's sessions and keys stop working on the next request, because the lookups join `users`.
/// `true` if the user exists.
pub async fn set_user_active(app: &App, user: UserId, active: bool) -> Result<bool, CoreError> {
    Ok(app.ports.db.set_user_active(&app.system, user, active).await?)
}

/// Test support behind `/internal/seed`: the fixture's logins (passwords hashed here, so the database only ever sees a
/// PHC string) and provider links. Called by [`crate::seed`] after the rows it depends on exist.
pub(crate) async fn seed_identities(app: &App, f: &SeedFixture) -> Result<(), CoreError> {
    let mut users = Vec::with_capacity(f.logins.len());
    for l in &f.logins {
        let password_hash = match &l.password {
            Some(pw) => {
                Some(app.ports.hasher.hash(pw, &random_salt(app)?).map_err(|e| CoreError::Internal(e.to_string()))?)
            }
            None => None,
        };
        users.push(UserRecord {
            id: l.user_id,
            email: Some(normalize_email(&l.email)),
            password_hash,
            is_active: l.is_active,
        });
    }
    app.ports.db.seed_users(&app.system, &users).await?;
    let now = app.ports.clock.now();
    for a in &f.accounts {
        app.ports
            .db
            .link_account(&NewAccount {
                provider: a.provider.clone(),
                provider_account_id: a.provider_account_id.clone(),
                user_id: a.user_id,
                email: None,
                created_at: now,
            })
            .await?;
    }
    for k in &f.api_keys {
        let id = ApiKeyId(app.ports.ids.new_uuid());
        app.ports
            .db
            .insert_api_key(&NewApiKey {
                id,
                user_id: k.user_id,
                token_hash: api_key_digest(app, &k.token)?,
                hash_version: app.cfg.api_key_hash_current,
                prefix: k.token.chars().take(API_KEY_PREFIX_LEN).collect(),
                label: k.label.clone(),
                created_at: now,
                expires_at: k.expires_at,
            })
            .await?;
        if k.revoked {
            app.ports.db.revoke_api_key(k.user_id, id, now).await?;
        }
    }
    Ok(())
}

/// What a request carries, parsed by the API layer. Core decides which one counts and whether it needs a CSRF token.
#[derive(Debug, Default, Clone, Copy)]
pub struct Credentials<'a> {
    /// `Authorization: Bearer <session token>`.
    pub bearer: Option<&'a str>,
    /// `X-Api-Key`.
    pub api_key: Option<&'a str>,
    /// The session cookie's value.
    pub session_cookie: Option<&'a str>,
    /// The CSRF cookie's value, and the token the request sent in its header.
    pub csrf_cookie: Option<&'a str>,
    pub csrf_header: Option<&'a str>,
    /// POST, PUT, PATCH or DELETE: anything that is not a safe method.
    pub mutating: bool,
}

/// How the caller proved who they are.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Via {
    /// A session, by its digest (what sign-out revokes).
    Session(TokenHash),
    /// An API key, by its id.
    ApiKey(ApiKeyId),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Authenticated {
    pub principal: Principal,
    pub via: Via,
}

/// Constant-time equality of two byte strings (the comparison time does not depend on where they first differ; the
/// length is not secret). For CSRF tokens and the admin token.
pub fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    use subtle::ConstantTimeEq;
    a.len() == b.len() && bool::from(a.ct_eq(b))
}

/// The double-submit check: the token the request sent in its header must equal the one in its cookie. Both must be
/// there and not empty, so a request with neither (or both blank) is not "equal".
pub fn csrf_ok(cookie: Option<&str>, header: Option<&str>) -> bool {
    // MUTANT (control run only): accept any request. The contract suite's CSRF tests must catch it.
    #[cfg(feature = "mutant_skip_csrf")]
    {
        let _ = (cookie, header);
        return true;
    }
    #[cfg(not(feature = "mutant_skip_csrf"))]
    match (cookie, header) {
        (Some(c), Some(h)) => !c.is_empty() && ct_eq(c.as_bytes(), h.as_bytes()),
        _ => false,
    }
}

/// Whether this process asks a cookie-authenticated mutation for the CSRF token: `CoreConfig::csrf_enforce`.
fn csrf_enforced(app: &App) -> bool {
    // MUTANT (control run only): the switch is ignored when it is on, so no cookie mutation is ever asked for the token.
    // The contract suite's main pass (enforcement on) must catch it.
    #[cfg(feature = "mutant_csrf_never")]
    {
        let _ = app;
        return false;
    }
    // MUTANT (control run only): the switch is ignored when it is off, so every cookie mutation is asked for the token.
    // The contract suite's `csrfoff` pass (enforcement off) must catch it.
    #[cfg(all(feature = "mutant_csrf_always", not(feature = "mutant_csrf_never")))]
    {
        let _ = app;
        return true;
    }
    #[cfg(not(any(feature = "mutant_csrf_never", feature = "mutant_csrf_always")))]
    app.cfg.csrf_enforce
}

/// A fresh double-submit token: 128 random bits, hex.
pub fn new_csrf_token(app: &App) -> Result<String, CoreError> {
    random_hex(app, 16)
}

/// The credential of a request, and who it belongs to. A bearer token wins when there is one, then an API key (neither
/// can be an ambient credential, so neither needs a CSRF token); otherwise the session cookie counts, and a mutation
/// made with it must carry the CSRF token when `CoreConfig::csrf_enforce` is on (it is off in the composition roots'
/// defaults while the stock frontend is in use: then the token is not consulted at all). See `tests/credentials.rs` and
/// `tests/api_keys.rs` for the tables.
pub async fn authenticate_request(app: &App, c: &Credentials<'_>) -> Result<Authenticated, CoreError> {
    if let Some(token) = c.bearer {
        let principal = crate::authenticate(app, token).await?;
        return Ok(Authenticated { principal, via: Via::Session(crate::hash_token(token)) });
    }
    // an empty header is no key (Plane: `if not token`), and only a missing key is a 401
    if let Some(key) = c.api_key.filter(|k| !k.is_empty()) {
        let (principal, id) = authenticate_api_key(app, key).await?;
        return Ok(Authenticated { principal, via: Via::ApiKey(id) });
    }
    if let Some(token) = c.session_cookie {
        let principal = crate::authenticate(app, token).await?;
        if c.mutating && csrf_enforced(app) && !csrf_ok(c.csrf_cookie, c.csrf_header) {
            return Err(CoreError::Csrf);
        }
        return Ok(Authenticated { principal, via: Via::Session(crate::hash_token(token)) });
    }
    Err(CoreError::Unauthenticated)
}

// --- API keys (ADR-0052 K1) ---------------------------------------------------------------------------------------

/// The digest scheme that ships: plain SHA-256 of the token (`hash_token`), no pepper. An API key is at least 256 random
/// bits, so a leaked digest cannot be reversed with or without a pepper, and a pepper would add a secret and a rotation
/// story to every target for no gain; sessions took the same decision on 2026-10-22 (Q-0052.3, owner decision
/// 2026-10-24).
pub const API_KEY_HASH_V1: i16 = 1;
/// The scheme new keys are stored under, and the one a key is moved to when it is next used.
pub const API_KEY_HASH_CURRENT: i16 = API_KEY_HASH_V1;
/// The schemes a key is looked up under. Version 1 only: the hatch below is documented, not used.
pub const API_KEY_HASH_SUPPORTED: &[i16] = &[API_KEY_HASH_V1];

/// The digest an API key `token` is stored and looked up under in scheme `version`, or `None` for a version this
/// build has no scheme for (so a row of that version can never match).
///
/// This is the escape hatch of Q-0052.3: to introduce a pepper (or any other scheme) later WITHOUT invalidating a
/// key, add an arm here, keep version 1, and move `API_KEY_HASH_CURRENT` and `API_KEY_HASH_SUPPORTED`; keys then move
/// to the new scheme as they are used. The steps, the query that shows progress and the caveats are in ADR-0052's
/// 2026-10-24 note. Tests prove the mechanism with a stand-in scheme that exists only under `cfg(test)`.
pub fn api_key_hash(version: i16, token: &str) -> Option<TokenHash> {
    match version {
        API_KEY_HASH_V1 => Some(crate::hash_token(token)),
        #[cfg(test)]
        key_hash_tests::STAND_IN_VERSION => Some(key_hash_tests::stand_in_digest(token)),
        _ => None,
    }
}

/// The versions an `X-Api-Key` is looked up under, in the order they are tried: the current one first (a key that is at
/// the current version, which is nearly every key, costs one lookup), then the other supported ones in ascending order,
/// each once. The current one is always included, listed or not: a key this build writes must be one it can read.
fn api_key_lookup_versions(cfg: &CoreConfig) -> Vec<i16> {
    let mut rest: Vec<i16> =
        cfg.api_key_hash_supported.iter().copied().filter(|v| *v != cfg.api_key_hash_current).collect();
    rest.sort_unstable();
    rest.dedup();
    let mut all = vec![cfg.api_key_hash_current];
    all.extend(rest);
    all
}

/// The digest of `token` in the scheme new keys are stored under.
fn api_key_digest(app: &App, token: &str) -> Result<TokenHash, CoreError> {
    let v = app.cfg.api_key_hash_current;
    api_key_hash(v, token).ok_or_else(|| CoreError::Internal(format!("no API key hash scheme for version {v}")))
}

/// What every key starts with, so one can be recognised in a log or a leaked file (and found by a secret scanner).
const API_KEY_MARKER: &str = "thrk_";
/// The marker and the first four characters of the secret: all that is ever shown again.
const API_KEY_PREFIX_LEN: usize = 9;
const MAX_LABEL_LEN: usize = 100;

/// The key behind `key`, looked up under every version this process reads, with the version it was found under. A row is
/// a match only when it is stored under the version whose digest found it: a digest belongs to the scheme that made it.
async fn find_api_key(app: &App, key: &str, now: Timestamp) -> Result<Option<(LiveApiKey, i16)>, CoreError> {
    for version in api_key_lookup_versions(&app.cfg) {
        let Some(digest) = api_key_hash(version, key) else { continue };
        if let Some(live) = app.ports.db.api_key(&digest, now).await? {
            if live.hash_version == version {
                return Ok(Some((live, version)));
            }
        }
    }
    Ok(None)
}

/// An `X-Api-Key` to its user, or [`CoreError::InvalidApiKey`]: the key must exist, not be revoked, not be expired, and
/// belong to an active user, and the lookup does not say which of those failed. A successful use stamps `last_used_at`
/// through [`Background`], at most once per `session_touch_after_secs`, so a busy key costs one write an hour and a
/// request never waits for it.
///
/// The hash hatch (Q-0052.3): the key is looked up under every version the process reads, and one that is stored under
/// another version than the current one is moved to the current one by the same background task, as a compare-and-set
/// on the version it was found at. A failed move changes nothing the caller sees and is retried by the next use; a key
/// that is not used is never moved, and keeps working while its version is read.
async fn authenticate_api_key(app: &App, key: &str) -> Result<(Principal, ApiKeyId), CoreError> {
    let now = app.ports.clock.now();
    let (live, version) = find_api_key(app, key, now).await?.ok_or(CoreError::InvalidApiKey)?;
    let stale = live
        .last_used_at
        .is_none_or(|t| now - t >= thrust_contracts::chrono::TimeDelta::seconds(app.cfg.session_touch_after_secs));
    let current = app.cfg.api_key_hash_current;
    // the digest is made here, in the request, so the background task holds a digest and not the key
    let move_to = if version == current { None } else { api_key_hash(current, key).map(|digest| (digest, current)) };
    if stale || move_to.is_some() {
        let db = app.ports.db.clone();
        app.ports.background.spawn(Box::pin(async move {
            // bookkeeping: a failed write changes nothing the caller sees
            if stale {
                let _ = db.touch_api_key(live.id, now).await;
            }
            if let Some((digest, to)) = move_to {
                let _ = db.rehash_api_key(live.id, version, &digest, to).await;
            }
        }));
    }
    Ok((Principal { user_id: live.user_id }, live.id))
}

/// Make a key for `who`: `thrk_` and 256 random bits (hex), stored as its digest only (under the current version, 1 =
/// plain SHA-256 unless the hatch is in use), so the token is in the answer to this call and nowhere else. The expiry, when there is one, must be in the future.
pub async fn create_api_key(app: &App, who: &Principal, req: CreateApiKeyRequest) -> Result<CreatedApiKey, CoreError> {
    let label = req.label.unwrap_or_default();
    if label.chars().count() > MAX_LABEL_LEN {
        return Err(CoreError::Invalid(format!("a label is at most {MAX_LABEL_LEN} characters")));
    }
    let now = app.ports.clock.now();
    if req.expires_at.is_some_and(|e| e <= now) {
        return Err(CoreError::Invalid("expires_at must be in the future".into()));
    }
    let token = format!("{API_KEY_MARKER}{}", random_hex(app, 32)?);
    let prefix = token[..API_KEY_PREFIX_LEN].to_string();
    let id = ApiKeyId(app.ports.ids.new_uuid());
    app.ports
        .db
        .insert_api_key(&NewApiKey {
            id,
            user_id: who.user_id,
            token_hash: api_key_digest(app, &token)?,
            hash_version: app.cfg.api_key_hash_current,
            prefix: prefix.clone(),
            label: label.clone(),
            created_at: now,
            expires_at: req.expires_at,
        })
        .await?;
    Ok(CreatedApiKey { id, token, prefix, label, created_at: now, expires_at: req.expires_at })
}

pub async fn list_api_keys(app: &App, who: &Principal) -> Result<Vec<ApiKeyInfo>, CoreError> {
    Ok(app.ports.db.list_api_keys(who.user_id).await?)
}

/// Revoke `who`'s key `id`: effective on the next request. Idempotent; a key that is unknown or someone else's is
/// `NotFound` (a caller cannot probe for other users' key ids).
pub async fn revoke_api_key(app: &App, who: &Principal, id: ApiKeyId) -> Result<(), CoreError> {
    if app.ports.db.revoke_api_key(who.user_id, id, app.ports.clock.now()).await? {
        Ok(())
    } else {
        Err(CoreError::NotFound)
    }
}

#[cfg(test)]
mod key_hash_tests;

// --- OIDC login (ADR-0050) --------------------------------------------------------------------------------------

fn oidc_error(e: OidcError) -> CoreError {
    match e {
        // the provider answered and what it answered does not authenticate: not a server fault
        OidcError::Rejected(_) => CoreError::Unauthenticated,
        OidcError::Provider(m) => CoreError::Upstream(m),
    }
}

/// Begin a login: make a `state`, a `nonce` and a PKCE verifier (256 random bits each), store them for
/// `oidc_login_ttl_secs` (the `state` as its digest), and return the provider's authorization URL to send the browser
/// to. `NotFound` when no provider is configured.
pub async fn oidc_start(app: &App) -> Result<String, CoreError> {
    let oidc = app.ports.oidc.as_ref().ok_or(CoreError::NotFound)?;
    let (state, nonce, code_verifier) = (random_hex(app, 32)?, random_hex(app, 32)?, random_hex(app, 32)?);
    let now = app.ports.clock.now();
    app.ports
        .db
        .insert_oidc_login(&NewOidcLogin {
            state_hash: crate::hash_token(&state),
            nonce: nonce.clone(),
            code_verifier: code_verifier.clone(),
            created_at: now,
            expires_at: now + thrust_contracts::chrono::TimeDelta::seconds(app.cfg.oidc_login_ttl_secs),
        })
        .await?;
    oidc.authorization_url(&state, &nonce, &code_verifier).await.map_err(oidc_error)
}

/// Finish a login. The order is the security of it: the `state` must be one this server started, unused and unexpired
/// (taking it deletes it, so a replay finds nothing), and only then is the provider asked, with the STORED verifier and
/// nonce. What the provider vouches for needs a verified email; the account is the one linked to `(issuer, sub)`, else
/// the active user with that email (who is then linked), else nobody (no just-in-time provisioning at P0, Q-0050.1).
pub async fn oidc_callback(app: &App, code: &str, state: &str) -> Result<SignedIn, CoreError> {
    let oidc = app.ports.oidc.as_ref().ok_or(CoreError::NotFound)?;
    if code.is_empty() || state.is_empty() {
        return Err(CoreError::Invalid("a code and a state are required".into()));
    }
    let now = app.ports.clock.now();
    let login = app
        .ports
        .db
        .take_oidc_login(&crate::hash_token(state))
        .await?
        .filter(|l| l.expires_at > now)
        .ok_or_else(|| CoreError::Invalid("unknown, used or expired login state".into()))?;
    // MUTANT (control run only): put the login back, so that its state works a second time. The replayed-callback tests must catch it.
    #[cfg(feature = "mutant_oidc_state_reusable")]
    app.ports
        .db
        .insert_oidc_login(&NewOidcLogin {
            state_hash: crate::hash_token(state),
            nonce: login.nonce.clone(),
            code_verifier: login.code_verifier.clone(),
            created_at: now,
            expires_at: login.expires_at,
        })
        .await?;
    let claims = oidc.exchange(code, &login.code_verifier, &login.nonce).await.map_err(oidc_error)?;

    // MUTANT (control run only): accept an unverified email. The contract suite's unverified-email test must catch it.
    #[cfg(not(feature = "mutant_unverified_email"))]
    if !claims.email_verified {
        return Err(CoreError::Forbidden);
    }
    let email = claims.email.as_deref().map(normalize_email).filter(|e| !e.is_empty()).ok_or(CoreError::Forbidden)?;

    let linked = app.ports.db.user_for_account(&claims.issuer, &claims.subject).await?;
    let user = match linked {
        Some(u) => u,
        None => {
            let u = app.ports.db.user_by_email(&email).await?.ok_or(CoreError::Forbidden)?;
            app.ports
                .db
                .link_account(&NewAccount {
                    provider: claims.issuer.clone(),
                    provider_account_id: claims.subject.clone(),
                    user_id: u.id,
                    email: Some(email),
                    created_at: now,
                })
                .await?;
            u
        }
    };
    if !user.is_active {
        return Err(CoreError::Forbidden);
    }
    mint_session(app, user.id).await
}
