//! The TLS side of `migrate` (issue #49): the settings it reads, the handshake `verify-ca` makes, and the two ways it must not
//! quietly drop to plaintext. No Docker: the handshakes run in memory between a real rustls server and the client under test, over
//! `tokio::io::duplex`, with certificates made for each run (a wrong CA and an expired leaf are one call each). The same client
//! against a real Postgres is `tls` in `thrust-adapters-selfhosted/tests/pg_adapter.rs`.
//!
//! What these tests are for. `thrust-pg` accepts only TLS (`hostnossl ... reject`), and it is reached through a tunnel at
//! `127.0.0.1`, while its certificate names a hostname. So `verify-ca` has to check the chain, the signature and the dates and
//! NOT the name, and it has to be the only thing that is skipped: a verifier that accepted anything would pass every case that
//! should succeed and the whole suite would still look green. The cases that must fail are therefore the evidence.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "test code: a panic is how a test fails, and clippy.toml exempts only #[test] functions and #[cfg(test)] modules, not the helpers around them"
)]

mod common;

use std::fmt;
use std::sync::Arc;

use common::proptest_config;
use proptest::prelude::*;
use rcgen::{
    date_time_ymd, BasicConstraints, CertificateParams, CertifiedIssuer, DnType, IsCa, KeyPair, KeyUsagePurpose,
};
use rstest::rstest;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use thrust_adapters_postgres::tls::{connect, error_chain, ConnectError, Tls, TlsConfigError};
use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream};
use tokio::net::TcpListener;
use tokio_postgres::tls::{MakeTlsConnect, TlsConnect};
use tokio_rustls::TlsAcceptor;

// ---- the settings: DATABASE_SSLMODE and DATABASE_SSLROOTCERT -----------------------------------------------------------------

fn ca_pem(name: &str) -> String {
    ca(name).pem()
}

fn ca(name: &str) -> CertifiedIssuer<'static, KeyPair> {
    let mut p = CertificateParams::new(Vec::<String>::new()).unwrap();
    p.distinguished_name.push(DnType::CommonName, name);
    p.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    p.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    CertifiedIssuer::self_signed(p, KeyPair::generate().unwrap()).unwrap()
}

/// What a root-certificate file can hold. The first four are each a different mistake someone makes with a file path.
#[derive(Debug, Clone, Copy)]
enum RootFile {
    /// DATABASE_SSLROOTCERT is not set
    NotSet,
    /// one good certificate
    Good,
    /// a file that is there and holds nothing a CA bundle would: empty, or some other text
    NoCertificate,
    /// a block that decodes as PEM and is not a certificate (it fails when it is parsed, not when it is decoded)
    BadCertificate,
    /// a block that says CERTIFICATE and is not PEM at all (it fails when it is decoded: the other branch)
    BadPem,
    /// a good certificate and a bad one of each kind: refused whole, a bundle is not trusted in part
    GoodAndBad,
    GoodAndBadPem,
}

/// Valid base64 of three zero bytes: decodes, and is not a certificate.
const BAD_BLOCK: &str = "-----BEGIN CERTIFICATE-----\nAAAA\n-----END CERTIFICATE-----\n";
/// Not base64.
const BAD_PEM: &str = "-----BEGIN CERTIFICATE-----\n!!!not base64!!!\n-----END CERTIFICATE-----\n";

fn root_file(kind: RootFile) -> Option<Vec<u8>> {
    match kind {
        RootFile::NotSet => None,
        RootFile::Good => Some(ca_pem("good").into_bytes()),
        RootFile::NoCertificate => Some(b"not a certificate, just a note\n".to_vec()),
        RootFile::BadCertificate => Some(BAD_BLOCK.as_bytes().to_vec()),
        RootFile::BadPem => Some(BAD_PEM.as_bytes().to_vec()),
        RootFile::GoodAndBad => Some(format!("{}{BAD_BLOCK}", ca_pem("good")).into_bytes()),
        RootFile::GoodAndBadPem => Some(format!("{}{BAD_PEM}", ca_pem("good")).into_bytes()),
    }
}

