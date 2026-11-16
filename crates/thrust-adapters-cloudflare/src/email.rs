//! The Worker's `send_email` binding as the [`EmailSender`] port (E3 of the password reset by email, D-041, Q-0073.1, ADR-0073):
//! Cloudflare Email Sending on the Cloudflare adapter set, where the self-hosted set has SMTP (`thrust-adapters-selfhosted::smtp`).
//!
//! What a deployment sets, read once per request by [`EmailConfig::from_lookup`] (the composition root passes it the Worker's vars):
//!
//! | variable | |
//! |---|---|
//! | `THRUST_EMAIL_FROM` | the sender's address, `no-reply@notify.example.com`: an address on a domain the account has Email Sending enabled for (`wrangler email sending list`). Unset is "no mail service": the port is `None` and a reset is off (the app says so and draws no link) |
//! | `THRUST_EMAIL_FROM_NAME` | the name shown beside it (`Thrust`). Optional; refused without a sender |
//!
//! and the binding itself, `[[send_email]] name = "EMAIL"` ([`EMAIL_BINDING`]). A sender with no binding, a sender that is not a plain
//! address and a name with no sender are errors the Worker reports instead of starting without mail, in the spirit of the OIDC
//! provider's configuration: a reset nobody notices is not on is worse than a Worker that says so.
//!
//! What is in this file is split so that what can be tested off wasm32 is. [`EmailConfig`], [`outgoing`] (what a message becomes: the
//! checks that stop an address or a subject from carrying a second header or a second recipient, and the fields the binding is given)
//! and [`classify`] (what the binding's refusal means to the port) are plain functions with their own generative tests
//! (`tests/email.rs`). [`CfEmail`] is the few lines of JavaScript glue around them, and it runs only in workerd: the `mail` pass of the
//! contract suite on a Worker target walks it (the local simulator of the binding is its mail server, `scripts/contract.sh`).
//!
//! The binding answers with an id (`messageId`) when the service has accepted the message, which is what the port returns; that is
//! not delivery to a mailbox, which nothing here can see.
//!
//! **What the binding's errors are.** A rejected `send()` carries a `code` (`E_SENDER_NOT_VERIFIED`, `E_RATE_LIMIT_EXCEEDED`, ... the
//! table in Cloudflare's Workers API page) and a message. The local simulator's refusals have a message and no code
//! ("email from x not allowed"), and the one recorded refusal of the deployed binding (spike 0002) has the same words, so a refusal
//! with no code is read by its message: "not allowed" is a refusal, anything else is an outage. An unknown code is an outage too: a
//! message sent again later is the lesser harm than one dropped for a refusal that was only a code this table has not met.

use std::fmt;

use async_trait::async_trait;
use thrust_ports::{EmailError, EmailMessage, EmailSender};
use worker::send::SendFuture;

/// The name of the Worker's binding: `[[send_email]] name = "EMAIL"`.
pub const EMAIL_BINDING: &str = "EMAIL";

const FROM: &str = "THRUST_EMAIL_FROM";
const FROM_NAME: &str = "THRUST_EMAIL_FROM_NAME";

/// The longest address (RFC 5321's path, 254 characters), local part (64) and display name (a name shown beside an address is
/// short; this stops a paragraph).
const MAX_ADDRESS: usize = 254;
const MAX_LOCAL: usize = 64;
const MAX_NAME: usize = 100;
/// RFC 5322's longest header line, which a subject is one of.
const MAX_SUBJECT: usize = 998;

// ---------------------------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------------------------

/// A configuration that cannot be used, and why, in words that name the variable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmailConfigError(pub String);

impl fmt::Display for EmailConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for EmailConfigError {}

/// Who the mail is from. Nothing in it is a secret: the binding is the credential.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmailConfig {
    /// A plain address on a domain the account sends from.
    pub from: String,
    /// The name beside it, when there is one.
    pub name: Option<String>,
}

