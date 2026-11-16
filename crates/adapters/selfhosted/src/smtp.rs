//! SMTP as the [`EmailSender`] port, on `lettre`, for the self-hosted set (E2 of the password reset by email, D-041, Q-0073.1).
//!
//! What a deployment sets, read once at start by [`SmtpConfig::from_lookup`] (the composition root passes it the environment; a
//! password is a secret, so it comes from the environment and from nowhere else):
//!
//! | variable | |
//! |---|---|
//! | `THRUST_SMTP_HOST` | the relay. Unset is "no mail service": the port is `None` and a reset is off (the app says so, and draws no link) |
//! | `THRUST_SMTP_FROM` | the sender, `no-reply@example.com` or `Thrust <no-reply@example.com>`. Required with a host |
//! | `THRUST_SMTP_TLS` | `starttls` (the default: the connection is upgraded and a relay that cannot is refused, never used in the clear), `tls` (the connection is encrypted from the first byte, port 465) or `none` |
//! | `THRUST_SMTP_PORT` | by the mode: 587, 465, 25 |
//! | `THRUST_SMTP_USERNAME`, `THRUST_SMTP_PASSWORD` | both or neither; refused with `tls=none`, which would send the password in the clear |
//! | `THRUST_SMTP_CA_FILE` | a PEM file of the CA that signed the relay's certificate. When it is set the relay's certificate must chain to it **and to nothing else**; unset, the Mozilla roots decide |
//! | `THRUST_SMTP_TIMEOUT_SECS` | the most one message may take, connecting included (10) |
//!
//! A half-written configuration (a port with no host, a username with no password, a mode that is not one of the three) is an error
//! at start, in the OIDC provider's spirit: a login nobody notices is missing is worse than a server that does not come up.
//!
//! What it does with a message: one connection per message (no pool: a reset mail is rare and a pooled connection a relay closed is
//! one more thing to fail), a text part and, when there is one, an HTML alternative; a `Message-ID` it makes and returns, which is the id
//! the port promises (the relay's own queue id is its own to log). The recipient must be a bare address: a name, a list or a header
//! smuggled into it is refused before anything is sent.
//!
//! Answers, for the port's two kinds of failure: a relay's permanent refusal (5xx: an address it will not send to, a sender it does
//! not own, a password it does not know) and a message `lettre` cannot make are [`EmailError::Rejected`]; a transient reply (4xx), a
//! relay that cannot be reached or does not answer in time, and a TLS handshake that fails are [`EmailError::Unavailable`].

use std::fmt;
use std::path::PathBuf;
use std::time::Duration;

use async_trait::async_trait;
use lettre::message::{Mailbox, MultiPart, SinglePart};
use lettre::transport::smtp::authentication::Credentials;
use lettre::transport::smtp::client::{Certificate, CertificateStore, Tls, TlsParameters};
use lettre::transport::smtp::extension::ClientId;
use lettre::{Address, AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};
use thrust_ports::{EmailError, EmailMessage, EmailSender};

/// Every variable this module reads, so that one of them without the host is noticed.
const HOST: &str = "THRUST_SMTP_HOST";
const OTHERS: [&str; 7] = [
    "THRUST_SMTP_FROM",
    "THRUST_SMTP_TLS",
    "THRUST_SMTP_PORT",
    "THRUST_SMTP_USERNAME",
    "THRUST_SMTP_PASSWORD",
    "THRUST_SMTP_CA_FILE",
    "THRUST_SMTP_TIMEOUT_SECS",
];

/// How the connection is protected.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SmtpTls {
    /// In the clear. For a relay on the same host or a sink in a test; never with a password.
    None,
    /// Plain first, then upgraded with `STARTTLS`, which is required: a relay that does not offer it is not used.
    StartTls,
    /// Encrypted from the first byte (SMTPS, port 465).
    Tls,
}

impl SmtpTls {
    /// The word a deployment writes in `THRUST_SMTP_TLS`.
    pub fn name(self) -> &'static str {
        match self {
            SmtpTls::None => "none",
            SmtpTls::StartTls => "starttls",
            SmtpTls::Tls => "tls",
        }
    }

    fn default_port(self) -> u16 {
        match self {
            SmtpTls::None => 25,
            SmtpTls::StartTls => 587,
            SmtpTls::Tls => 465,
        }
    }
}

/// What is wrong with the configuration, and which variable. Never carries a password.
#[derive(Debug, PartialEq, Eq)]
pub struct SmtpConfigError(String);

impl fmt::Display for SmtpConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for SmtpConfigError {}

fn bad(message: impl Into<String>) -> SmtpConfigError {
    SmtpConfigError(message.into())
}