/// The spec, written as a table and not as the code under test: only `disable` and `verify-ca` exist; a root file means nothing
/// without `verify-ca`, and `verify-ca` means nothing without a root file that holds certificates and only certificates.
/// Everything else, `require` and `verify-full` included, is refused and not mapped to something near it, so a setting somebody
/// believes is stricter than it is can never be accepted.
fn spec(mode: Option<&str>, file: RootFile) -> Result<&'static str, TlsConfigError> {
    use RootFile::*;
    match (mode, file) {
        (None | Some("disable"), NotSet) => Ok("disable"),
        (None | Some("disable"), _) => Err(TlsConfigError::RootCertWithoutVerify),
        (Some("verify-ca"), NotSet) => Err(TlsConfigError::RootCertMissing),
        (Some("verify-ca"), Good) => Ok("verify-ca"),
        (Some("verify-ca"), NoCertificate) => Err(TlsConfigError::RootCertEmpty),
        (Some("verify-ca"), BadCertificate | BadPem | GoodAndBad | GoodAndBadPem) => {
            Err(TlsConfigError::RootCertInvalid)
        }
        (Some(other), _) => Err(TlsConfigError::UnsupportedMode(other.to_string())),
    }
}

// Exhaustive over the classes: 8 modes x 7 root files.
#[rstest]
fn the_settings_follow_the_table(
    #[values(
        None,
        Some(""),
        Some("disable"),
        Some("verify-ca"),
        Some("require"),
        Some("verify-full"),
        Some("VERIFY-CA"),
        Some("prefer")
    )]
    mode: Option<&str>,
    #[values(
        RootFile::NotSet,
        RootFile::Good,
        RootFile::NoCertificate,
        RootFile::BadCertificate,
        RootFile::BadPem,
        RootFile::GoodAndBad,
        RootFile::GoodAndBadPem
    )]
    file: RootFile,
) {
    let bytes = root_file(file);
    let got = Tls::from_settings(mode, bytes.as_deref()).map(|t| t.mode());
    assert_eq!(got, spec(mode, file), "mode {mode:?}, root file {file:?}");
}

#[test]
fn every_error_names_the_setting_to_change() {
    // pinned: what an operator reads at 2 a.m. has to say which variable, not "invalid configuration"
    for (err, needle) in [
        (TlsConfigError::UnsupportedMode("require".into()), "DATABASE_SSLMODE"),
        (TlsConfigError::UnsupportedMode("require".into()), "'require'"),
        (TlsConfigError::RootCertMissing, "DATABASE_SSLROOTCERT"),
        (TlsConfigError::RootCertWithoutVerify, "DATABASE_SSLROOTCERT"),
        (TlsConfigError::RootCertEmpty, "DATABASE_SSLROOTCERT"),
        (TlsConfigError::RootCertInvalid, "DATABASE_SSLROOTCERT"),
    ] {
        assert!(err.to_string().contains(needle), "{err:?} says '{err}', which lacks '{needle}'");
    }
}

proptest! {
    #![proptest_config(proptest_config(256))]

    /// Whatever the setting says, nothing but `disable` and `verify-ca` is accepted: no spelling, case or padding turns another
    /// mode into one of them. Random text alone would almost never be `require`, so the generator is weighted toward the words
    /// somebody would actually type, and toward the two accepted ones with something added or changed.
    #[test]
    fn prop_every_other_mode_is_refused(
        mode in prop_oneof![
            1 => ".{0,24}",
            4 => prop::sample::select(vec![
                "require", "verify-full", "prefer", "allow", "verify_ca", "verifyca", "Disable", "DISABLE", "Verify-CA", "VERIFY-CA",
                " disable", "disable ", " verify-ca", "verify-ca ", "disable\n", "verify-ca\0", "disabled", "verify-ca-full",
            ]).prop_map(String::from),
        ]
    ) {
        prop_assume!(mode != "disable" && mode != "verify-ca");
        let good = ca_pem("good");
        for root in [None, Some(good.as_bytes())] {
            let got = Tls::from_settings(Some(&mode), root);
            prop_assert_eq!(got.map(|t| t.mode()), Err(TlsConfigError::UnsupportedMode(mode.clone())));
        }
    }
}

