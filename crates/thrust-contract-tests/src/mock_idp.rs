//! A mock OIDC identity provider, served by the test process on `127.0.0.1` (issue #38, spike 0003 step 4).
//!
//! It speaks what Zitadel speaks, written by hand: a discovery document, a JWKS with one RS256 key, and a token endpoint
//! that checks the client, the redirect URI and the PKCE verifier, and signs the ID token with the `rsa` crate. The
//! target under test verifies those tokens with the `openidconnect` crate, so the two sides share no code.
//!
//! The browser's leg (`/authorize`) is a method, not a route: [`MockIdp::authorize`] takes the URL the target sent the
//! browser to, checks that it carries everything an authorization-code login with PKCE must (an independent
//! assertion about the target's request), picks the identity that "logs in", and returns the callback URL the provider
//! would send the browser back to. `tamper` makes the token endpoint answer with an ID token that must not be accepted.
//!
//! One instance per test process, on its own thread and runtime (a `#[tokio::test]` runtime ends with its test).
//! Configuration comes from the environment `scripts/contract.sh` sets for the target as well: `THRUST_MOCK_IDP_PORT`,
//! `THRUST_OIDC_CLIENT_ID`, `THRUST_OIDC_CLIENT_SECRET`, `THRUST_OIDC_REDIRECT_URI`. A deployed Worker cannot reach
//! `127.0.0.1`, so these tests are not part of `scripts/stage2-contract.sh`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, post};
use axum::{Form, Json, Router};
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use base64::Engine as _;
use rsa::pkcs1v15::SigningKey;
use rsa::sha2::{Digest, Sha256};
use rsa::signature::{SignatureEncoding, Signer};
use rsa::traits::PublicKeyParts;
use rsa::RsaPrivateKey;
use serde_json::{json, Value};

/// Who "logs in" at the provider and what the ID token claims about them.
#[derive(Debug, Clone)]
pub struct Identity {
    pub sub: String,
    pub email: Option<String>,
    /// `None`: the `email_verified` claim is absent.
    pub email_verified: Option<bool>,
}

impl Identity {
    pub fn verified(sub: &str, email: &str) -> Identity {
        Identity { sub: sub.into(), email: Some(email.into()), email_verified: Some(true) }
    }
}

/// How the ID token is made wrong, if it is. Each must be refused by a relying party that verifies what it is given.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tamper {
    None,
    /// The token carries a nonce other than the one in the authorization request.
    WrongNonce,
    /// `aud` names another client.
    WrongAudience,
    /// `iss` names another issuer.
    WrongIssuer,
    /// `exp` is an hour ago.
    Expired,
    /// The signature is corrupted.
    BadSignature,
}

/// What the browser is sent back with.
#[derive(Debug, Clone)]
pub struct Callback {
    pub redirect_uri: String,
    pub code: String,
    pub state: String,
}

impl Callback {
    pub fn url(&self) -> String {
        format!("{}?code={}&state={}", self.redirect_uri, self.code, self.state)
    }
}

struct Code {
    identity: Identity,
    nonce: String,
    challenge: String,
    redirect_uri: String,
    tamper: Tamper,
    used: bool,
}

struct Inner {
    issuer: String,
    client_id: String,
    client_secret: String,
    redirect_uri: String,
    key: RsaPrivateKey,
    codes: Mutex<HashMap<String, Code>>,
}

pub struct MockIdp {
    inner: Arc<Inner>,
}

fn env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| {
        panic!("{name} is not set: the OIDC tests need the mock identity provider's configuration (scripts/contract.sh sets it)")
    })
}

static SHARED: OnceLock<MockIdp> = OnceLock::new();

