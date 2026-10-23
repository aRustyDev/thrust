//! Self-hosted (native) adapters: clock, ids and the `Background` executor on tokio.
//! The Postgres connection pool lives here too (added with the Postgres stage).

use chrono::Utc;
use thrust_contracts::{Timestamp, Uuid};
use async_trait::async_trait;
use thrust_ports::{
    Background, BoxFuture, Clock, Fetch, FetchError, FetchRequest, FetchResponse, IdGen, Rng, RngError,
};

pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Timestamp {
        Utc::now()
    }
}

pub struct NativeIdGen;

impl IdGen for NativeIdGen {
    fn new_uuid(&self) -> Uuid {
        Uuid::now_v7()
    }
}

/// The operating system's generator (`getrandom`): the `Rng` port natively.
pub struct NativeRng;

impl Rng for NativeRng {
    fn fill(&self, buf: &mut [u8]) -> Result<(), RngError> {
        getrandom::fill(buf).map_err(|e| RngError(e.to_string()))
    }
}

/// `reqwest` as the `Fetch` port. Redirects are not followed (a redirect is an answer like any other), and a request
/// that takes longer than ten seconds is an error, so a provider that hangs cannot hold a login open.
pub struct ReqwestFetch(reqwest::Client);

impl ReqwestFetch {
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .expect("a reqwest client with rustls");
        Self(client)
    }
}

impl Default for ReqwestFetch {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Fetch for ReqwestFetch {
    async fn send(&self, req: FetchRequest) -> Result<FetchResponse, FetchError> {
        let e = |x: reqwest::Error| FetchError(x.to_string());
        let method = reqwest::Method::from_bytes(req.method.as_bytes()).map_err(|x| FetchError(x.to_string()))?;
        let mut b = self.0.request(method, &req.url).body(req.body);
        for (k, v) in &req.headers {
            b = b.header(k, v);
        }
        let res = b.send().await.map_err(e)?;
        let status = res.status().as_u16();
        let headers = res
            .headers()
            .iter()
            .map(|(k, v)| (k.to_string(), String::from_utf8_lossy(v.as_bytes()).into_owned()))
            .collect();
        let body = res.bytes().await.map_err(e)?.to_vec();
        Ok(FetchResponse { status, headers, body })
    }
}

pub mod pg;

/// `Background` on the tokio runtime.
pub struct TokioBackground;

impl Background for TokioBackground {
    fn spawn(&self, fut: BoxFuture<()>) {
        tokio::spawn(fut);
    }
}