// ---- the handshake: what `verify-ca` accepts and refuses ---------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq)]
enum Trust {
    /// the CA that signed the server's certificate
    Right,
    /// an unrelated CA
    Wrong,
    /// a bundle: an unrelated CA first, the right one second
    BundleRightSecond,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Validity {
    Valid,
    Expired,
    NotYetValid,
}

/// The name the server's certificate carries, and the host the client is told it is connecting to.
const CERT_NAME: &str = "thrust-pg.example.test";

fn leaf(
    issuer: &CertifiedIssuer<'static, KeyPair>,
    name: &str,
    validity: Validity,
) -> (CertificateDer<'static>, PrivateKeyDer<'static>) {
    let mut p = CertificateParams::new(vec![name.to_string()]).unwrap();
    match validity {
        Validity::Valid => {}
        Validity::Expired => {
            p.not_before = date_time_ymd(2000, 1, 1);
            p.not_after = date_time_ymd(2001, 1, 1);
        }
        Validity::NotYetValid => {
            p.not_before = date_time_ymd(2900, 1, 1);
            p.not_after = date_time_ymd(2901, 1, 1);
        }
    }
    let key = KeyPair::generate().unwrap();
    let cert = p.signed_by(&key, issuer).unwrap();
    (cert.der().clone(), PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key.serialize_der())))
}

fn provider() -> Arc<rustls::crypto::CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

fn acceptor(cert: CertificateDer<'static>, key: PrivateKeyDer<'static>) -> TlsAcceptor {
    let cfg = rustls::ServerConfig::builder_with_provider(provider())
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(vec![cert], key)
        .unwrap();
    TlsAcceptor::from(Arc::new(cfg))
}

/// One handshake and one round trip over it, as the client under test sees them. `Err` is the client's own error, in Debug form so
/// that the rustls variant that caused it (`UnknownIssuer`, `Expired`, ...) is in the text.
async fn handshake(trust: Trust, validity: Validity, host: &str, server_name: &str) -> Result<(), String> {
    let right = ca("right");
    let wrong = ca("wrong");
    let trusted = match trust {
        Trust::Right => right.pem(),
        Trust::Wrong => wrong.pem(),
        Trust::BundleRightSecond => format!("{}{}", wrong.pem(), right.pem()),
    };
    let (cert, key) = leaf(&right, server_name, validity);
    let tls = Tls::from_settings(Some("verify-ca"), Some(trusted.as_bytes())).unwrap();
    let mut make = tls.connector().expect("verify-ca has a connector");

    let (client_io, server_io) = tokio::io::duplex(16 * 1024);
    let acceptor = acceptor(cert, key);
    let server = tokio::spawn(async move {
        if let Ok(mut s) = acceptor.accept(server_io).await {
            let mut buf = [0u8; 4];
            if s.read_exact(&mut buf).await.is_ok() {
                let _ = s.write_all(&buf).await;
                let _ = s.flush().await;
            }
        }
    });

    let connect = MakeTlsConnect::<DuplexStream>::make_tls_connect(&mut make, host).map_err(|e| format!("{e:?}"))?;
    let mut stream = connect.connect(client_io).await.map_err(|e| format!("{e:?}"))?;
    stream.write_all(b"ping").await.map_err(|e| format!("{e:?}"))?;
    stream.flush().await.map_err(|e| format!("{e:?}"))?;
    let mut echo = [0u8; 4];
    stream.read_exact(&mut echo).await.map_err(|e| format!("{e:?}"))?;
    drop(stream);
    let _ = server.await;
    assert_eq!(&echo, b"ping", "the stream the client got back must carry data both ways");
    Ok(())
}