impl EmailConfig {
    /// The mail service's configuration from `get` (a variable's value, or `None` when it is not set or is empty), or `None` when
    /// the deployment has none (`THRUST_EMAIL_FROM` unset). A name with no sender, and a sender or a name that is not usable, are
    /// errors.
    pub fn from_lookup(get: impl Fn(&str) -> Option<String>) -> Result<Option<EmailConfig>, EmailConfigError> {
        let get = |name: &str| get(name).filter(|v| !v.is_empty());
        let (from, name) = (get(FROM), get(FROM_NAME));
        let Some(from) = from else {
            return match name {
                Some(_) => {
                    Err(EmailConfigError(format!("{FROM_NAME} is set but {FROM} is not: there is no sender to name")))
                }
                None => Ok(None),
            };
        };
        if !is_plain_address(&from) {
            return Err(EmailConfigError(format!(
                "{FROM} must be one plain address (no name, no angle brackets, no list), like no-reply@notify.example.com"
            )));
        }
        if let Some(n) = &name {
            if !is_display_name(n) {
                return Err(EmailConfigError(format!(
                    "{FROM_NAME} must be a name of at most {MAX_NAME} characters with no control character, quote, angle bracket or backslash"
                )));
            }
        }
        Ok(Some(EmailConfig { from, name }))
    }
}

/// One address and nothing else: a local part of atoms (letters, digits and ``!#$%&'*+-/=?^_`{|}~`` between single dots), `@`, and a
/// domain of at least two labels of letters, digits and inner hyphens. No space, no quote, no angle bracket, no comma, no control
/// character: whatever would let a second recipient, a name or a header ride in an address is not an address here. (It is the shape
/// the core accepts, less its rule for the last label, which is the core's to apply: this is the guard in front of the binding.)
pub fn is_plain_address(s: &str) -> bool {
    if s.is_empty() || s.len() > MAX_ADDRESS {
        return false;
    }
    let Some((local, domain)) = s.split_once('@') else { return false };
    let atom = |c: char| c.is_ascii_alphanumeric() || "!#$%&'*+/=?^_`{|}~-".contains(c);
    let local_ok =
        !local.is_empty() && local.len() <= MAX_LOCAL && local.split('.').all(|p| !p.is_empty() && p.chars().all(atom));
    let label = |l: &str| {
        !l.is_empty()
            && l.len() <= 63
            && !l.starts_with('-')
            && !l.ends_with('-')
            && l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
    };
    let mut labels = 0;
    let domain_ok = domain.split('.').all(|l| {
        labels += 1;
        label(l)
    });
    local_ok && domain_ok && labels >= 2
}

/// A name shown beside the sender's address: printable, short, with nothing that would end it early or start a header.
fn is_display_name(s: &str) -> bool {
    !s.trim().is_empty() && s.chars().count() <= MAX_NAME && !s.chars().any(|c| c.is_control() || "\"<>\\".contains(c))
}

// ---------------------------------------------------------------------------------------------
// What a message becomes
// ---------------------------------------------------------------------------------------------

/// The fields the binding is given for one message: the whole of what leaves the Worker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outgoing {
    pub from: String,
    pub from_name: Option<String>,
    pub to: String,
    pub subject: String,
    pub text: String,
    pub html: Option<String>,
}

/// Check `message` and say what the binding is given for it. A message the binding must not be asked about is
/// [`EmailError::Rejected`] and nothing is sent: a recipient that is not one plain address (a name, a list, a header smuggled into
/// it), a subject that is empty, too long or has a line break in it. The core makes the recipient from an address it has checked
/// and the subject from a constant, so these are the second line, for the day a caller does not.
pub fn outgoing(cfg: &EmailConfig, message: &EmailMessage) -> Result<Outgoing, EmailError> {
    #[cfg(not(feature = "mutant_cf_email_recipient_unchecked"))]
    if !is_plain_address(&message.to) {
        return Err(EmailError::Rejected("the recipient is not one plain address".into()));
    }
    if message.subject.trim().is_empty() {
        return Err(EmailError::Rejected("the message has no subject".into()));
    }
    if message.subject.len() > MAX_SUBJECT || message.subject.chars().any(char::is_control) {
        return Err(EmailError::Rejected(
            "the subject is too long, or has a line break or another control character in it".into(),
        ));
    }
    #[cfg(feature = "mutant_cf_email_from_is_recipient")]
    let from = message.to.clone();
    #[cfg(not(feature = "mutant_cf_email_from_is_recipient"))]
    let from = cfg.from.clone();
    Ok(Outgoing {
        from,
        from_name: cfg.name.clone(),
        to: message.to.clone(),
        subject: message.subject.clone(),
        text: message.text.clone(),
        html: message.html.clone(),
    })
}

// ---------------------------------------------------------------------------------------------
// What the binding's refusal means
// ---------------------------------------------------------------------------------------------

