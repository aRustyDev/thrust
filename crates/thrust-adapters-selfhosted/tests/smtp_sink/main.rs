//! The SMTP adapter against Mailpit (a mail server and sink written by someone else, run by testcontainers: Docker, so this binary is out of the
//! hermetic loop, as `pg_adapter` is). What `smtp_wire`'s scripted server cannot say, because it is the same author's idea of SMTP: that a server
//! that parses messages for itself reads what the adapter sends the way the adapter was told it, in plain, over STARTTLS and over implicit TLS, with
//! a login, and that the server's own refusals (Mailpit's "chaos" triggers answer with the codes it chooses) are read as refusals and outages.
//!
//! The TLS tests are the ones with the most to prove, so each has its controls in the same container: the server REQUIRES TLS (a plain client is
//! refused by the server itself), so a message that arrived over TLS did; and a certificate from another CA, one that does not name the host, and the
//! Mozilla roots alone are each refused, so the success was the verifier saying yes and not nothing.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "test code: a panic is how a test fails, and clippy.toml exempts only #[test] functions and #[cfg(test)] modules, not the helpers around them"
)]

use std::net::IpAddr;
use std::path::PathBuf;
use std::time::Duration;

use rcgen::{BasicConstraints, CertificateParams, CertifiedIssuer, DnType, IsCa, KeyPair, KeyUsagePurpose, SanType};
use testcontainers::core::{ContainerPort, WaitFor};
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, GenericImage, ImageExt};
use thrust_adapters_selfhosted::smtp::{SmtpConfig, SmtpEmail, SmtpTls};
use thrust_ports::{EmailError, EmailMessage, EmailSender};

/// Pinned, so a new Mailpit cannot change what these tests mean. 1.33.1 is what they were written against (2026-11-14).
const MAILPIT: &str = "v1.33.1";

struct Mailpit {
    _container: ContainerAsync<GenericImage>,
    smtp: u16,
    http: u16,
}

/// A Mailpit with `env` set, and `files` (path in the container, contents) put in it before it starts.
async fn mailpit(env: &[(&str, &str)], files: &[(&str, Vec<u8>)]) -> Mailpit {
    let image = GenericImage::new("axllent/mailpit", MAILPIT)
        .with_exposed_port(ContainerPort::Tcp(1025))
        .with_exposed_port(ContainerPort::Tcp(8025))
        .with_wait_for(WaitFor::message_on_stdout("accessible via"));
    let mut request: testcontainers::ContainerRequest<GenericImage> = image.into();
    for (k, v) in env {
        request = request.with_env_var(*k, *v);
    }
    for (path, bytes) in files {
        request = request.with_copy_to(*path, bytes.clone());
    }
    let container = request.start().await.expect("start the mailpit container");
    let smtp = container.get_host_port_ipv4(1025).await.expect("mapped smtp port");
    let http = container.get_host_port_ipv4(8025).await.expect("mapped http port");
    let mp = Mailpit { _container: container, smtp, http };
    mp.ready().await;
    mp
}