/// What a refusal says, for the cases with exactly one fault. A rustls version names the same fault in more than one way
/// (`Expired` or `ExpiredContext`, shown as "expired"), so each is a set of spellings of one fault.
fn fault_words(trust: Trust, validity: Validity) -> Option<&'static [&'static str]> {
    match (trust, validity) {
        (Trust::Wrong, Validity::Valid) => Some(&["UnknownIssuer"]),
        (Trust::Right | Trust::BundleRightSecond, Validity::Expired) => Some(&["Expired", "expired"]),
        (Trust::Right | Trust::BundleRightSecond, Validity::NotYetValid) => Some(&["NotValidYet", "not valid yet"]),
        _ => None,
    }
}

// Exhaustive: 3 trust variants x 3 validities x 2 names. The spec is one line, and it is the point: a certificate is accepted when
// its chain reaches a CA the client was given and its dates hold, WHATEVER name it carries, because the client reaches the
// database at 127.0.0.1 through a tunnel and the certificate names a host.
#[rstest]
#[tokio::test]
async fn verify_ca_accepts_what_chains_to_the_given_ca_and_is_in_date_whatever_its_name(
    #[values(Trust::Right, Trust::Wrong, Trust::BundleRightSecond)] trust: Trust,
    #[values(Validity::Valid, Validity::Expired, Validity::NotYetValid)] validity: Validity,
    #[values("127.0.0.1", CERT_NAME)] host: &str,
) {
    let got = handshake(trust, validity, host, CERT_NAME).await;
    let accepted = trust != Trust::Wrong && validity == Validity::Valid;
    match (accepted, &got) {
        (true, Ok(())) => {}
        (true, Err(e)) => panic!("{trust:?} / {validity:?} / host {host}: must be accepted, got {e}"),
        (false, Ok(())) => panic!("{trust:?} / {validity:?} / host {host}: must be refused, and it was accepted"),
        (false, Err(e)) => {
            if let Some(words) = fault_words(trust, validity) {
                assert!(
                    words.iter().any(|w| e.contains(w)),
                    "{trust:?} / {validity:?}: refused for another reason than {words:?}: {e}"
                );
            }
        }
    }
}

#[tokio::test]
async fn verify_ca_still_refuses_a_name_that_does_not_match_when_the_chain_is_also_wrong() {
    // pinned: the name is the one thing it skips. A wrong CA with a wrong name is refused for the CA, which is checked first.
    let got = handshake(Trust::Wrong, Validity::Valid, "127.0.0.1", "some-other-host.example.test").await;
    assert!(got.as_ref().is_err_and(|e| e.contains("UnknownIssuer")), "{got:?}");
}

#[tokio::test]
async fn a_peer_that_does_not_speak_tls_is_refused_and_does_not_hang() {
    let tls = Tls::from_settings(Some("verify-ca"), Some(ca_pem("any").as_bytes())).unwrap();
    let mut make = tls.connector().unwrap();
    let (client_io, mut server_io) = tokio::io::duplex(1024);
    // what a Postgres that said 'S' and then was not a TLS server would send
    let peer = tokio::spawn(async move {
        let _ = server_io.write_all(b"this is not a TLS record at all\n").await;
        let _ = server_io.shutdown().await;
    });
    let connect = MakeTlsConnect::<DuplexStream>::make_tls_connect(&mut make, "127.0.0.1").unwrap();
    let got = connect.connect(client_io).await;
    let _ = peer.await;
    assert!(got.is_err(), "a stream that is not TLS must not become one");
}

// ---- no silent plaintext ----------------------------------------------------------------------------------------------------

