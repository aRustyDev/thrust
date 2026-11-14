//! TLS for a native connection to Postgres (issue #49): what `migrate` needs to reach `thrust-pg` from the deploy job.
//!
//! `thrust-pg` accepts only TLS (`hostnossl all all all reject`), and the deploy reaches it through `cloudflared access tcp`, that
//! is at `127.0.0.1`, while the server's certificate (a Cloudflare Origin CA one, ADR-0094) names a host. So the one mode this
//! offers besides `disable` is `verify-ca`, libpq's word for it: the server's certificate must chain to a CA the caller gave, be
//! signed correctly and be in date, and its NAME is not compared. The name is the only check skipped, and the skip is in one
//! place (`ChainOnly`), which `tests/tls.rs` runs against a real rustls server for every other fault.
//!
//! What is left out on purpose. There is no `require` (TLS that trusts any certificate, which is what libpq's `require` is) and
//! no `verify-full` (the name checked too): nothing here calls for either, and a mode nobody uses is a mode that can be reached by
//! a typo. Any other setting is refused, not mapped to the nearest one.
//!
//! The crypto is rustls with the `ring` provider, and only the root certificates the caller names are trusted: not the operating
//! system's, whatever the machine happens to hold.

use std::fmt;
use std::future::Future;
use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::client::WebPkiServerVerifier;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, InvalidDnsNameError, ServerName, UnixTime};
use rustls::{CertificateError, ClientConfig, DigitallySignedStruct, RootCertStore, SignatureScheme};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio_postgres::config::SslMode;
use tokio_postgres::tls::{ChannelBinding, MakeTlsConnect, TlsConnect, TlsStream};

/// What a caller asked for, and the TLS configuration that goes with it.
#[derive(Clone)]
pub enum Tls {
    /// No TLS. What `migrate` has always done, and the default, so the CI paths that migrate a throwaway container are unchanged.
    Disable,
    /// TLS, with the server's certificate chain checked against the given roots and its name not checked.
    VerifyCa(Arc<ClientConfig>),
}

impl fmt::Debug for Tls {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Tls({})", self.mode())
    }
}

/// Why `DATABASE_SSLMODE` and `DATABASE_SSLROOTCERT` cannot be used. Each says which of the two to change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TlsConfigError {
    /// A mode other than `disable` and `verify-ca`, whatever its spelling.
    UnsupportedMode(String),
    /// `verify-ca` and no root certificate file.
    RootCertMissing,
    /// A root certificate file and a mode that does not use it: one of the two is a mistake.
    RootCertWithoutVerify,
    /// The file holds no PEM certificate at all.
    RootCertEmpty,
    /// The file holds a certificate that cannot be read. Refused whole: a bundle is not trusted in part.
    RootCertInvalid,
}

impl fmt::Display for TlsConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedMode(m) => {
                write!(f, "DATABASE_SSLMODE '{m}' is not supported: use 'disable' (the default) or 'verify-ca'")
            }
            Self::RootCertMissing => {
                write!(f, "DATABASE_SSLMODE is verify-ca, so DATABASE_SSLROOTCERT must name a PEM file with the CA certificate(s) to trust")
            }
            Self::RootCertWithoutVerify => write!(
                f,
                "DATABASE_SSLROOTCERT is set but DATABASE_SSLMODE is not verify-ca: a root certificate does nothing without it, so one of the two is a mistake"
            ),
            Self::RootCertEmpty => write!(f, "DATABASE_SSLROOTCERT holds no PEM certificate"),
            Self::RootCertInvalid => write!(
                f,
                "DATABASE_SSLROOTCERT holds a certificate that cannot be read: nothing in the file is trusted unless all of it is good"
            ),
        }
    }
}

impl std::error::Error for TlsConfigError {}

impl Tls {
    /// Turn the two settings into a configuration, or say which one is wrong. Pure: the caller reads the file and the environment.
    pub fn from_settings(mode: Option<&str>, root_cert_pem: Option<&[u8]>) -> Result<Tls, TlsConfigError> {
        match mode {
            None | Some("disable") => match root_cert_pem {
                None => Ok(Tls::Disable),
                Some(_) => Err(TlsConfigError::RootCertWithoutVerify),
            },
            Some("verify-ca") => {
                let pem = root_cert_pem.ok_or(TlsConfigError::RootCertMissing)?;
                Ok(Tls::VerifyCa(Arc::new(verify_ca_config(pem)?)))
            }
            Some(other) => Err(TlsConfigError::UnsupportedMode(other.to_string())),
        }
    }

