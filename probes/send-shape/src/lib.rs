//! One port, one axum handler. The only thing that varies is how the port trait is declared.
//!
//!   (default)    #[async_trait]            futures are Send
//!   unsend       #[async_trait(?Send)]     futures are not Send
//!   cfg-switch   ?Send on wasm32 only      futures are Send natively, not Send on wasm32
//!
//! axum requires every handler future to be `Send` on EVERY target it is compiled for, because
//! `Router` stores `Box<dyn ... + Send>`. So if the HTTP layer is shared by both composition
//! roots (it is: ARCHITECTURE 2a `thrust-api-core`), the port futures must be Send on wasm32 too.

use std::sync::Arc;

use axum::routing::get;
use axum::{Extension, Router};

#[cfg(feature = "unsend")]
#[async_trait::async_trait(?Send)]
pub trait Port: Send + Sync {
    async fn hit(&self) -> u32;
}

#[cfg(all(feature = "cfg-switch", not(feature = "unsend")))]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
pub trait Port: Send + Sync {
    async fn hit(&self) -> u32;
}

#[cfg(not(any(feature = "unsend", feature = "cfg-switch")))]
#[async_trait::async_trait]
pub trait Port: Send + Sync {
    async fn hit(&self) -> u32;
}

async fn handler(Extension(p): Extension<Arc<dyn Port>>) -> String {
    p.hit().await.to_string()
}

pub fn router() -> Router {
    Router::new().route("/", get(handler))
}
