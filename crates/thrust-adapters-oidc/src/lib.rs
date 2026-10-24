//! The OIDC relying party (ADR-0050): authorization code with PKCE, discovery through
//! `.well-known/openid-configuration`, and ID-token verification, by the `openidconnect` crate.
//!
//! The crate does no I/O of its own. Discovery, the token endpoint and the key set are fetched through the [`Fetch`]
//! port, so one implementation serves the native server (reqwest) and the Worker (`worker::Fetch`) (issue #38, spike
//! 0003 K1). It holds no per-login state: `state`, `nonce` and the PKCE verifier are the caller's, stored in a database
//! row between the two legs, so an isolate that dies between them loses nothing.
//!
//! Every call discovers the provider again (a metadata fetch and a key-set fetch). That is two extra round trips per
//! leg and no cache to go stale or to differ between targets; a cache is a later optimisation, not a P0 question.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use async_trait::async_trait;
use openidconnect::core::{CoreClient, CoreProviderMetadata, CoreResponseType};
use openidconnect::{
    AsyncHttpClient, AuthenticationFlow, AuthorizationCode, ClientId, ClientSecret, CsrfToken, HttpRequest,
    HttpResponse, IssuerUrl, Nonce, PkceCodeChallenge, PkceCodeVerifier, RedirectUrl, RequestTokenError, Scope,
    TokenResponse as _,
};
use thrust_ports::{Fetch, FetchError, FetchRequest, OidcClaims, OidcError, OidcProvider};

/// One provider and this application's registration with it.
#[derive(Debug, Clone)]
pub struct OidcConfig {
    /// The issuer URL: discovery is at `{issuer}/.well-known/openid-configuration`, and the document's `issuer` must
    /// equal it.
    pub issuer: String,
    pub client_id: String,
    /// `None` for a public client (PKCE alone).
    pub client_secret: Option<String>,
    /// Where the provider sends the browser back: the callback route's absolute URL. Registered with the provider.
    pub redirect_uri: String,
}

/// [`OidcProvider`] over [`Fetch`].
pub struct OpenIdProvider {
    cfg: OidcConfig,
    http: FetchClient,
}

impl OpenIdProvider {
    pub fn new(cfg: OidcConfig, fetch: Arc<dyn Fetch>) -> Self {
        Self { cfg, http: FetchClient(fetch) }
    }

    /// Fetch the provider's discovery document and key set.
    async fn metadata(&self) -> Result<CoreProviderMetadata, OidcError> {
        let issuer = IssuerUrl::new(self.cfg.issuer.clone()).map_err(|e| OidcError::Provider(format!("issuer: {e}")))?;
        CoreProviderMetadata::discover_async(issuer, &self.http)
            .await
            .map_err(|e| OidcError::Provider(format!("discovery: {e}")))
    }

    fn redirect_url(&self) -> Result<RedirectUrl, OidcError> {
        RedirectUrl::new(self.cfg.redirect_uri.clone()).map_err(|e| OidcError::Provider(format!("redirect uri: {e}")))
    }

    fn client_id(&self) -> ClientId {
        ClientId::new(self.cfg.client_id.clone())
    }

    fn client_secret(&self) -> Option<ClientSecret> {
        self.cfg.client_secret.clone().map(ClientSecret::new)
    }
}

#[async_trait]
impl OidcProvider for OpenIdProvider {
    async fn authorization_url(&self, state: &str, nonce: &str, code_verifier: &str) -> Result<String, OidcError> {
        let client = CoreClient::from_provider_metadata(self.metadata().await?, self.client_id(), self.client_secret())
            .set_redirect_uri(self.redirect_url()?);
        // owned, so the closures below borrow nothing from this call
        let (state, nonce) = (CsrfToken::new(state.to_string()), Nonce::new(nonce.to_string()));
        let challenge = PkceCodeChallenge::from_code_verifier_sha256(&PkceCodeVerifier::new(code_verifier.to_string()));
        let (url, _, _) = client
            .authorize_url(
                AuthenticationFlow::<CoreResponseType>::AuthorizationCode,
                move || state,
                move || nonce,
            )
            .add_scope(Scope::new("email".to_string()))
            .add_scope(Scope::new("profile".to_string()))
            .set_pkce_challenge(challenge)
            .url();
        Ok(url.to_string())
    }

    async fn exchange(&self, code: &str, code_verifier: &str, nonce: &str) -> Result<OidcClaims, OidcError> {
        let client = CoreClient::from_provider_metadata(self.metadata().await?, self.client_id(), self.client_secret())
            .set_redirect_uri(self.redirect_url()?);
        let token = client
            .exchange_code(AuthorizationCode::new(code.to_string()))
            .map_err(|e| OidcError::Provider(format!("token endpoint: {e}")))?
            .set_pkce_verifier(PkceCodeVerifier::new(code_verifier.to_string()))
            .request_async(&self.http)
            .await
            .map_err(|e| match e {
                // the provider answered with an OAuth error (a bad or reused code, a failed PKCE check): not us, not it
                RequestTokenError::ServerResponse(r) => OidcError::Rejected(format!("token endpoint: {r}")),
                other => OidcError::Provider(format!("token endpoint: {other}")),
            })?;
        let id_token = token.id_token().ok_or_else(|| OidcError::Rejected("the response has no ID token".into()))?;
        // signature, issuer, audience, expiry and the nonce
        let claims = id_token
            .claims(&client.id_token_verifier(), &Nonce::new(nonce.to_string()))
            .map_err(|e| OidcError::Rejected(format!("ID token: {e}")))?;
        Ok(OidcClaims {
            issuer: claims.issuer().to_string(),
            subject: claims.subject().to_string(),
            email: claims.email().map(|e| e.to_string()),
            email_verified: claims.email_verified().unwrap_or(false),
        })
    }
}

/// The `Fetch` port as the HTTP client `openidconnect` wants. The future is `Send` (the port's is), which is what lets
/// a handler that awaits it sit in the shared axum router on both targets.
struct FetchClient(Arc<dyn Fetch>);

#[derive(Debug)]
struct ClientError(String);

impl std::fmt::Display for ClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ClientError {}

impl From<FetchError> for ClientError {
    fn from(e: FetchError) -> Self {
        ClientError(e.0)
    }
}

impl<'c> AsyncHttpClient<'c> for FetchClient {
    type Error = ClientError;
    type Future = Pin<Box<dyn Future<Output = Result<HttpResponse, ClientError>> + Send + 'c>>;

    fn call(&'c self, request: HttpRequest) -> Self::Future {
        Box::pin(async move {
            let (parts, body) = request.into_parts();
            let headers = parts
                .headers
                .iter()
                .map(|(k, v)| (k.to_string(), String::from_utf8_lossy(v.as_bytes()).into_owned()))
                .collect();
            let res = self
                .0
                .send(FetchRequest { method: parts.method.to_string(), url: parts.uri.to_string(), headers, body })
                .await?;
            let mut builder = openidconnect::http::Response::builder().status(res.status);
            for (k, v) in res.headers {
                builder = builder.header(k, v);
            }
            builder.body(res.body).map_err(|e| ClientError(e.to_string()))
        })
    }
}