/// The SSLRequest every TLS-capable Postgres client sends first: length 8, code 80877103.
const SSL_REQUEST: [u8; 8] = [0, 0, 0, 8, 4, 210, 22, 47];

/// A server that takes one connection, answers an SSLRequest with `answer`, hangs up its sending side, and returns every byte the
/// client sent. It never speaks the protocol, so a client that goes on in plaintext shows up as bytes after the SSLRequest.
async fn spy(answer: u8) -> (u16, tokio::task::JoinHandle<Vec<u8>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let handle = tokio::spawn(async move {
        let (mut s, _) = listener.accept().await.unwrap();
        let mut got = vec![0u8; 8];
        s.read_exact(&mut got).await.unwrap();
        if got == SSL_REQUEST {
            let _ = s.write_all(&[answer]).await;
        }
        // hang up the sending side so a client waiting for an answer sees the end and returns, whatever it was going to do
        let _ = s.shutdown().await;
        let _ = s.read_to_end(&mut got).await;
        got
    });
    (port, handle)
}

fn url(port: u16, query: &str) -> String {
    format!("postgres://u:p@127.0.0.1:{port}/db{query}")
}

#[tokio::test]
async fn verify_ca_does_not_fall_back_to_plaintext_when_the_server_says_no_to_tls() {
    // tokio-postgres defaults to `sslmode=prefer`, which goes on in plaintext if the server answers 'N'. `verify-ca` must not.
    let (port, spy) = spy(b'N').await;
    let tls = Tls::from_settings(Some("verify-ca"), Some(ca_pem("any").as_bytes())).unwrap();
    let got = connect(&url(port, ""), &tls).await;
    let sent = spy.await.unwrap();
    let Err(err) = got else { panic!("a server that refuses TLS must fail verify-ca, not be talked to in plaintext") };
    assert_eq!(sent, SSL_REQUEST, "after the server said no, the client sent more: it went on without TLS");
    // and the person reading the log is told why: tokio-postgres's own text for this is only "error performing TLS handshake"
    let said = error_chain(&err);
    assert!(
        said.contains("server does not support TLS"),
        "the refusal does not say that the server has no TLS: {said}"
    );
}

#[tokio::test]
async fn the_spy_would_have_seen_a_plaintext_start() {
    // the control for the test above: the same spy against `disable`, which does start in plaintext. Without it, "the client sent
    // nothing more" could be a spy that sees nothing.
    let (port, spy) = spy(b'N').await;
    let got = connect(&url(port, ""), &Tls::from_settings(None, None).unwrap()).await;
    let sent = spy.await.unwrap();
    assert!(got.is_err(), "the spy hangs up, so this connect fails");
    assert!(
        sent.len() >= 8 && sent[4..8] == [0, 3, 0, 0],
        "a plaintext StartupMessage (protocol 3.0) must be what arrived: {sent:?}"
    );
}

#[rstest]
#[case::url_asks_for_tls_and_setting_says_disable("?sslmode=require", None, "the URL says sslmode=require")]
#[case::url_says_disable_and_setting_asks_for_tls(
    "?sslmode=disable",
    Some("verify-ca"),
    "the URL says sslmode=disable"
)]
#[tokio::test]
async fn a_url_and_a_setting_that_contradict_each_other_are_refused_before_any_connection(
    #[case] query: &str,
    #[case] mode: Option<&str>,
    #[case] words: &str,
) {
    let good = ca_pem("any");
    let tls = Tls::from_settings(mode, mode.map(|_| good.as_bytes())).unwrap();
    // port 1 has nothing listening: a refusal that needed the network would be a different error
    let got = connect(&url(1, query), &tls).await;
    match got {
        // the text a person reads, not the string inside the variant
        Err(e @ ConnectError::SslModeConflict(_)) => assert!(e.to_string().contains(words), "'{e}' lacks '{words}'"),
        Err(other) => panic!("a conflict is refused as a conflict, got {other}"),
        Ok(_) => panic!("a conflict must not connect"),
    }
}

