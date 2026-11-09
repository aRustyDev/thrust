//! What every v1 route does before its own work: read the API key, check the caller's permission, count the request against
//! the key's allowance, and report what is left on the answer.
//!
//! The order is DRF's, and the corpus pins it: **authentication, then permissions, then throttling, then the handler**. So a
//! missing key is a 401 and an invalid one a 403 before anything else; a caller who may not do the thing gets a 403 that is
//! not counted and carries no rate headers; and every answer after that, whatever its status, carries `X-RateLimit-Remaining`
//! and `X-RateLimit-Reset`. A 429 carries `Retry-After` and neither of those.

use std::sync::{Arc, Mutex, PoisonError};

use axum::extract::{FromRequestParts, Path, Request};
use axum::http::request::Parts;
use axum::http::{HeaderName, HeaderValue};
use axum::middleware::Next;
use axum::response::Response;
use serde::de::DeserializeOwned;
use serde::Deserialize;
use thrust_contracts::{ApiKeyId, Principal, ProjectId, Role, Uuid, WorkspaceSlug};
use thrust_core::{App, CoreError, Credentials, Permission, Via};
use thrust_ports::{RateDecision, Scope, WorkspaceScope};

use crate::V1Error;

const REMAINING: HeaderName = HeaderName::from_static("x-ratelimit-remaining");
const RESET: HeaderName = HeaderName::from_static("x-ratelimit-reset");

/// Plane's default allowance for one API key: `60/minute` (`apps/api/plane/settings/common.py`, `API_KEY_RATE_LIMIT`).
/// A composition root that is told otherwise puts a different one in the request's extensions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct V1Config {
    pub rate_limit: u32,
    pub rate_window_secs: u64,
}

impl Default for V1Config {
    fn default() -> Self {
        Self { rate_limit: 60, rate_window_secs: 60 }
    }
}

impl V1Config {
    /// Plane's spelling of `API_KEY_RATE_LIMIT` (DRF's): `N/period`, where the period is read from its first letter (`s`econd,
    /// `m`inute, `h`our, `d`ay), as in `100000/minute`; a bare `N` is per minute.
    pub fn parse(spec: &str) -> Result<Self, String> {
        let spec = spec.trim();
        let (count, period) = spec.split_once('/').unwrap_or((spec, "minute"));
        let rate_limit = count.trim().parse::<u32>().map_err(|_| format!("\"{count}\" is not a number of requests"))?;
        let rate_window_secs = match period.trim().chars().next() {
            Some('s') => 1,
            Some('m') => 60,
            Some('h') => 3600,
            Some('d') => 86_400,
            _ => return Err(format!("\"{period}\" is not a period (second, minute, hour or day)")),
        };
        Ok(Self { rate_limit, rate_window_secs })
    }
}

/// What the pin reports about a request that was let through.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RateInfo {
    /// What is left of the allowance after this request: the limit less the requests counted, this one included.
    pub remaining: u32,
    /// A Unix timestamp. The pin computes `int(now + duration)` on every request, so it is always one whole window ahead of
    /// the request, whatever the window's real end is (`apps/api/plane/api/rate_limit.py`).
    pub reset: i64,
}

/// A cell the middleware puts in the request and a [`Ctx`] fills, so the headers can go on any answer, an error included.
#[derive(Clone, Default)]
pub(crate) struct RateSlot(Arc<Mutex<Option<RateInfo>>>);

impl RateSlot {
    fn set(&self, info: RateInfo) {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = Some(info);
    }
    fn take(&self) -> Option<RateInfo> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).take()
    }
}

/// The layer that adds the rate headers to whatever the route answered, if the request got far enough to be counted.
pub(crate) async fn rate_headers(mut req: Request, next: Next) -> Response {
    let slot = RateSlot::default();
    req.extensions_mut().insert(slot.clone());
    let mut resp = next.run(req).await;
    if let Some(info) = slot.take() {
        resp.headers_mut().insert(REMAINING, HeaderValue::from(info.remaining));
        resp.headers_mut().insert(RESET, HeaderValue::from(info.reset));
    }
    resp
}

/// The caller of a v1 route: who they are by their API key (and nothing else: a bearer token or a cookie is not a credential
/// here), and what they may do.
pub struct Ctx {
    pub app: App,
    pub principal: Principal,
    key: ApiKeyId,
    cfg: V1Config,
    slot: RateSlot,
}

