//! Sign-in with an emailed code (F-A04, D-042): a person who has an account and would rather not type a password asks for a code, is
//! mailed six digits, and signs in with them. Two use-cases: [`request_login_code`] makes a code and mails it, and
//! [`sign_in_with_code`] takes an address and a code and makes a session, as a password does.
//!
//! The code is six digits (the pin's own: `secrets.randbelow(900000) + 100000`), drawn without bias, shown once in the mail and stored
//! only as a digest. Six digits are guessable, so what protects an account is what limits the guessing, and all of it is the pin's: a
//! code lives ten minutes (`login_code_ttl_secs`), allows five tries (`login_code_max_attempts`, the fifth wrong one ends it), a window
//! of ten minutes makes at most four codes for an address (`login_code_window_secs`, `login_code_max_issued`), and a right code is
//! used up with the count of codes made, so one code signs one person in once. On top of that Thrust limits the tries per address, as it
//! does a password's (`sign_in_rate_limit`).
//!
//! Sign-in only. The pin also makes an account out of a code (`magic-sign-up`); Thrust does not (the owner, 2026-11-16: "E4 is sign-in
//! only"), so a code is made, and mailed, only for an address an account has: nothing is sent to a stranger, and what is answered does not
//! say which (the pin's answer to an address nobody has is the same as to one somebody has).
//!
//! **Dark with the reset by email**: a deployment with no mail service, or no configured base URL, answers [`CodeRequestError::NotConfigured`]
//! to everything, and `/api/instances/` says `is_smtp_configured: false` (the app offers a code only when it says true). The two share one
//! switch ([`mail_configured`]), because the app has one: it draws "Forgot your password?" and "Sign in with unique code" from the same
//! flag, so a deployment that could answer one and not the other would draw a button that goes nowhere.

use thiserror::Error;
use thrust_contracts::chrono::TimeDelta;
use thrust_contracts::{Timestamp, UserId};
use thrust_ports::{EmailMessage, LoginCodeCheck, LoginCodeIssue, NewLoginCode, RateDecision};

use crate::auth::{is_valid_email, mint_session, normalize_email};
use crate::{hash_token, App, CoreError, SignedIn};

/// How many different codes there are: 100000 to 999999.
const CODE_SPAN: u32 = 900_000;
/// The largest multiple of [`CODE_SPAN`] a `u32` holds. A draw at or above it is thrown away: `bits % CODE_SPAN` would give the first
/// 2^32 mod 900000 codes one more way to come up than the rest.
const CODE_LIMIT: u32 = (u32::MAX / CODE_SPAN) * CODE_SPAN;
/// Draws before the random source is given up on. Each one is thrown away with a probability under one in twenty thousand.
const MAX_DRAWS: usize = 16;
/// The limiter key of an address's tries at a code: the sign-in's class (`signin:`, which is the one the Worker's `RL_SIGNIN` binding counts: the
/// Cloudflare adapter has a binding for `signin:` and `v1:` keys and refuses any other, which no in-memory limiter notices and the `worker-cf`
/// cell did), with a segment of its own, so that tries at a code and tries at a password are counted apart. No address has a `:` in it.
const LIMITER_KEY: &str = "signin:code:";
/// The longest text that is looked at as a code; anything longer is no code, and is refused without a try being counted or a hash made.
const MAX_CODE_LEN: usize = 32;

/// The six digits that 32 random bits stand for, or `None` for the few draws that would make some codes likelier than others.
pub fn code_from_bits(bits: u32) -> Option<u32> {
    (bits < CODE_LIMIT).then(|| 100_000 + bits % CODE_SPAN)
}

fn draw_code(app: &App) -> Result<String, CoreError> {
    for _ in 0..MAX_DRAWS {
        let mut buf = [0u8; 4];
        app.ports.rng.fill(&mut buf).map_err(|e| CoreError::Internal(e.to_string()))?;
        if let Some(code) = code_from_bits(u32::from_be_bytes(buf)) {
            return Ok(code.to_string());
        }
    }
    Err(CoreError::Internal("the random source gave no usable bits".into()))
}

/// Whether this deployment can mail, which is what the app's `is_smtp_configured` says: a mail service, and the address that a link in a
/// mail starts with. The reset by email needs both; the code needs the first, but the app's one flag is what it reads, so they are one.
pub fn mail_configured(app: &App) -> bool {
    app.ports.email.is_some() && app.cfg.public_base_url.is_some()
}

