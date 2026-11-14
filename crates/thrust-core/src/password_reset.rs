//! Email recovery (F-A03, D-041): a person who cannot sign in asks for a link by email, and the link lets them choose a new
//! password. Two use-cases: [`request_password_reset`] makes a token and mails the link; [`reset_password`] takes a link's
//! `uidb64` and `token` and a new password.
//!
//! The token is 256 random bits, shown once (in the mail) and stored only as its digest, like a session's. It is single use,
//! it expires (`password_reset_ttl_secs`), and a user has at most one that works. It is not consumed by a request that fails the
//! password checks: a weak password must not burn the link, because the person would have to ask again (the pin's token is
//! stateless and behaves the same way). A reset ends every session of the account and signs nobody in, as the pin's does.
//!
//! The link starts with `CoreConfig::public_base_url`, which a deployment configures: it is never taken from a request's `Host`
//! header, which a caller controls, so a forged one could mail a victim a link to someone else's page with a working token.
//!
//! A deployment with no mail service (`Ports::email` is `None`) or no base URL cannot mail: every request is
//! [`ResetRequestError::NotConfigured`], and the app is told so (`is_smtp_configured`), so no one is offered the link.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use thiserror::Error;
use thrust_contracts::chrono::TimeDelta;
use thrust_contracts::{Timestamp, UserId};
use thrust_ports::{EmailMessage, NewPasswordReset};

use crate::auth::{is_valid_email, normalize_email, password_is_strong, random_hex, random_salt};
use crate::{hash_token, App, CoreError};

/// The bits of a reset token.
const TOKEN_BYTES: usize = 32;

/// What `request_password_reset` did for a request that was well formed and for a known account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResetRequested {
    /// A message with the link was accepted by the mail service.
    Sent,
    /// Nothing was made or sent: a link was made for the account a moment ago and still works (the cooldown), or the account is
    /// deactivated. The person is told what they are told after a send, so a request is no way to learn which.
    NothingSent,
}

#[derive(Debug, Error)]
pub enum ResetRequestError {
    /// No mail service, or no address to build the link on: this deployment cannot send a reset.
    #[error("this deployment cannot send email")]
    NotConfigured,
    #[error("not an email address")]
    InvalidEmail,
    /// No account has that address. The pin says so (`USER_DOES_NOT_EXIST`), as its sign-in and email-check do.
    #[error("no such user")]
    UnknownUser,
    /// The mail service refused the message or could not be reached. The link that was made for it is gone again, so a second
    /// request is not held back by the cooldown of a mail that was never sent.
    #[error("the mail service did not take the message: {0}")]
    Mail(String),
    #[error(transparent)]
    Core(#[from] CoreError),
}

#[derive(Debug, Error)]
pub enum PasswordResetError {
    /// The link's `uidb64` or `token` is not good: it does not decode, names no account, is unknown, was used, has expired, or
    /// is another account's. The pin does not say which, and neither does this.
    #[error("the reset link is not valid")]
    InvalidToken,
    #[error("no password")]
    PasswordMissing,
    #[error("password too weak")]
    PasswordTooWeak,
    #[error(transparent)]
    Core(#[from] CoreError),
}

/// What [`issue_password_reset`] made: the two values the link carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssuedReset {
    pub uidb64: String,
    pub token: String,
    pub expires_at: Timestamp,
}

/// The user id as the link carries it: the base64url of its text, without padding (the pin's `urlsafe_base64_encode(smart_bytes(id))`).
pub fn uidb64(user: UserId) -> String {
    URL_SAFE_NO_PAD.encode(user.to_string())
}

fn user_from_uidb64(s: &str) -> Option<UserId> {
    let bytes = URL_SAFE_NO_PAD.decode(s.trim_end_matches('=')).ok()?;
    String::from_utf8(bytes).ok()?.parse().ok()
}

/// Make a reset token for `user` and store its digest. `None` when a token was made for the user within the cooldown: the one that
/// was made still works, and nothing new is stored. Used by [`request_password_reset`], and by the test seam that tests use to get a
/// token without a mailbox.
pub async fn issue_password_reset(app: &App, user: UserId) -> Result<Option<IssuedReset>, CoreError> {
    let token = random_hex(app, TOKEN_BYTES)?;
    let now = app.ports.clock.now();
    let expires_at = now + TimeDelta::seconds(app.cfg.password_reset_ttl_secs);
    let not_before = now - TimeDelta::seconds(app.cfg.password_reset_cooldown_secs);
    // MUTANT (control run only): the cooldown is ignored, so a request mails the account every time. The cooldown tests must catch it.
    #[cfg(feature = "mutant_reset_ignores_cooldown")]
    let not_before = now + TimeDelta::days(3650);
    let stored = app
        .ports
        .db
        .insert_password_reset(
            &NewPasswordReset { token_hash: hash_token(&token), user_id: user, created_at: now, expires_at },
            not_before,
        )
        .await?;
    Ok(stored.then(|| IssuedReset { uidb64: uidb64(user), token, expires_at }))
}

/// Every character but the unreserved ones, as `%XX`: the email address in the link's query.
fn percent_encode(s: &str) -> String {
    use std::fmt::Write;
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
            out.push(b as char);
        } else {
            // writing to a `String` cannot fail
            let _ = write!(out, "%{b:02X}");
        }
    }
    out
}