impl<S: Send + Sync> FromRequestParts<S> for Ctx {
    type Rejection = V1Error;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, V1Error> {
        let app = parts
            .extensions
            .get::<App>()
            .cloned()
            .ok_or_else(|| V1Error::internal("the v1 routes were mounted without the App extension"))?;
        let cfg = parts.extensions.get::<V1Config>().copied().unwrap_or_default();
        let slot = parts.extensions.get::<RateSlot>().cloned().unwrap_or_default();

        // The header as DRF reads it: an empty one is no key (401); one that is not text is a key nothing matches (403).
        let key = parts
            .headers
            .get(thrust_api::API_KEY_HEADER)
            .map(|v| String::from_utf8_lossy(v.as_bytes()).into_owned())
            .filter(|k| !k.is_empty());
        let credentials = Credentials {
            bearer: None,
            api_key: key.as_deref(),
            session_cookie: None,
            csrf_cookie: None,
            csrf_header: None,
            mutating: false,
        };
        let authenticated = thrust_core::authenticate_request(&app, &credentials).await?;
        match authenticated.via {
            Via::ApiKey(key) => Ok(Ctx { app, principal: authenticated.principal, key, cfg, slot }),
            // `authenticate_request` reads only the credentials given, and only an API key was given.
            Via::Session(_) => Err(V1Error::internal("an API key request was authenticated by a session")),
        }
    }
}

impl Ctx {
    /// The caller's permission on a project (the routes of states, labels and work items), then the throttle.
    ///
    /// The pin's permission classes answer 403 for a project the caller has no membership of, and for one that is not there:
    /// there is no membership to find. So both of core's answers are the same here.
    pub async fn project_scope(&self, slug: &str, project: ProjectId, perm: Permission) -> Result<Scope, V1Error> {
        let scope = thrust_core::authorize(&self.app, &self.principal, &WorkspaceSlug::new(slug), project, perm)
            .await
            .map_err(denied)?;
        self.throttle().await?;
        Ok(scope)
    }

    /// The caller's membership of a workspace with at least `min` as their role there (the routes of projects), then the throttle.
    /// A workspace that is not there, one the caller is not in, and a role that is not enough are all a 403, and none is counted.
    pub async fn workspace_scope(&self, slug: &str, min: Role) -> Result<WorkspaceScope, V1Error> {
        let scope = self.workspace_access(slug).await?;
        if scope.role() < min {
            return Err(V1Error::PermissionDenied);
        }
        self.throttle().await?;
        Ok(scope)
    }

    /// The caller's membership of a workspace, with no throttle: for a route whose permission is more than a role (a project's
    /// writers, which core decides), so that the route counts the request itself once the permission has passed.
    pub async fn workspace_access(&self, slug: &str) -> Result<WorkspaceScope, V1Error> {
        thrust_core::authorize_workspace(&self.app, &self.principal, &WorkspaceSlug::new(slug)).await.map_err(denied)
    }

    /// Count this request against the key. The key is its own allowance: two keys of one user do not share one.
    pub async fn throttle(&self) -> Result<(), V1Error> {
        let key = format!("v1:{}", self.key.0);
        let decision = self
            .app
            .ports
            .limiter
            .hit(&key, self.cfg.rate_limit, self.cfg.rate_window_secs)
            .await
            .map_err(CoreError::from)?;
        match decision {
            RateDecision::Limited { retry_after_secs } => Err(V1Error::RateLimited { retry_after_secs }),
            RateDecision::Allowed { remaining } => {
                let now = self.app.ports.clock.now().timestamp();
                let window = i64::try_from(self.cfg.rate_window_secs).unwrap_or(i64::MAX);
                self.slot.set(RateInfo { remaining, reset: now.saturating_add(window) });
                Ok(())
            }
        }
    }
}

/// Core says "no such project" and "not a member" apart; the pin's permission classes do not.
fn denied(e: CoreError) -> V1Error {
    match e {
        CoreError::NotFound | CoreError::Forbidden => V1Error::PermissionDenied,
        other => other.into(),
    }
}

/// A uuid as Django's `<uuid:...>` converter reads it: lower-case hex in the canonical hyphenated form, and nothing else.
/// (`uuid` itself also takes upper case, braces and bare hex, none of which the pin's URLconf matches.)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlaneUuid(pub Uuid);

impl TryFrom<String> for PlaneUuid {
    type Error = &'static str;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        let id = Uuid::parse_str(&s).map_err(|_| "not a uuid")?;
        if id.hyphenated().to_string() == s {
            Ok(PlaneUuid(id))
        } else {
            Err("not a canonical uuid")
        }
    }
}

impl<'de> Deserialize<'de> for PlaneUuid {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        PlaneUuid::try_from(s).map_err(serde::de::Error::custom)
    }
}

/// `axum`'s `Path`, whose rejection is the URLconf's: a route that does not match is Django's 404 with no credential read,
/// so put this extractor before [`Ctx`].
pub struct V1Path<T>(pub T);

impl<S: Send + Sync, T: DeserializeOwned + Send> FromRequestParts<S> for V1Path<T> {
    type Rejection = V1Error;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, V1Error> {
        match Path::<T>::from_request_parts(parts, state).await {
            Ok(Path(v)) => Ok(V1Path(v)),
            Err(_) => Err(V1Error::PageNotFound),
        }
    }
}
