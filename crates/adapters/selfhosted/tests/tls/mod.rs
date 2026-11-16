//! `verify-ca` (issue #49, `thrust_adapters_postgres::tls`) against a REAL Postgres 16 that takes only TLS, as `thrust-pg` does
//! (`hostnossl all all all reject`). The in-memory handshakes in `thrust-adapters-postgres/tests/tls.rs` show what the verifier
//! accepts and refuses; this shows what only a server can: that Postgres's own SSLRequest exchange and SCRAM login work through the
//! client, that the SERVER says the session is encrypted, that the SERVER refuses a plaintext client, and that the whole schema
//! migrates over the encrypted connection.
//!
//! Certificates are made per run, for the name `thrust-pg.example.test`, while the client connects to `127.0.0.1`: the situation
//! the deploy is in through `cloudflared access tcp`, and the reason `verify-ca` does not compare names.

use rcgen::{
    date_time_ymd, BasicConstraints, CertificateParams, CertifiedIssuer, DnType, IsCa, KeyPair, KeyUsagePurpose,
};
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, ImageExt};
use testcontainers_modules::postgres::Postgres;
use thrust_adapters_postgres::tls::{connect, error_chain, Tls};

const CERT_NAME: &str = "thrust-pg.example.test";

fn ca() -> CertifiedIssuer<'static, KeyPair> {
    let mut p = CertificateParams::new(Vec::<String>::new()).unwrap();
    p.distinguished_name.push(DnType::CommonName, "thrust test CA");
    p.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    p.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    CertifiedIssuer::self_signed(p, KeyPair::generate().unwrap()).unwrap()
}

/// A server certificate signed by `issuer`, in PEM, with its key in PEM. `expired` makes one that ended in 2001.
fn server_cert(issuer: &CertifiedIssuer<'static, KeyPair>, expired: bool) -> (String, String) {
    let mut p = CertificateParams::new(vec![CERT_NAME.to_string()]).unwrap();
    if expired {
        p.not_before = date_time_ymd(2000, 1, 1);
        p.not_after = date_time_ymd(2001, 1, 1);
    }
    let key = KeyPair::generate().unwrap();
    let cert = p.signed_by(&key, issuer).unwrap();
    (cert.pem(), key.serialize_pem())
}

/// Written by the container's own command: the files have to belong to `postgres` and the key to be 0600, or Postgres refuses to
/// start, and `with_copy_to` makes root-owned files. The `exec` hands over to the image's entrypoint, which then initialises as it
/// always does. `pg_hba` is the point of this server: TLS or nothing, and `local` open for the entrypoint's own setup.
const TLS_ONLY_SERVER: &str = r#"set -e
mkdir -p /tls
printf '%s\n' "$TLS_CERT" > /tls/server.crt
printf '%s\n' "$TLS_KEY" > /tls/server.key
printf '%s\n' 'local all all trust' 'hostssl all all all scram-sha-256' 'hostnossl all all all reject' > /tls/pg_hba.conf
chown postgres:postgres /tls/server.crt /tls/server.key /tls/pg_hba.conf
chmod 600 /tls/server.key
exec docker-entrypoint.sh postgres -c fsync=off -c ssl=on -c ssl_cert_file=/tls/server.crt -c ssl_key_file=/tls/server.key -c hba_file=/tls/pg_hba.conf
"#;

