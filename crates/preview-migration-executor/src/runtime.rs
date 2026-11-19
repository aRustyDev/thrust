use futures_util::StreamExt;
use thrust_adapters_cloudflare::HyperdriveConnector;
use thrust_adapters_postgres::{migrate_preview_with, Connector, MigrateOutcome};
use worker::{event, Context, Env, Request, Response, Result};

use crate::{append_bounded, is_route, Config};

#[event(fetch)]
async fn fetch(mut req: Request, env: Env, _ctx: Context) -> Result<Response> {
    if !is_route(req.method().as_ref(), &req.path()) {
        return Ok(Response::empty()?.with_status(404));
    }
    let config = (|| {
        Config::parse(
            &env.var("EXECUTOR_ENABLED").ok()?.to_string(),
            &env.var("PREVIEW_NUMBER").ok()?.to_string(),
            &env.var("PREVIEW_COMMIT").ok()?.to_string(),
            &env.var("PREVIEW_DATABASE").ok()?.to_string(),
        )
    })();
    let Some(config) = config else {
        return Ok(Response::empty()?.with_status(503));
    };
    let hd = match env.hyperdrive("HYPERDRIVE") {
        Ok(hd) if config.matches_binding(&hd.user(), &hd.database()) => hd,
        _ => return Ok(Response::empty()?.with_status(503)),
    };
    let mut body = Vec::new();
    let mut stream = match req.stream() {
        Ok(stream) => stream,
        Err(_) => return Ok(Response::empty()?.with_status(400)),
    };
    while let Some(chunk) = stream.next().await {
        let chunk = match chunk {
            Ok(chunk) => chunk,
            Err(_) => return Ok(Response::empty()?.with_status(400)),
        };
        if !append_bounded(&mut body, &chunk) {
            return Ok(Response::empty()?.with_status(413));
        }
    }
    let Some(artifact) = config.artifact(&body) else {
        return Ok(Response::empty()?.with_status(400));
    };
    drop(body);
    // A fresh exclusive handle is dropped on all outcomes, including rollback or I/O failure.
    let connector = HyperdriveConnector::new(hd);
    let result = match connector.checkout(true).await {
        Ok(connection) => migrate_preview_with(connection.client(), config.number, &config.database, &artifact).await,
        Err(_) => return Ok(Response::empty()?.with_status(503)),
    };
    match result {
        Ok(MigrateOutcome::Baseline { .. } | MigrateOutcome::Applied { .. } | MigrateOutcome::Current) => {
            Ok(Response::empty()?.with_status(204))
        }
        Ok(MigrateOutcome::Older { .. }) => Ok(Response::empty()?.with_status(409)),
        Err(_) => Ok(Response::empty()?.with_status(503)),
    }
}
