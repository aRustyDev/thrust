// Template: the harness crate's src/lib.rs. Example names: crate `api` (AppState, router); rename to yours. [re-run]
//! E2E harness: serves the built web UI and the real API router, in-process, on an ephemeral port.

use std::path::PathBuf;
use tower_http::services::{ServeDir, ServeFile};

pub struct TestApp {
    pub base_url: String,
    pub state: api::AppState,
    server: tokio::task::JoinHandle<()>,
}

impl TestApp {
    pub async fn start() -> Self {
        let dist = PathBuf::from(
            std::env::var("E2E_WEB_DIST")
                .expect("E2E_WEB_DIST must point at the built web UI (see .cargo/config.toml [env])"),
        );
        assert!(
            dist.join("index.html").exists(),
            "web UI not built at {}: run `dx build --platform web` first",
            dist.display()
        );
        let state = api::AppState::default();
        // Unknown paths fall back to index.html so client-side routes deep-link correctly.
        let ui = ServeDir::new(&dist).fallback(ServeFile::new(dist.join("index.html")));
        let app = api::router(state.clone()).fallback_service(ui);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        Self { base_url: format!("http://{addr}"), state, server }
    }
}

impl Drop for TestApp {
    fn drop(&mut self) {
        self.server.abort();
    }
}