async fn tls_only_server(cert_pem: &str, key_pem: &str) -> (ContainerAsync<Postgres>, String) {
    let container = Postgres::default()
        .with_tag("16-alpine")
        .with_env_var("TLS_CERT", cert_pem)
        .with_env_var("TLS_KEY", key_pem)
        .with_cmd(["sh", "-c", TLS_ONLY_SERVER])
        .start()
        .await
        .expect("start the TLS-only postgres container");
    let port = container.get_host_port_ipv4(5432).await.expect("mapped port");
    // testcontainers' default credentials are a throwaway for this container only
    (container, format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres"))
}

fn verify_ca(ca_pem: &str) -> Tls {
    Tls::from_settings(Some("verify-ca"), Some(ca_pem.as_bytes())).unwrap()
}

#[tokio::test]
async fn verify_ca_reaches_a_tls_only_server_and_the_server_says_the_session_is_encrypted() {
    let ca = ca();
    let (cert, key) = server_cert(&ca, false);
    let (_container, url) = tls_only_server(&cert, &key).await;

    let conn = connect(&url, &verify_ca(&ca.pem()))
        .await
        .expect("verify-ca with the right CA connects, whatever the host name is");
    // The server's own account of the session, not the client's: this is what makes "it connected" mean "it connected over TLS".
    let ssl: bool =
        conn.client.query_one("SELECT ssl FROM pg_stat_ssl WHERE pid = pg_backend_pid()", &[]).await.unwrap().get(0);
    assert!(ssl, "the server reports this session as not encrypted");
    thrust_adapters_postgres::migrate(&conn.client)
        .await
        .expect("the whole schema migrates over the encrypted connection");
    // What happens when the client hangs up on a Postgres is recorded, not asserted: it printed `Ok(())` against 16-alpine
    // (2026-11-13). `migrate` does not act on the result of `close` either way, and says why in its own comment.
    println!("CLOSE-OBSERVATION {:?}", conn.close().await);

    // CONTROL: the same server refuses a client that does not use TLS. Without it, "TLS-only" could be a server that takes anything.
    let plain = connect(&url, &Tls::from_settings(None, None).unwrap()).await;
    let msg = match plain {
        Ok(_) => panic!("a TLS-only server must refuse a plaintext client"),
        Err(e) => error_chain(&e),
    };
    assert!(msg.contains("no encryption"), "refused, but not by the server's pg_hba rule: {msg}");

    // CONTROL: the right server, an unrelated CA. Refused for the CA, so the success above was the verifier saying yes, not nothing.
    let wrong = ca_other();
    let refused = connect(&url, &verify_ca(&wrong.pem())).await;
    let msg = match refused {
        Ok(_) => panic!("an unrelated CA must not be trusted"),
        Err(e) => error_chain(&e),
    };
    assert!(msg.contains("UnknownIssuer"), "refused, but not for the certificate chain: {msg}");
}

fn ca_other() -> CertifiedIssuer<'static, KeyPair> {
    let mut p = CertificateParams::new(Vec::<String>::new()).unwrap();
    p.distinguished_name.push(DnType::CommonName, "an unrelated CA");
    p.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    p.key_usages = vec![KeyUsagePurpose::KeyCertSign];
    CertifiedIssuer::self_signed(p, KeyPair::generate().unwrap()).unwrap()
}

#[tokio::test]
async fn an_expired_server_certificate_is_refused_even_when_its_ca_is_trusted() {
    let ca = ca();
    let (cert, key) = server_cert(&ca, true);
    let (_container, url) = tls_only_server(&cert, &key).await;
    let got = connect(&url, &verify_ca(&ca.pem())).await;
    let msg = match got {
        Ok(_) => panic!("an expired certificate must be refused"),
        Err(e) => error_chain(&e),
    };
    assert!(msg.contains("xpired"), "refused, but not for the date: {msg}");
}

#[tokio::test]
async fn a_server_without_tls_is_refused_by_verify_ca_and_not_talked_to_in_plaintext() {
    // the image's own default: no certificate, ssl off. tokio-postgres's `prefer` would carry on in plaintext here.
    let container = Postgres::default().with_tag("16-alpine").start().await.expect("start postgres");
    let port = container.get_host_port_ipv4(5432).await.expect("mapped port");
    let url = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");
    let got = connect(&url, &verify_ca(&ca().pem())).await;
    let msg = match got {
        Ok(_) => panic!("verify-ca must not connect to a server that has no TLS"),
        Err(e) => error_chain(&e),
    };
    assert!(msg.contains("does not support TLS"), "refused, but not because the server has no TLS: {msg}");
}