/// What `request_login_code` did for a request that was well formed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodeRequested {
    /// A message with the code was accepted by the mail service.
    Sent,
    /// Nothing was made or sent: no account has the address. The answer is the one a sent code gets, so a request is no way to learn
    /// which addresses have an account (the pin's `magic-generate` does not say either: it mails a code to any address, for its sign-up).
    NothingSent,
}

#[derive(Debug, Error)]
pub enum CodeRequestError {
    /// No mail service, or no address to build a link on: this deployment cannot send.
    #[error("this deployment cannot send email")]
    NotConfigured,
    #[error("not an email address")]
    InvalidEmail,
    /// The account has been made as many codes as a window allows. The pin answers `EMAIL_CODE_ATTEMPT_EXHAUSTED_SIGN_IN`.
    #[error("too many codes have been made for this address")]
    Capped,
    /// The mail service refused the message or could not be reached. The code that was made for it is taken back, with its place in the
    /// count, so a mail service that is down does not use up the codes a person may ask for.
    #[error("the mail service did not take the message: {0}")]
    Mail(String),
    #[error(transparent)]
    Core(#[from] CoreError),
}

#[derive(Debug, Error)]
pub enum CodeSignInError {
    /// No account has the address (the pin's `USER_DOES_NOT_EXIST`, as for a password).
    #[error("no such user")]
    UnknownUser,
    #[error("this deployment cannot send email")]
    NotConfigured,
    /// A code works for the address and this is not it.
    #[error("the code is not right")]
    Invalid,
    /// No code works for the address: none was asked for, it was used, its time is over, or it was exhausted.
    #[error("the code has expired")]
    Expired,
    /// This was the last wrong try the code allows: it is gone.
    #[error("too many wrong tries")]
    Exhausted,
    /// The right code, for an account that is deactivated. The code is used up.
    #[error("the account is deactivated")]
    Deactivated,
    #[error("too many tries")]
    RateLimited { retry_after_secs: u64 },
    #[error(transparent)]
    Core(#[from] CoreError),
}

/// What [`issue_login_code`] made: the digits, which are mailed once and nowhere else.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssuedLoginCode {
    pub code: String,
    pub expires_at: Timestamp,
}

/// Make a code for `user` and store its digest. `None` when the user has been made as many codes as a window allows: nothing was stored.
/// Used by [`request_login_code`], and by the test seam that tests use to get a code without a mailbox.
pub async fn issue_login_code(app: &App, user: UserId) -> Result<Option<IssuedLoginCode>, CoreError> {
    issue_login_code_with(
        app,
        user,
        app.cfg.login_code_ttl_secs,
        app.cfg.login_code_window_secs,
        app.cfg.login_code_max_issued,
    )
    .await
}

/// [`issue_login_code`] with the lifetime, the window and the count given: what the test seam of a deployed target uses to make a code that
/// has expired already (a negative `ttl_secs`), or that no earlier code of a window blocks.
pub async fn issue_login_code_with(
    app: &App,
    user: UserId,
    ttl_secs: i64,
    window_secs: i64,
    max_issued: u32,
) -> Result<Option<IssuedLoginCode>, CoreError> {
    let code = draw_code(app)?;
    let now = app.ports.clock.now();
    let expires_at = now + TimeDelta::seconds(ttl_secs);
    // MUTANT (control run only): the window's count is not enforced, so an address can be mailed a code as often as it is asked for.
    // The cap tests must catch it.
    #[cfg(feature = "mutant_code_cap_ignored")]
    let max_issued = u32::MAX;
    let stored = app
        .ports
        .db
        .issue_login_code(
            &NewLoginCode { user_id: user, code_hash: hash_token(&code), created_at: now, expires_at },
            window_secs,
            max_issued,
        )
        .await?;
    Ok((stored == LoginCodeIssue::Issued).then_some(IssuedLoginCode { code, expires_at }))
}

/// The pin's order, then Thrust's: the address is one (the pin has no answer for one that is not: its `validate_email` raises and the
/// answer is a 500, which a route of Thrust does not write itself), this deployment can mail (`SMTP_NOT_CONFIGURED`), and then a code is
/// made and mailed for an account that has the address. An address that no account has is answered as a mailed one.
pub async fn request_login_code(app: &App, email: &str) -> Result<CodeRequested, CodeRequestError> {
    let email = normalize_email(email);
    if !is_valid_email(&email) {
        return Err(CodeRequestError::InvalidEmail);
    }
    let (true, Some(sender)) = (mail_configured(app), app.ports.email.as_ref()) else {
        return Err(CodeRequestError::NotConfigured);
    };
    let known = app.ports.db.user_by_email(&email).await.map_err(CoreError::from)?;
    let Some(user) = known else {
        // MUTANT (control run only): an address no account has is mailed a message all the same, so a stranger is sent mail. The tests of
        // what is sent, and to whom, must catch it.
        #[cfg(feature = "mutant_code_mails_unknown")]
        let _ = sender
            .send(&EmailMessage {
                to: email,
                subject: "Your Thrust sign-in code".into(),
                text: "A code was asked for.\n".into(),
                html: None,
            })
            .await;
        return Ok(CodeRequested::NothingSent);
    };
    let Some(issued) = issue_login_code(app, user.id).await? else {
        return Err(CodeRequestError::Capped);
    };
    let minutes = app.cfg.login_code_ttl_secs / 60;
    let message = EmailMessage {
        to: email,
        subject: format!("Your Thrust sign-in code is {}", issued.code),
        text: format!(
            "Your Thrust sign-in code is {}.\n\n\
             It works once, for {minutes} minutes. If you did not ask for it, ignore this message: no one can sign in to your account without it.\n",
            issued.code
        ),
        html: None,
    };
    if let Err(e) = sender.send(&message).await {
        // the code was never delivered: it must not work if it is found, and must not count against the person
        let _ = app.ports.db.discard_login_code(user.id, &hash_token(&issued.code)).await;
        return Err(CodeRequestError::Mail(e.to_string()));
    }
    Ok(CodeRequested::Sent)
}

/// Sign in with an address and a code. The order is the pin's, with Thrust's limiter first: the tries per address, an account with the
/// address (`USER_DOES_NOT_EXIST`), a deployment that can mail, the code (right, wrong, the last wrong one, or none that works), and
/// last the account's being active, so a code that is right for a deactivated account is used up and refused (`USER_ACCOUNT_DEACTIVATED`).
pub async fn sign_in_with_code(app: &App, email: &str, code: &str) -> Result<SignedIn, CodeSignInError> {
    let email = normalize_email(email);
    let code = code.trim();
    let decision = app
        .ports
        .limiter
        .hit(&format!("{LIMITER_KEY}{email}"), app.cfg.sign_in_rate_limit, app.cfg.sign_in_rate_window_secs)
        .await
        .map_err(CoreError::from)?;
    if let RateDecision::Limited { retry_after_secs } = decision {
        return Err(CodeSignInError::RateLimited { retry_after_secs });
    }
    let Some(user) = app.ports.db.user_by_email(&email).await.map_err(CoreError::from)? else {
        return Err(CodeSignInError::UnknownUser);
    };
    if !mail_configured(app) {
        return Err(CodeSignInError::NotConfigured);
    }
    if code.chars().count() > MAX_CODE_LEN {
        return Err(CodeSignInError::Invalid);
    }
    // MUTANT (control run only): the tries a code allows are not counted against any limit, so it can be guessed for as long as it lives.
    // The exhaustion tests must catch it.
    #[cfg(not(feature = "mutant_code_attempts_unbounded"))]
    let max_attempts = app.cfg.login_code_max_attempts;
    #[cfg(feature = "mutant_code_attempts_unbounded")]
    let max_attempts = u32::MAX;
    let now = app.ports.clock.now();
    match app.ports.db.check_login_code(user.id, &hash_token(code), now, max_attempts).await.map_err(CoreError::from)? {
        LoginCodeCheck::Matched => {}
        LoginCodeCheck::Wrong => return Err(CodeSignInError::Invalid),
        LoginCodeCheck::Exhausted => return Err(CodeSignInError::Exhausted),
        LoginCodeCheck::Expired => return Err(CodeSignInError::Expired),
    }
    // MUTANT (control run only): a deactivated account signs in with a right code. The deactivation tests must catch it.
    #[cfg(not(feature = "mutant_code_deactivated_signs_in"))]
    if !user.is_active {
        return Err(CodeSignInError::Deactivated);
    }
    Ok(mint_session(app, user.id).await?)
}