/// The link the app's reset page reads (`uidb64`, `token`, `email` from its query, as the pin's mail carries them).
fn reset_link(base: &str, issued: &IssuedReset, email: &str) -> String {
    format!(
        "{}/accounts/reset-password/?uidb64={}&token={}&email={}",
        base.trim_end_matches('/'),
        issued.uidb64,
        issued.token,
        percent_encode(email)
    )
}

/// The pin's order, then Thrust's: this deployment can mail (the pin's `SMTP_NOT_CONFIGURED`), the address is one, an account has it
/// (`USER_DOES_NOT_EXIST`), and then a link is made and mailed. A deactivated account is told what a mailed one is told and mailed
/// nothing.
pub async fn request_password_reset(app: &App, email: &str) -> Result<ResetRequested, ResetRequestError> {
    let (Some(sender), Some(base)) = (app.ports.email.as_ref(), app.cfg.public_base_url.as_deref()) else {
        return Err(ResetRequestError::NotConfigured);
    };
    let email = normalize_email(email);
    if !is_valid_email(&email) {
        return Err(ResetRequestError::InvalidEmail);
    }
    let Some(user) = app.ports.db.user_by_email(&email).await.map_err(CoreError::from)? else {
        return Err(ResetRequestError::UnknownUser);
    };
    if !user.is_active {
        return Ok(ResetRequested::NothingSent);
    }
    let Some(issued) = issue_password_reset(app, user.id).await? else {
        return Ok(ResetRequested::NothingSent);
    };
    let minutes = app.cfg.password_reset_ttl_secs / 60;
    let message = EmailMessage {
        to: email.clone(),
        subject: "Reset your Thrust password".into(),
        text: format!(
            "Someone asked to reset the password of the Thrust account for {email}.\n\n\
             To choose a new password, open this link within {minutes} minutes:\n\n{}\n\n\
             If you did not ask for this, ignore this message: your password stays as it is.\n",
            reset_link(base, &issued, &email)
        ),
        html: None,
    };
    if let Err(e) = sender.send(&message).await {
        // the link was never delivered: it must not hold the next request back, or work if it is found
        let _ = app.ports.db.take_password_reset(&hash_token(&issued.token)).await;
        return Err(ResetRequestError::Mail(e.to_string()));
    }
    Ok(ResetRequested::Sent)
}

/// Set a new password with a link's values. The order is the pin's: the link is checked (it decodes, is for an account, is unknown or
/// expired or another's), then the password is (missing, then too weak); the token is used up only when the password has passed, so a
/// refusal of the password leaves the link working. The account's sessions all end and no one is signed in.
pub async fn reset_password(app: &App, uidb64: &str, token: &str, password: &str) -> Result<(), PasswordResetError> {
    let token_hash = hash_token(token);
    let now = app.ports.clock.now();
    let Some(uid) = user_from_uidb64(uidb64) else {
        return Err(PasswordResetError::InvalidToken);
    };
    let Some(row) = app.ports.db.find_password_reset(&token_hash).await.map_err(CoreError::from)? else {
        return Err(PasswordResetError::InvalidToken);
    };
    // MUTANT (control run only): an expired link still works. The expiry tests must catch it.
    #[cfg(not(feature = "mutant_reset_ignores_expiry"))]
    if row.expires_at <= now {
        return Err(PasswordResetError::InvalidToken);
    }
    // MUTANT (control run only): a link works for an account other than the one it was made for. The mismatch tests must catch it.
    #[cfg(not(feature = "mutant_reset_uid_unchecked"))]
    if row.user_id != uid {
        return Err(PasswordResetError::InvalidToken);
    }
    let _ = uid;
    let Some(user) = app.ports.db.user_by_id(row.user_id).await.map_err(CoreError::from)? else {
        return Err(PasswordResetError::InvalidToken);
    };
    // MUTANT (control run only): the token is used up before the password is checked, so a weak password burns the link.
    #[cfg(feature = "mutant_reset_takes_token_first")]
    let _ = app.ports.db.take_password_reset(&token_hash).await.map_err(CoreError::from)?;
    if password.is_empty() {
        return Err(PasswordResetError::PasswordMissing);
    }
    // MUTANT (control run only): any password is accepted. The strength tests must catch it.
    #[cfg(not(feature = "mutant_reset_skips_strength"))]
    if !password_is_strong(password) {
        return Err(PasswordResetError::PasswordTooWeak);
    }
    // MUTANT (control run only): the link is not used up, so it resets the password again. The single-use tests must catch it.
    #[cfg(not(any(feature = "mutant_reset_token_reusable", feature = "mutant_reset_takes_token_first")))]
    if app.ports.db.take_password_reset(&token_hash).await.map_err(CoreError::from)?.is_none() {
        // another request used it between the check and now
        return Err(PasswordResetError::InvalidToken);
    }
    let phc = app
        .ports
        .hasher
        .hash(password, &random_salt(app)?)
        .map_err(|e| PasswordResetError::Core(CoreError::Internal(e.to_string())))?;
    if !app.ports.db.set_password_hash(&app.system, user.id, &phc).await.map_err(CoreError::from)? {
        return Err(PasswordResetError::InvalidToken);
    }
    // MUTANT (control run only): the account's other sessions survive the reset. The session tests must catch it.
    #[cfg(not(feature = "mutant_reset_keeps_sessions"))]
    app.ports.db.revoke_user_sessions(user.id, now).await.map_err(CoreError::from)?;
    Ok(())
}
