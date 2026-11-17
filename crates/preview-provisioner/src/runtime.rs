use futures_util::StreamExt;
use thrust_adapters_cloudflare::HyperdriveConnector;
use thrust_adapters_postgres::Connector;
use tokio_postgres::types::Type;
use worker::{event, Context, Env, Request, Response, Result};

use crate::{password, route, Route};

#[event(fetch)]
async fn fetch(mut req: Request, env: Env, _ctx: Context) -> Result<Response> {
    let Some(route) = route(&req.method().to_string(), &req.path()) else {
        return Ok(Response::empty()?.with_status(404));
    };
    let secret = if matches!(route, Route::Provision(_)) {
        let mut stream = req.stream()?;
        let mut body = Vec::new();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            if body.len().saturating_add(chunk.len()) > 256 {
                return Ok(Response::empty()?.with_status(413));
            }
            body.extend_from_slice(&chunk);
        }
        match password(&body) {
            Some(value) => Some(value),
            None => return Ok(Response::empty()?.with_status(400)),
        }
    } else {
        None
    };
    let result = async {
        let connector = HyperdriveConnector::new(env.hyperdrive("HYPERDRIVE").map_err(|_| ())?);
        let connection = connector.checkout(true).await.map_err(|_| ())?;
        let client = connection.client();
        match route {
            Route::Inventory => {
                let rows = client.query_typed("SELECT number, schema_exists, login_role, search_path, pending FROM preview_admin.inventory()", &[]).await.map_err(|_| ())?;
                let mut entries = Vec::with_capacity(rows.len());
                for row in rows {
                    let number: i32 = row.try_get(0).map_err(|_| ())?;
                    let schema: bool = row.try_get(1).map_err(|_| ())?;
                    let login_role: bool = row.try_get(2).map_err(|_| ())?;
                    let search_path: bool = row.try_get(3).map_err(|_| ())?;
                    let pending: bool = row.try_get(4).map_err(|_| ())?;
                    entries.push(serde_json::json!({"number": number, "schema": schema, "login_role": login_role, "search_path": search_path, "pending": pending}));
                }
                Response::from_json(&entries).map_err(|_| ())
            }
            Route::Provision(number) => {
                let secret = secret.as_deref().ok_or(())?;
                client.query_typed("SELECT preview_admin.provision($1, $2)", &[(&number, Type::INT4), (&secret, Type::TEXT)]).await.map_err(|_| ())?;
                Response::empty().map_err(|_| ())
            }
            Route::Deprovision(number) => {
                client.query_typed("SELECT preview_admin.deprovision($1)", &[(&number, Type::INT4)]).await.map_err(|_| ())?;
                Response::empty().map_err(|_| ())
            }
            Route::Acknowledge(number) => {
                client.query_typed("SELECT preview_admin.acknowledge($1)", &[(&number, Type::INT4)]).await.map_err(|_| ())?;
                Response::empty().map_err(|_| ())
            }
        }
    }.await;
    match result {
        Ok(response) => Ok(response),
        Err(()) => Ok(Response::empty()?.with_status(503)),
    }
}
