//! Identity use-cases (issue #38): argon2id sign-in, the session lifecycle, CSRF, API keys and the OIDC login.
//!
//! Nothing here knows how a request arrived or how a cookie is spelled: `thrust-api` parses headers into
//! [`Credentials`] and formats the answer. Everything here takes its randomness, its clock and its hashing from the
//! ports, so a test can repeat a run and count hash work.

use argon2::{Algorithm, Argon2, Params, PasswordHasher as _, PasswordVerifier as _, Version};
use thrust_contracts::*;
use thrust_ports::*;

use crate::{App, CoreError};

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
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(DIGITS[usize::from(b >> 4)] as char);
        out.push(DIGITS[usize::from(b & 0x0f)] as char);
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

fn random_salt(app: &App) -> Result<[u8; SALT_LEN], CoreError> {
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
        .insert_session(&NewSession { token_hash: crate::hash_token(&token), user_id: user, created_at: now, expires_at })
        .await?;
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
            Some(pw) => Some(
                app.ports.hasher.hash(pw, &random_salt(app)?).map_err(|e| CoreError::Internal(e.to_string()))?,
            ),
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
    Ok(())
}

/// What a request carries, parsed by the API layer. Core decides which one counts and whether it needs a CSRF token.
#[derive(Debug, Default, Clone, Copy)]
pub struct Credentials<'a> {
    /// `Authorization: Bearer <session token>`.
    pub bearer: Option<&'a str>,
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

/// A fresh double-submit token: 128 random bits, hex.
pub fn new_csrf_token(app: &App) -> Result<String, CoreError> {
    random_hex(app, 16)
}

/// The credential of a request, and who it belongs to. A bearer token wins when there is one (it cannot be an ambient
/// credential, so it needs no CSRF token); otherwise the session cookie counts, and a mutation made with it must carry
/// the CSRF token. See `tests/credentials.rs` for the table.
pub async fn authenticate_request(app: &App, c: &Credentials<'_>) -> Result<Authenticated, CoreError> {
    if let Some(token) = c.bearer {
        let principal = crate::authenticate(app, token).await?;
        return Ok(Authenticated { principal, via: Via::Session(crate::hash_token(token)) });
    }
    if let Some(token) = c.session_cookie {
        let principal = crate::authenticate(app, token).await?;
        if c.mutating && !csrf_ok(c.csrf_cookie, c.csrf_header) {
            return Err(CoreError::Csrf);
        }
        return Ok(Authenticated { principal, via: Via::Session(crate::hash_token(token)) });
    }
    Err(CoreError::Unauthenticated)
}