/// The relay and how to talk to it. `Debug` hides the password.
#[derive(Clone)]
pub struct SmtpConfig {
    pub host: String,
    pub port: u16,
    pub tls: SmtpTls,
    /// A user name and its password, both or neither.
    pub credentials: Option<(String, String)>,
    pub from: Mailbox,
    /// A PEM file of the CA(s) the relay's certificate must chain to. Set: nothing else is trusted. Unset: the Mozilla roots.
    pub ca_file: Option<PathBuf>,
    /// The most one message may take, connecting included.
    pub timeout: Duration,
}

impl fmt::Debug for SmtpConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SmtpConfig")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("tls", &self.tls)
            .field("username", &self.credentials.as_ref().map(|(user, _)| user))
            .field("password", &self.credentials.as_ref().map(|_| "<redacted>"))
            .field("from", &self.from)
            .field("ca_file", &self.ca_file)
            .field("timeout", &self.timeout)
            .finish()
    }
}

impl SmtpConfig {
    /// The configuration `get` describes (the environment, in the server), or `None` when there is no host: no mail service. An
    /// empty value is an unset one.
    pub fn from_lookup(get: impl Fn(&str) -> Option<String>) -> Result<Option<SmtpConfig>, SmtpConfigError> {
        let get = |name: &str| get(name).filter(|v| !v.is_empty());
        let Some(host) = get(HOST) else {
            return match OTHERS.iter().find(|name| get(name).is_some()) {
                Some(name) => Err(bad(format!("{name} is set but {HOST} is not: set the host, or unset them all"))),
                None => Ok(None),
            };
        };
        if host.chars().any(|c| c.is_whitespace() || c == '/') {
            return Err(bad(format!("{HOST} is a host name or an address, without a scheme or a path")));
        }
        let tls = match get("THRUST_SMTP_TLS").map(|t| t.to_ascii_lowercase()).as_deref() {
            None | Some("starttls") => SmtpTls::StartTls,
            Some("tls") => SmtpTls::Tls,
            Some("none") => SmtpTls::None,
            Some(other) => return Err(bad(format!("THRUST_SMTP_TLS is starttls, tls or none, not '{other}'"))),
        };
        let port = match get("THRUST_SMTP_PORT") {
            None => tls.default_port(),
            Some(p) => match p.parse::<u16>() {
                Ok(n) if n != 0 => n,
                _ => return Err(bad(format!("THRUST_SMTP_PORT is a port number (1 to 65535), not '{p}'"))),
            },
        };
        let credentials = match (get("THRUST_SMTP_USERNAME"), get("THRUST_SMTP_PASSWORD")) {
            (Some(user), Some(password)) => Some((user, password)),
            (None, None) => None,
            _ => return Err(bad("THRUST_SMTP_USERNAME and THRUST_SMTP_PASSWORD are set together, or not at all")),
        };
        #[cfg(not(feature = "mutant_smtp_password_in_clear"))]
        if credentials.is_some() && tls == SmtpTls::None {
            return Err(bad("THRUST_SMTP_TLS=none would send the password in the clear: use starttls or tls, or drop the credentials"));
        }
        let Some(from) = get("THRUST_SMTP_FROM") else {
            return Err(bad("THRUST_SMTP_FROM is required with a host: the address the mail is sent from"));
        };
        let from = from
            .parse::<Mailbox>()
            .map_err(|_| bad("THRUST_SMTP_FROM is an address (no-reply@example.com) or a name and an address (Thrust <no-reply@example.com>)"))?;
        let timeout = match get("THRUST_SMTP_TIMEOUT_SECS") {
            None => Duration::from_secs(10),
            Some(t) => match t.parse::<u64>() {
                Ok(n) if (1..=300).contains(&n) => Duration::from_secs(n),
                _ => return Err(bad(format!("THRUST_SMTP_TIMEOUT_SECS is a number of seconds (1 to 300), not '{t}'"))),
            },
        };
        Ok(Some(SmtpConfig {
            host,
            port,
            tls,
            credentials,
            from,
            ca_file: get("THRUST_SMTP_CA_FILE").map(PathBuf::from),
            timeout,
        }))
    }
}

/// The relay as the port. Built once, when the server starts.
pub struct SmtpEmail {
    transport: AsyncSmtpTransport<Tokio1Executor>,
    from: Mailbox,
    /// The part of the `Message-ID` after the `@`: the sender's domain.
    domain: String,
    deadline: Duration,
}

impl fmt::Debug for SmtpEmail {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SmtpEmail").field("from", &self.from).field("deadline", &self.deadline).finish_non_exhaustive()
    }
}