/// Codes of a refusal that would come again for the same message (Cloudflare's Workers API page, "Error codes"): the message is
/// malformed or too large, its headers are not allowed, the sender or the recipient is one the service will not use.
const REFUSED: [&str; 16] = [
    "E_VALIDATION_ERROR",
    "E_FIELD_MISSING",
    "E_TOO_MANY_RECIPIENTS",
    "E_TOO_MANY_ATTACHMENTS",
    "E_SENDER_NOT_VERIFIED",
    "E_RECIPIENT_NOT_ALLOWED",
    "E_RECIPIENT_SUPPRESSED",
    "E_SENDER_DOMAIN_NOT_AVAILABLE",
    "E_CONTENT_TOO_LARGE",
    "E_HEADER_NOT_ALLOWED",
    "E_HEADER_USE_API_FIELD",
    "E_HEADER_VALUE_INVALID",
    "E_HEADER_VALUE_TOO_LONG",
    "E_HEADER_NAME_INVALID",
    "E_HEADERS_TOO_LARGE",
    "E_HEADERS_TOO_MANY",
];

/// What the binding's rejection means to the port. `code` is the error's `code` property when it has one, `message` its message.
/// A code in [`REFUSED`] is [`EmailError::Rejected`]; every other code (the limits, an account that is paused, an internal error,
/// a delivery that failed, and a code this table has not met) is [`EmailError::Unavailable`]; and with no code, the words "not
/// allowed" are a refusal (the local simulator's, and the deployed binding's for a sender domain the account does not own) and
/// anything else is an outage. The text carries the code and the message.
pub fn classify(code: Option<&str>, message: &str) -> EmailError {
    let text = match (code, message.is_empty()) {
        (Some(c), false) => format!("{c}: {message}"),
        (Some(c), true) => c.to_string(),
        (None, false) => message.to_string(),
        (None, true) => "the mail service refused the message and said nothing".to_string(),
    };
    let refused = match code {
        Some(c) => REFUSED.contains(&c),
        None => message.to_ascii_lowercase().contains("not allowed"),
    };
    #[cfg(feature = "mutant_cf_email_swap_refusal_and_outage")]
    let refused = !refused;
    #[cfg(feature = "mutant_cf_email_unknown_code_is_refused")]
    let refused = refused || code.is_some();
    if refused {
        EmailError::Rejected(text)
    } else {
        EmailError::Unavailable(text)
    }
}

// ---------------------------------------------------------------------------------------------
// The binding
// ---------------------------------------------------------------------------------------------

/// The `send_email` binding as the port. Runs in workerd only (see the module's note).
pub struct CfEmail {
    binding: worker::SendEmail,
    cfg: EmailConfig,
}

impl CfEmail {
    pub fn new(binding: worker::SendEmail, cfg: EmailConfig) -> Self {
        Self { binding, cfg }
    }
}

/// `code` and `message` of whatever a rejected `send()` threw. A thrown value that is not an `Error`, or has no `code`, gives what
/// it has: the classifier reads a missing code by the message.
fn parts_of(thrown: &worker::wasm_bindgen::JsValue) -> (Option<String>, String) {
    let get = |name: &str| {
        worker::js_sys::Reflect::get(thrown, &worker::wasm_bindgen::JsValue::from_str(name))
            .ok()
            .and_then(|v| v.as_string())
    };
    (get("code").filter(|c| !c.is_empty()), get("message").or_else(|| thrown.as_string()).unwrap_or_default())
}

#[async_trait]
impl EmailSender for CfEmail {
    async fn send(&self, message: &EmailMessage) -> Result<String, EmailError> {
        let out = outgoing(&self.cfg, message)?;
        SendFuture::new(async move {
            let named = out.from_name.as_deref().map(|name| worker::EmailAddress::new(name, &out.from));
            let mut builder = match &named {
                Some(from) => worker::SendEmailBuilder::builder_with_email_address_and_str(from, &out.to, &out.subject),
                None => worker::SendEmailBuilder::builder(&out.from, &out.to, &out.subject),
            }
            .text(&out.text);
            if let Some(html) = &out.html {
                builder = builder.html(html);
            }
            match self.binding.send_with_builder(&builder.build()).await {
                Ok(sent) => Ok(sent.message_id()),
                Err(thrown) => {
                    let (code, message) = parts_of(thrown.as_ref());
                    Err(classify(code.as_deref(), &message))
                }
            }
        })
        .await
    }
}