impl MockIdp {
    /// The process's mock provider, started on first use.
    pub fn shared() -> &'static MockIdp {
        SHARED.get_or_init(|| {
            let port: u16 = env("THRUST_MOCK_IDP_PORT").parse().expect("THRUST_MOCK_IDP_PORT is a port");
            let inner = Arc::new(Inner {
                issuer: format!("http://127.0.0.1:{port}"),
                client_id: env("THRUST_OIDC_CLIENT_ID"),
                client_secret: env("THRUST_OIDC_CLIENT_SECRET"),
                redirect_uri: env("THRUST_OIDC_REDIRECT_URI"),
                key: RsaPrivateKey::new(&mut rand::rngs::OsRng, 2048).expect("an RSA key"),
                codes: Mutex::new(HashMap::new()),
            });
            let (ready_tx, ready_rx) = std::sync::mpsc::channel();
            let served = inner.clone();
            std::thread::Builder::new()
                .name("mock-idp".into())
                .spawn(move || {
                    let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap();
                    rt.block_on(async move {
                        let listener = tokio::net::TcpListener::bind(("127.0.0.1", port))
                            .await
                            .expect("the mock identity provider's port is free");
                        let app = Router::new()
                            .route("/.well-known/openid-configuration", get(discovery))
                            .route("/jwks", get(jwks))
                            .route("/token", post(token))
                            .with_state(served);
                        ready_tx.send(()).unwrap();
                        axum::serve(listener, app).await.unwrap();
                    });
                })
                .unwrap();
            ready_rx.recv_timeout(std::time::Duration::from_secs(60)).expect("the mock identity provider started");
            MockIdp { inner }
        })
    }

    pub fn issuer(&self) -> &str {
        &self.inner.issuer
    }

    pub fn client_id(&self) -> &str {
        &self.inner.client_id
    }

    pub fn token_endpoint(&self) -> String {
        format!("{}/token", self.inner.issuer)
    }

    /// The browser's visit to the provider: check the authorization request the target built, log `identity` in, and
    /// return what the provider would redirect the browser to. Panics if the request is not a well-formed
    /// authorization-code request with PKCE (S256), the right client and the registered redirect URI.
    pub fn authorize(&self, authorization_url: &str, identity: Identity, tamper: Tamper) -> Callback {
        let url = reqwest::Url::parse(authorization_url).expect("the Location is a URL");
        assert_eq!(
            format!("{}://{}:{}{}", url.scheme(), url.host_str().unwrap(), url.port().unwrap(), url.path()),
            format!("{}/authorize", self.inner.issuer),
            "the browser must be sent to the provider's authorization endpoint"
        );
        let q: HashMap<String, String> = url.query_pairs().map(|(k, v)| (k.into_owned(), v.into_owned())).collect();
        let get = |k: &str| q.get(k).unwrap_or_else(|| panic!("the authorization request has no `{k}`: {authorization_url}"));
        assert_eq!(get("response_type"), "code");
        assert_eq!(get("client_id"), &self.inner.client_id);
        assert_eq!(get("redirect_uri"), &self.inner.redirect_uri, "the registered redirect URI, exactly");
        let scopes: Vec<&str> = get("scope").split(' ').collect();
        assert!(scopes.contains(&"openid") && scopes.contains(&"email"), "scopes: {scopes:?}");
        let is_hex64 = |s: &str| s.len() == 64 && s.bytes().all(|c| c.is_ascii_hexdigit());
        assert!(is_hex64(get("state")), "state must be 256 random bits: {}", get("state"));
        assert!(is_hex64(get("nonce")), "nonce must be 256 random bits: {}", get("nonce"));
        assert_eq!(get("code_challenge_method"), "S256", "PKCE with S256");
        assert_eq!(get("code_challenge").len(), 43, "an S256 challenge is 43 characters of base64url");
        let code = format!("code-{}", uuid::Uuid::new_v4().simple());
        self.inner.codes.lock().unwrap().insert(
            code.clone(),
            Code {
                identity,
                nonce: get("nonce").clone(),
                challenge: get("code_challenge").clone(),
                redirect_uri: get("redirect_uri").clone(),
                tamper,
                used: false,
            },
        );
        Callback { redirect_uri: get("redirect_uri").clone(), code, state: get("state").clone() }
    }

    /// Control for the provider itself: what it does with an exchange that carries a PKCE verifier that is not the one
    /// the challenge was made from. It must refuse (`invalid_grant`), or no test that sends the right one proves PKCE.
    pub async fn exchange_with_verifier(&self, callback: &Callback, code_verifier: &str) -> (u16, Value) {
        let http = reqwest::Client::builder().no_proxy().build().unwrap();
        let r = http
            .post(self.token_endpoint())
            .basic_auth(&self.inner.client_id, Some(&self.inner.client_secret))
            .form(&[
                ("grant_type", "authorization_code"),
                ("code", callback.code.as_str()),
                ("redirect_uri", callback.redirect_uri.as_str()),
                ("code_verifier", code_verifier),
            ])
            .send()
            .await
            .unwrap();
        (r.status().as_u16(), r.json().await.unwrap())
    }
}