    /// `disable` or `verify-ca`: the setting this was made from.
    pub fn mode(&self) -> &'static str {
        match self {
            Tls::Disable => "disable",
            Tls::VerifyCa(_) => "verify-ca",
        }
    }

    /// The connector to hand `tokio_postgres`, or none for `disable`.
    pub fn connector(&self) -> Option<RustlsConnector> {
        match self {
            Tls::Disable => None,
            Tls::VerifyCa(config) => Some(RustlsConnector { config: config.clone() }),
        }
    }
}

fn provider() -> Arc<rustls::crypto::CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

fn verify_ca_config(root_cert_pem: &[u8]) -> Result<ClientConfig, TlsConfigError> {
    let mut roots = RootCertStore::empty();
    let mut found = 0usize;
    for cert in CertificateDer::pem_slice_iter(root_cert_pem) {
        let cert = cert.map_err(|_| TlsConfigError::RootCertInvalid)?;
        roots.add(cert).map_err(|_| TlsConfigError::RootCertInvalid)?;
        found += 1;
    }
    if found == 0 {
        return Err(TlsConfigError::RootCertEmpty);
    }
    let provider = provider();
    let inner = WebPkiServerVerifier::builder_with_provider(Arc::new(roots), provider.clone())
        .build()
        .map_err(|_| TlsConfigError::RootCertInvalid)?;
    ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|_| TlsConfigError::RootCertInvalid)
        .map(|b| b.dangerous().with_custom_certificate_verifier(Arc::new(ChainOnly { inner })).with_no_client_auth())
}

/// The ordinary web PKI check (chain to a given root, signatures, dates, key usage) with ONE answer changed: a certificate that is
/// good in every way and carries another name is accepted.
///
/// Why that is the only thing it can change. rustls checks the chain and the dates first and the name last, so the "not valid for
/// this name" error exists only for a certificate that passed everything else; a chain, signature or date fault comes back as its
/// own error and is returned as it is. `tests/tls.rs` holds every one of those faults against a real handshake.
#[derive(Debug)]
struct ChainOnly {
    inner: Arc<WebPkiServerVerifier>,
}

impl ServerCertVerifier for ChainOnly {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        match self.inner.verify_server_cert(end_entity, intermediates, server_name, ocsp_response, now) {
            Err(rustls::Error::InvalidCertificate(CertificateError::NotValidForName))
            | Err(rustls::Error::InvalidCertificate(CertificateError::NotValidForNameContext { .. })) => {
                Ok(ServerCertVerified::assertion())
            }
            other => other,
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.inner.verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.inner.verify_tls13_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.inner.supported_verify_schemes()
    }
}

// ---- the glue tokio-postgres needs ----------------------------------------------------------------------------------------

/// `tokio_postgres`'s `MakeTlsConnect` over rustls. Channel binding is not offered, so a server that insists on SCRAM-PLUS would
/// refuse it; `scram-sha-256` and the `channel_binding=prefer` default are unaffected.
#[derive(Clone)]
pub struct RustlsConnector {
    config: Arc<ClientConfig>,
}

impl<S> MakeTlsConnect<S> for RustlsConnector
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    type Stream = RustlsStream<S>;
    type TlsConnect = RustlsConnect;
    type Error = InvalidDnsNameError;

    fn make_tls_connect(&mut self, domain: &str) -> Result<RustlsConnect, InvalidDnsNameError> {
        // an IP address is a valid server name too; the name is not compared under verify-ca, but it must be one rustls can hold
        let name = ServerName::try_from(domain.to_owned())?;
        Ok(RustlsConnect { connector: tokio_rustls::TlsConnector::from(self.config.clone()), name })
    }
}

pub struct RustlsConnect {
    connector: tokio_rustls::TlsConnector,
    name: ServerName<'static>,
}

impl<S> TlsConnect<S> for RustlsConnect
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    type Stream = RustlsStream<S>;
    type Error = io::Error;
    type Future = Pin<Box<dyn Future<Output = io::Result<RustlsStream<S>>> + Send>>;

    fn connect(self, stream: S) -> Self::Future {
        Box::pin(async move { Ok(RustlsStream(self.connector.connect(self.name, stream).await?)) })
    }
}

/// An established TLS stream. A newtype only so that it can implement tokio-postgres's `TlsStream`.
pub struct RustlsStream<S>(tokio_rustls::client::TlsStream<S>);

impl<S: AsyncRead + AsyncWrite + Unpin> AsyncRead for RustlsStream<S> {
    fn poll_read(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_read(cx, buf)
    }
}