impl Mailpit {
    /// The container's log says it started, and the ports it publishes are not always taking connections yet (seen: "connection refused" on a port
    /// a second after its container reported ready, when several started at once). Ready is what a client can see: the SMTP port says `220`
    /// (and for a server that speaks TLS first, accepts the connection) and the API answers.
    async fn ready(&self) {
        use tokio::io::AsyncReadExt;
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        loop {
            let smtp = async {
                let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", self.smtp)).await.ok()?;
                // a TLS-first server says nothing until the client does: connected is as ready as it gets, so a short wait for a banner
                let mut buf = [0u8; 4];
                match tokio::time::timeout(Duration::from_millis(300), stream.read_exact(&mut buf)).await {
                    Ok(Ok(_)) => (&buf == b"220 " || &buf == b"220-").then_some(()),
                    Ok(Err(_)) => None,
                    Err(_) => Some(()),
                }
            };
            let api = reqwest::get(format!("http://127.0.0.1:{}/api/v1/info", self.http))
                .await
                .is_ok_and(|r| r.status().is_success());
            if api && smtp.await.is_some() {
                return;
            }
            assert!(std::time::Instant::now() < deadline, "mailpit's ports never took a connection");
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    async fn get(&self, path: &str) -> serde_json::Value {
        let body = reqwest::get(format!("http://127.0.0.1:{}{path}", self.http))
            .await
            .expect("mailpit's API answers")
            .bytes()
            .await
            .expect("its body");
        serde_json::from_slice(&body).expect("its JSON")
    }

    /// Every message the server holds, in full (the list gives only a summary).
    async fn inbox(&self) -> Vec<serde_json::Value> {
        let list = self.get("/api/v1/messages").await;
        let mut all = Vec::new();
        for m in list["messages"].as_array().unwrap() {
            all.push(self.get(&format!("/api/v1/message/{}", m["ID"].as_str().unwrap())).await);
        }
        all
    }
}

fn config(m: &Mailpit, tls: SmtpTls) -> SmtpConfig {
    SmtpConfig {
        host: "127.0.0.1".into(),
        port: m.smtp,
        tls,
        credentials: None,
        from: "Thrust <no-reply@example.com>".parse().unwrap(),
        ca_file: None,
        timeout: Duration::from_secs(15),
    }
}

fn message(subject: &str) -> EmailMessage {
    EmailMessage {
        to: "alice@example.org".into(),
        subject: subject.into(),
        text: "Follow the link.\n".into(),
        html: None,
    }
}

/// Line endings as a reader sees them: the wire has CRLF and the test writes LF.
fn lf(s: &str) -> String {
    s.replace("\r\n", "\n").trim_end().to_string()
}

#[tokio::test]
async fn a_message_is_read_by_the_server_as_it_was_given() {
    let mp = mailpit(&[], &[]).await;
    let email = SmtpEmail::new(&config(&mp, SmtpTls::None)).unwrap();
    let text = "Réinitialisez votre mot de passe ✉\nLine two\n.\nAfter a dot line\n..two dots\n";
    let html = "<p>Réinitialisez <a href=\"https://projects.example.com/x?a=1&amp;b=2\">le lien</a> ✉</p>\n";
    let given = EmailMessage {
        to: "alice@example.org".into(),
        subject: "Réinitialisez votre mot de passe ✉ \"Thrust\"".into(),
        text: text.into(),
        html: Some(html.into()),
    };
    let id = email.send(&given).await.expect("mailpit takes it");

    let inbox = mp.inbox().await;
    assert_eq!(inbox.len(), 1, "one message sent, one held");
    let held = &inbox[0];
    assert_eq!(
        format!("<{}>", held["MessageID"].as_str().unwrap()),
        id,
        "the id the adapter returned is the Message-ID the server read"
    );
    assert_eq!(held["From"]["Address"], "no-reply@example.com");
    assert_eq!(held["From"]["Name"], "Thrust");
    assert_eq!(held["To"][0]["Address"], "alice@example.org");
    assert_eq!(held["To"].as_array().unwrap().len(), 1);
    assert_eq!(held["Subject"], given.subject.as_str());
    assert_eq!(lf(held["Text"].as_str().unwrap()), lf(text));
    assert_eq!(lf(held["HTML"].as_str().unwrap()), lf(html));

    // and a message with no HTML part has none
    email.send(&message("text only")).await.unwrap();
    let inbox = mp.inbox().await;
    let plain = inbox.iter().find(|m| m["Subject"] == "text only").expect("the second message");
    assert_eq!(plain["HTML"], "");
}

/// Mailpit's chaos triggers make IT answer with a code at a step. The adapter reads Mailpit's 4xx as an outage and its 5xx as a refusal, at the
/// sender and at the recipient: the two readings, from a server whose codes nobody here chose.
#[tokio::test]
async fn the_servers_own_refusals_and_outages_are_read_as_such() {
    for (trigger, outage) in
        [("Sender:451:100", true), ("Sender:550:100", false), ("Recipient:452:100", true), ("Recipient:554:100", false)]
    {
        let mp = mailpit(&[("MP_CHAOS_TRIGGERS", trigger)], &[]).await;
        let got = SmtpEmail::new(&config(&mp, SmtpTls::None)).unwrap().send(&message("hello")).await;
        match (outage, &got) {
            (true, Err(EmailError::Unavailable(_))) | (false, Err(EmailError::Rejected(_))) => {}
            _ => panic!("{trigger}: read the wrong way: {got:?}"),
        }
        assert!(mp.inbox().await.is_empty(), "{trigger}: a refused message was kept");
    }
}

/// A server that will send only to some addresses refuses the others for good, and keeps nothing of them.
#[tokio::test]
async fn an_address_the_server_will_not_send_to_is_a_refusal() {
    let mp = mailpit(&[("MP_SMTP_ALLOWED_RECIPIENTS", "^[a-z]+@example\\.org$")], &[]).await;
    let email = SmtpEmail::new(&config(&mp, SmtpTls::None)).unwrap();
    email.send(&message("to an allowed address")).await.expect("alice@example.org is allowed");
    let mut elsewhere = message("to another");
    elsewhere.to = "bob@example.net".into();
    let got = email.send(&elsewhere).await;
    assert!(matches!(got, Err(EmailError::Rejected(_))), "{got:?}");
    let inbox = mp.inbox().await;
    assert_eq!(inbox.len(), 1);
    assert_eq!(inbox[0]["Subject"], "to an allowed address");
}

// ---- TLS ----

fn ca() -> CertifiedIssuer<'static, KeyPair> {
    let mut p = CertificateParams::new(Vec::<String>::new()).unwrap();
    p.distinguished_name.push(DnType::CommonName, "thrust smtp test CA");
    p.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    p.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    CertifiedIssuer::self_signed(p, KeyPair::generate().unwrap()).unwrap()
}

/// A server certificate signed by `issuer`, with the given names (an address is an IP name, anything else a DNS one), as PEM; and its key.
fn server_cert(issuer: &CertifiedIssuer<'static, KeyPair>, names: &[&str]) -> (String, String) {
    let mut p = CertificateParams::new(Vec::<String>::new()).unwrap();
    p.subject_alt_names = names
        .iter()
        .map(|n| match n.parse::<IpAddr>() {
            Ok(ip) => SanType::IpAddress(ip),
            Err(_) => SanType::DnsName((*n).try_into().unwrap()),
        })
        .collect();
    let key = KeyPair::generate().unwrap();
    let cert = p.signed_by(&key, issuer).unwrap();
    (cert.pem(), key.serialize_pem())
}

/// A directory of the run's own, removed when the test is over, for the CA files the adapter is pointed at.
struct Dir(PathBuf);

impl Dir {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!("thrust-smtp-sink-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir_all(&dir).unwrap();
        Dir(dir)
    }
    fn file(&self, name: &str, body: &str) -> PathBuf {
        let path = self.0.join(name);
        std::fs::write(&path, body).unwrap();
        path
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

const TLS_FILES: [&str; 2] = ["/tls/server.crt", "/tls/server.key"];

fn tls_server_env<'a>(extra: &[(&'a str, &'a str)]) -> Vec<(&'a str, &'a str)> {
    let mut env = vec![("MP_SMTP_TLS_CERT", TLS_FILES[0]), ("MP_SMTP_TLS_KEY", TLS_FILES[1])];
    env.extend_from_slice(extra);
    env
}

/// STARTTLS. The server requires it, so a plain client is refused by the server and a message that arrived did so encrypted; and the certificate is
/// checked: the right CA is trusted, another CA is not, a certificate that does not name the host is not, and with no CA named the Mozilla roots do
/// not know this one.
#[tokio::test]
async fn starttls_is_required_and_the_relays_certificate_is_checked() {
    let (good, other) = (ca(), ca());
    let (cert, key) = server_cert(&good, &["127.0.0.1"]);
    let mp = mailpit(
        &tls_server_env(&[("MP_SMTP_REQUIRE_STARTTLS", "true")]),
        &[(TLS_FILES[0], cert.into_bytes()), (TLS_FILES[1], key.into_bytes())],
    )
    .await;
    let dir = Dir::new();
    let trusting = |pem: String, name: &str| {
        let mut cfg = config(&mp, SmtpTls::StartTls);
        cfg.ca_file = Some(dir.file(name, &pem));
        SmtpEmail::new(&cfg).unwrap()
    };

    // CONTROL: the server requires TLS; a client that does not use it is refused, by the server, for good
    let plain = SmtpEmail::new(&config(&mp, SmtpTls::None)).unwrap().send(&message("in the clear")).await;
    assert!(
        matches!(plain, Err(EmailError::Rejected(_))),
        "a server that requires STARTTLS must refuse a plain client: {plain:?}"
    );
    assert!(mp.inbox().await.is_empty());

    // the right CA: sent, and held
    trusting(good.pem(), "good.pem").send(&message("over starttls")).await.expect("the relay's CA is trusted");
    let inbox = mp.inbox().await;
    assert_eq!(inbox.len(), 1);
    assert_eq!(inbox[0]["Subject"], "over starttls");

    // CONTROL: another CA. Refused as an outage of the handshake, and nothing is kept
    let got = trusting(other.pem(), "other.pem").send(&message("wrong CA")).await;
    assert!(
        matches!(got, Err(EmailError::Unavailable(ref e)) if e.to_lowercase().contains("certificate") || e.contains("tls")),
        "UNTRUSTED CERTIFICATE ACCEPTED (another CA): {got:?}"
    );
    // CONTROL: no CA named, so the Mozilla roots decide, and they do not know a test CA
    let got = SmtpEmail::new(&config(&mp, SmtpTls::StartTls)).unwrap().send(&message("no CA")).await;
    assert!(
        matches!(got, Err(EmailError::Unavailable(_))),
        "UNTRUSTED CERTIFICATE ACCEPTED (no CA named, so the Mozilla roots): {got:?}"
    );
    assert_eq!(mp.inbox().await.len(), 1, "a message was kept that a refused handshake should not have carried");
}

/// The name in the certificate must be the host's: a certificate from the trusted CA that names another host is refused, so the trust in a CA is
/// not a trust in every certificate it ever signed.
#[tokio::test]
async fn a_certificate_that_does_not_name_the_host_is_refused_even_from_the_trusted_ca() {
    let issuer = ca();
    let (cert, key) = server_cert(&issuer, &["mail.elsewhere.example"]);
    let mp = mailpit(
        &tls_server_env(&[("MP_SMTP_REQUIRE_STARTTLS", "true")]),
        &[(TLS_FILES[0], cert.into_bytes()), (TLS_FILES[1], key.into_bytes())],
    )
    .await;
    let dir = Dir::new();
    let mut cfg = config(&mp, SmtpTls::StartTls);
    cfg.ca_file = Some(dir.file("ca.pem", &issuer.pem()));
    let got = SmtpEmail::new(&cfg).unwrap().send(&message("wrong name")).await;
    assert!(
        matches!(got, Err(EmailError::Unavailable(_))),
        "UNTRUSTED CERTIFICATE ACCEPTED (a name that is not the host's): {got:?}"
    );
    assert!(mp.inbox().await.is_empty());

    // CONTROL: the same server and CA, reached by a name its certificate has: it goes through. The refusal above was the name.
    let (cert, key) = server_cert(&issuer, &["127.0.0.1"]);
    let mp = mailpit(
        &tls_server_env(&[("MP_SMTP_REQUIRE_STARTTLS", "true")]),
        &[(TLS_FILES[0], cert.into_bytes()), (TLS_FILES[1], key.into_bytes())],
    )
    .await;
    let mut cfg = config(&mp, SmtpTls::StartTls);
    cfg.ca_file = Some(dir.file("ca2.pem", &issuer.pem()));
    SmtpEmail::new(&cfg).unwrap().send(&message("right name")).await.expect("a certificate that names the host");
}

/// Implicit TLS: the connection is encrypted from its first byte (SMTPS), and the same checks hold.
#[tokio::test]
async fn implicit_tls_reaches_a_server_that_speaks_tls_first() {
    let (good, other) = (ca(), ca());
    let (cert, key) = server_cert(&good, &["127.0.0.1"]);
    let mp = mailpit(
        &tls_server_env(&[("MP_SMTP_REQUIRE_TLS", "true")]),
        &[(TLS_FILES[0], cert.into_bytes()), (TLS_FILES[1], key.into_bytes())],
    )
    .await;
    let dir = Dir::new();
    let with = |pem: String, name: &str| {
        let mut cfg = config(&mp, SmtpTls::Tls);
        cfg.ca_file = Some(dir.file(name, &pem));
        SmtpEmail::new(&cfg).unwrap()
    };
    with(good.pem(), "good.pem").send(&message("over smtps")).await.expect("the relay's CA is trusted");
    let inbox = mp.inbox().await;
    assert_eq!(inbox.len(), 1);
    assert_eq!(inbox[0]["Subject"], "over smtps");
    // CONTROL: another CA is refused here too
    let got = with(other.pem(), "other.pem").send(&message("wrong CA")).await;
    assert!(
        matches!(got, Err(EmailError::Unavailable(_))),
        "UNTRUSTED CERTIFICATE ACCEPTED (another CA, over implicit TLS): {got:?}"
    );
    // CONTROL: a client that speaks plain SMTP first to a server that speaks TLS first gets no message through
    let got = SmtpEmail::new(&config(&mp, SmtpTls::None)).unwrap().send(&message("plain to smtps")).await;
    assert!(got.is_err(), "{got:?}");
    assert_eq!(mp.inbox().await.len(), 1);
}

/// A login. The server wants one (over STARTTLS, as Mailpit insists when the connection is not encrypted); the right password sends, a wrong one
/// is a refusal for good, and no login at all is refused too, which shows the server really asks. The password is made for the run.
#[tokio::test]
async fn a_login_is_made_with_the_credentials_and_a_wrong_one_is_a_refusal() {
    let issuer = ca();
    let (cert, key) = server_cert(&issuer, &["127.0.0.1"]);
    let password = uuid::Uuid::now_v7().simple().to_string();
    let auth = format!("mailer:{password}");
    let mp = mailpit(
        &tls_server_env(&[("MP_SMTP_REQUIRE_STARTTLS", "true"), ("MP_SMTP_AUTH", auth.as_str())]),
        &[(TLS_FILES[0], cert.into_bytes()), (TLS_FILES[1], key.into_bytes())],
    )
    .await;
    let dir = Dir::new();
    let ca_path = dir.file("ca.pem", &issuer.pem());
    let with = |credentials: Option<(&str, &str)>| {
        let mut cfg = config(&mp, SmtpTls::StartTls);
        cfg.ca_file = Some(ca_path.clone());
        cfg.credentials = credentials.map(|(u, p)| (u.to_string(), p.to_string()));
        SmtpEmail::new(&cfg).unwrap()
    };

    // CONTROL: no login. The server asks for one, so a message that arrives below did so because the login was made
    let got = with(None).send(&message("no login")).await;
    assert!(matches!(got, Err(EmailError::Rejected(_))), "a server that wants a login must refuse none: {got:?}");
    assert!(mp.inbox().await.is_empty());

    let got = with(Some(("mailer", "not the password"))).send(&message("wrong password")).await;
    assert!(
        matches!(got, Err(EmailError::Rejected(ref e)) if !e.contains(&password)),
        "a wrong password is a refusal: {got:?}"
    );
    let got = with(Some(("somebody-else", password.as_str()))).send(&message("wrong user")).await;
    assert!(matches!(got, Err(EmailError::Rejected(_))), "{got:?}");
    assert!(mp.inbox().await.is_empty());

    with(Some(("mailer", password.as_str()))).send(&message("logged in")).await.expect("the right credentials");
    let inbox = mp.inbox().await;
    assert_eq!(inbox.len(), 1);
    assert_eq!(inbox[0]["Subject"], "logged in");
    assert_eq!(inbox[0]["Username"], "mailer", "the server says who it thinks sent it");
}