async fn discovery(State(s): State<Arc<Inner>>) -> Json<Value> {
    Json(json!({
        "issuer": s.issuer,
        "authorization_endpoint": format!("{}/authorize", s.issuer),
        "token_endpoint": format!("{}/token", s.issuer),
        "jwks_uri": format!("{}/jwks", s.issuer),
        "response_types_supported": ["code"],
        "subject_types_supported": ["public"],
        "id_token_signing_alg_values_supported": ["RS256"],
        "grant_types_supported": ["authorization_code"],
        "token_endpoint_auth_methods_supported": ["client_secret_basic", "client_secret_post"],
        "scopes_supported": ["openid", "email", "profile"],
        "claims_supported": ["sub", "email", "email_verified", "name"],
        "code_challenge_methods_supported": ["S256"],
    }))
}

async fn jwks(State(s): State<Arc<Inner>>) -> Json<Value> {
    let public = s.key.to_public_key();
    Json(json!({ "keys": [{
        "kty": "RSA", "use": "sig", "alg": "RS256", "kid": "mock-1",
        "n": B64.encode(public.n().to_bytes_be()),
        "e": B64.encode(public.e().to_bytes_be()),
    }]}))
}

fn oauth_error(status: StatusCode, error: &str, description: &str) -> (StatusCode, Json<Value>) {
    (status, Json(json!({ "error": error, "error_description": description })))
}

fn now_secs() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64
}

async fn token(
    State(s): State<Arc<Inner>>,
    headers: HeaderMap,
    Form(p): Form<HashMap<String, String>>,
) -> (StatusCode, Json<Value>) {
    // client authentication: HTTP Basic, or the secret in the body
    let basic = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Basic "))
        .and_then(|v| base64::engine::general_purpose::STANDARD.decode(v).ok())
        .and_then(|v| String::from_utf8(v).ok());
    let presented = basic.or_else(|| Some(format!("{}:{}", p.get("client_id")?, p.get("client_secret")?)));
    if presented.as_deref() != Some(format!("{}:{}", s.client_id, s.client_secret).as_str()) {
        return oauth_error(StatusCode::UNAUTHORIZED, "invalid_client", "client authentication failed");
    }
    if p.get("grant_type").map(String::as_str) != Some("authorization_code") {
        return oauth_error(StatusCode::BAD_REQUEST, "unsupported_grant_type", "authorization_code only");
    }
    let mut codes = s.codes.lock().unwrap();
    let Some(entry) = p.get("code").and_then(|c| codes.get_mut(c)) else {
        return oauth_error(StatusCode::BAD_REQUEST, "invalid_grant", "unknown code");
    };
    if std::mem::replace(&mut entry.used, true) {
        return oauth_error(StatusCode::BAD_REQUEST, "invalid_grant", "code already used");
    }
    if p.get("redirect_uri") != Some(&entry.redirect_uri) {
        return oauth_error(StatusCode::BAD_REQUEST, "invalid_grant", "redirect_uri mismatch");
    }
    // PKCE: S256(code_verifier) must be the challenge the authorization request carried
    let verifier = p.get("code_verifier").map(String::as_str).unwrap_or("");
    let challenge = B64.encode(Sha256::digest(verifier.as_bytes()));
    if challenge != entry.challenge {
        return oauth_error(StatusCode::BAD_REQUEST, "invalid_grant", "PKCE verification failed");
    }

    let now = now_secs();
    let t = entry.tamper;
    let mut claims = json!({
        "iss": if t == Tamper::WrongIssuer { "https://evil.example.test" } else { s.issuer.as_str() },
        "sub": entry.identity.sub,
        "aud": if t == Tamper::WrongAudience { "some-other-client" } else { s.client_id.as_str() },
        "exp": if t == Tamper::Expired { now - 3600 } else { now + 300 },
        "iat": if t == Tamper::Expired { now - 7200 } else { now },
        "nonce": if t == Tamper::WrongNonce { "forged-nonce" } else { entry.nonce.as_str() },
    });
    if let Some(email) = &entry.identity.email {
        claims["email"] = json!(email);
    }
    if let Some(v) = entry.identity.email_verified {
        claims["email_verified"] = json!(v);
    }
    let header = json!({ "alg": "RS256", "typ": "JWT", "kid": "mock-1" });
    let signing_input = format!("{}.{}", B64.encode(header.to_string()), B64.encode(claims.to_string()));
    let mut signature = SigningKey::<Sha256>::new(s.key.clone()).sign(signing_input.as_bytes()).to_vec();
    if t == Tamper::BadSignature {
        let last = signature.len() - 1;
        signature[last] ^= 0xff;
    }
    let id_token = format!("{signing_input}.{}", B64.encode(signature));
    (
        StatusCode::OK,
        Json(json!({
            "access_token": format!("at-{}", uuid::Uuid::new_v4().simple()),
            "token_type": "Bearer",
            "expires_in": 300,
            "id_token": id_token,
        })),
    )
}