impl<S: AsyncRead + AsyncWrite + Unpin> AsyncWrite for RustlsStream<S> {
    fn poll_write(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.0).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_shutdown(cx)
    }
}

impl<S: AsyncRead + AsyncWrite + Unpin> TlsStream for RustlsStream<S> {
    fn channel_binding(&self) -> ChannelBinding {
        ChannelBinding::none()
    }
}

// ---- connecting -----------------------------------------------------------------------------------------------------------

/// A connected client and the task that drives its socket.
pub struct Connection {
    pub client: tokio_postgres::Client,
    driver: tokio::task::JoinHandle<Result<(), tokio_postgres::Error>>,
}

impl Connection {
    /// Drop the client and wait for the connection task to end.
    pub async fn close(self) -> Result<(), tokio_postgres::Error> {
        let Connection { client, driver } = self;
        drop(client);
        // a task that panicked or was cancelled has nothing more to report than that it is over
        driver.await.unwrap_or(Ok(()))
    }
}

#[derive(Debug)]
pub enum ConnectError {
    /// The URL does not parse. The URL is never put in the message: it holds the password.
    Url(String),
    /// The URL's `sslmode` and `DATABASE_SSLMODE` say opposite things. Refused, not resolved in favour of either.
    SslModeConflict(String),
    Postgres(tokio_postgres::Error),
}

impl fmt::Display for ConnectError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Url(m) => write!(f, "DATABASE_URL is not a valid connection string: {m}"),
            Self::SslModeConflict(m) => f.write_str(m),
            Self::Postgres(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for ConnectError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Postgres(e) => Some(e),
            _ => None,
        }
    }
}

/// An error and everything under it, as one line: `error performing TLS handshake: server does not support TLS`.
///
/// tokio-postgres keeps its cause out of its `Display` (it says "error performing TLS handshake", "db error" and nothing more), and
/// the cause is the part a person needs: which certificate fault, which `pg_hba.conf` line, which refusal. `Debug` has it too, but as
/// a struct dump. This is what `migrate` prints and what the tests read, so a deploy log says why the connection failed.
pub fn error_chain(err: &(dyn std::error::Error + 'static)) -> String {
    let mut line = err.to_string();
    let mut next = err.source();
    while let Some(cause) = next {
        let text = cause.to_string();
        // an error that repeats its cause in its own text (`{e}: {source}`) would print it twice
        if !line.ends_with(&text) {
            line.push_str(": ");
            line.push_str(&text);
        }
        next = cause.source();
    }
    line
}

/// Connect to `url` as `tls` says.
///
/// Two ways of going on without TLS are closed. `tokio_postgres` defaults to `sslmode=prefer`, which carries on in plaintext when
/// the server answers that it has no TLS, so `verify-ca` forces `require`: a server that says no is an error. And a URL whose own
/// `sslmode` contradicts the setting is refused before anything is sent, rather than one of the two being quietly ignored.
pub async fn connect(url: &str, tls: &Tls) -> Result<Connection, ConnectError> {
    let mut config: tokio_postgres::Config =
        url.parse().map_err(|e: tokio_postgres::Error| ConnectError::Url(e.to_string()))?;
    match (tls, config.get_ssl_mode()) {
        (Tls::Disable, SslMode::Require) => {
            return Err(ConnectError::SslModeConflict(
                "the URL says sslmode=require but DATABASE_SSLMODE is disable (the default): set DATABASE_SSLMODE=verify-ca and \
                 DATABASE_SSLROOTCERT, or take sslmode=require out of the URL"
                    .into(),
            ));
        }
        (Tls::VerifyCa(_), SslMode::Disable) => {
            return Err(ConnectError::SslModeConflict(
                "the URL says sslmode=disable but DATABASE_SSLMODE is verify-ca: take sslmode=disable out of the URL, or set \
                 DATABASE_SSLMODE=disable"
                    .into(),
            ));
        }
        _ => {}
    }
    let driver = match tls.connector() {
        None => {
            let (client, conn) = config.connect(tokio_postgres::NoTls).await.map_err(ConnectError::Postgres)?;
            (client, tokio::spawn(conn))
        }
        Some(connector) => {
            config.ssl_mode(SslMode::Require);
            let (client, conn) = config.connect(connector).await.map_err(ConnectError::Postgres)?;
            (client, tokio::spawn(conn))
        }
    };
    Ok(Connection { client: driver.0, driver: driver.1 })
}