impl SmtpEmail {
    /// Reads the CA file, if there is one, and builds the transport. No connection is made until a message is sent.
    pub fn new(cfg: &SmtpConfig) -> Result<Self, SmtpConfigError> {
        let domain = cfg.from.email.domain().to_string();
        let mut builder = AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(cfg.host.clone())
            .port(cfg.port)
            .timeout(Some(cfg.timeout))
            .hello_name(ClientId::Domain(domain.clone()));
        if cfg.tls != SmtpTls::None {
            let mut params = TlsParameters::builder(cfg.host.clone());
            match &cfg.ca_file {
                None => params = params.certificate_store(CertificateStore::WebpkiRoots),
                Some(path) => {
                    let pem = std::fs::read(path)
                        .map_err(|e| bad(format!("THRUST_SMTP_CA_FILE {} cannot be read: {e}", path.display())))?;
                    if !pem.windows(27).any(|w| w == b"-----BEGIN CERTIFICATE-----") {
                        return Err(bad(format!("THRUST_SMTP_CA_FILE {} holds no certificate (PEM)", path.display())));
                    }
                    let root = Certificate::from_pem(&pem).map_err(|e| {
                        bad(format!("THRUST_SMTP_CA_FILE {} is not a PEM certificate: {e}", path.display()))
                    })?;
                    params = params.certificate_store(CertificateStore::None).add_root_certificate(root);
                }
            }
            #[cfg(feature = "mutant_smtp_trust_any_certificate")]
            let params = params.dangerous_accept_invalid_certs(true).dangerous_accept_invalid_hostnames(true);
            let params = params.build().map_err(|e| bad(format!("the TLS settings for the relay: {e}")))?;
            builder = builder.tls(match cfg.tls {
                SmtpTls::Tls => Tls::Wrapper(params),
                // MUTANT (control run only): a relay that does not offer STARTTLS is used in the clear. The test that a relay that cannot do
                // TLS is not used must catch it.
                #[cfg(feature = "mutant_smtp_starttls_opportunistic")]
                _ => Tls::Opportunistic(params),
                #[cfg(not(feature = "mutant_smtp_starttls_opportunistic"))]
                _ => Tls::Required(params),
            });
        }
        if let Some((user, password)) = &cfg.credentials {
            builder = builder.credentials(Credentials::new(user.clone(), password.clone()));
        }
        Ok(SmtpEmail { transport: builder.build(), from: cfg.from.clone(), domain, deadline: cfg.timeout })
    }

    /// The message of the port's input, or why it cannot be one. The recipient is a bare address: `Name <a@b>`, `a@b, c@d` and an
    /// address with a line break in it are each refused here, so none of them can become a header or a second recipient.
    fn build(&self, message: &EmailMessage, id: &str) -> Result<Message, EmailError> {
        let to: Address =
            message.to.parse().map_err(|e| EmailError::Rejected(format!("the recipient is not an address: {e}")))?;
        #[cfg(feature = "mutant_smtp_from_is_recipient")]
        let from = Mailbox::new(None, to.clone());
        #[cfg(not(feature = "mutant_smtp_from_is_recipient"))]
        let from = self.from.clone();
        let builder = Message::builder()
            .from(from)
            .to(Mailbox::new(None, to))
            .subject(message.subject.clone())
            .message_id(Some(id.to_string()));
        let made = match &message.html {
            None => builder.singlepart(SinglePart::plain(message.text.clone())),
            Some(html) => builder.multipart(MultiPart::alternative_plain_html(message.text.clone(), html.clone())),
        };
        made.map_err(|e| EmailError::Rejected(format!("the message cannot be made: {e}")))
    }
}

/// A relay's refusal or an outage, from what `lettre` says. Permanent (5xx) replies and the client's own errors (a relay that offers no
/// way to log in with what is set) would come back the same on the next try; the rest (4xx, no connection, no answer, a failed
/// handshake) may not.
fn classify(e: &lettre::transport::smtp::Error) -> EmailError {
    let mut text = e.to_string();
    let mut source = std::error::Error::source(e);
    while let Some(s) = source {
        // `lettre` prints some of its sources itself: "Connection refused" twice is no more informative than once
        let said = s.to_string();
        if !text.contains(&said) {
            text.push_str(": ");
            text.push_str(&said);
        }
        source = s.source();
    }
    let refusal = e.is_permanent() || e.is_client();
    #[cfg(feature = "mutant_smtp_swap_refusal_and_outage")]
    let refusal = e.is_transient() || e.is_client();
    if refusal {
        EmailError::Rejected(text)
    } else {
        EmailError::Unavailable(text)
    }
}

#[async_trait]
impl EmailSender for SmtpEmail {
    async fn send(&self, message: &EmailMessage) -> Result<String, EmailError> {
        let id = format!("<{}@{}>", uuid::Uuid::now_v7().simple(), self.domain);
        let built = self.build(message, &id)?;
        match tokio::time::timeout(self.deadline, self.transport.send(built)).await {
            Err(_) => {
                Err(EmailError::Unavailable(format!("the relay took longer than {} s", self.deadline.as_secs_f64())))
            }
            Ok(Err(e)) => Err(classify(&e)),
            #[cfg(feature = "mutant_smtp_wrong_message_id")]
            Ok(Ok(_)) => Ok(format!("<{}@{}>", uuid::Uuid::now_v7().simple(), self.domain)),
            #[cfg(not(feature = "mutant_smtp_wrong_message_id"))]
            Ok(Ok(_)) => Ok(id),
        }
    }
}