// ---- what is printed ---------------------------------------------------------------------------------------------------------

#[test]
fn a_tls_prints_its_mode_and_nothing_else() {
    for (mode, root, want) in
        [(None, None, "Tls(disable)"), (Some("verify-ca"), Some(ca_pem("x").into_bytes()), "Tls(verify-ca)")]
    {
        let tls = Tls::from_settings(mode, root.as_deref()).unwrap();
        assert_eq!(format!("{tls:?}"), want);
    }
}

#[tokio::test]
async fn a_url_that_does_not_parse_is_reported_without_its_password() {
    // pinned: DATABASE_URL has the password in it, and a parse failure is the error most likely to be printed into a job log
    let secret = "SECRET-PW-0451";
    let url = format!("postgres://u:{secret}@127.0.0.1:not-a-port/db");
    let Err(err) = connect(&url, &Tls::Disable).await else { panic!("a URL with no port number must not connect") };
    assert!(matches!(err, ConnectError::Url(_)), "{err:?}");
    let shown = format!("{err} {err:?}");
    assert!(shown.contains("DATABASE_URL"), "{shown}");
    assert!(!shown.contains(secret), "the password is in the error: {shown}");
    assert!(std::error::Error::source(&err).is_none());
}

#[tokio::test]
async fn a_refused_connection_is_reported_with_its_cause() {
    // a port that had a listener a moment ago and has none: the connection is refused, whatever else the machine runs
    let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let Err(err) = connect(&url(port, ""), &Tls::Disable).await else { panic!("nothing listens there") };
    assert!(matches!(err, ConnectError::Postgres(_)), "{err:?}");
    assert!(!err.to_string().is_empty());
    assert!(std::error::Error::source(&err).is_some(), "the cause must be reachable");
    // and it is in the text a person reads: the cause comes after what the error itself says, and says more
    let said = error_chain(&err);
    assert!(
        said.starts_with(&err.to_string()) && said.len() > err.to_string().len(),
        "'{said}' adds nothing to '{err}'"
    );
}

// ---- the text a person reads when a connection fails -------------------------------------------------------------------------

/// One layer of an error chain: shows `.0`, has the next layer as its source.
#[derive(Debug)]
struct Layer(String, Option<Box<Layer>>);

impl fmt::Display for Layer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Layer {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.1.as_deref().map(|l| l as &(dyn std::error::Error + 'static))
    }
}

/// `texts` outermost first.
fn layers(texts: &[String]) -> Layer {
    let mut rev = texts.iter().rev();
    let innermost = Layer(rev.next().cloned().unwrap_or_default(), None);
    rev.fold(innermost, |inner, t| Layer(t.clone(), Some(Box::new(inner))))
}

proptest! {
    #![proptest_config(proptest_config(256))]

    /// Every layer's text, outermost first, joined by `: `, however deep the chain. The index in each text keeps two layers from
    /// accidentally repeating each other, which is the one case `error_chain` deliberately prints once.
    #[test]
    fn prop_error_chain_is_every_layer_outermost_first(words in prop::collection::vec("[a-z]{1,8}( [a-z]{1,8}){0,2}", 1..6)) {
        let texts: Vec<String> = words.iter().enumerate().map(|(i, w)| format!("L{i} {w}")).collect();
        prop_assert_eq!(error_chain(&layers(&texts)), texts.join(": "));
    }
}

#[test]
fn an_error_that_shows_the_same_text_as_its_cause_is_printed_once() {
    // pinned: `ConnectError::Postgres(e)` displays as `e` and has `e` as its source; without this the line would read "same: same: deeper"
    let texts = ["same".to_string(), "same".to_string(), "deeper".to_string()];
    assert_eq!(error_chain(&layers(&texts)), "same: deeper");
}
