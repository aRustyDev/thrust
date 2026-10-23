//! `AppDo`: the instance-level Durable Object of cell `worker-do` (spike 0002, issue #37; ADR-0040 Option 5).
//!
//! ONE object for the whole instance (single org, ADR-0003) holds the SQLite database. The `fetch` Worker forwards
//! every API request to it, the queue consumer and the cron handler forward through it too (they need the database),
//! and the shared axum router and the use-cases run inside it over the SQLite adapter (`thrust-adapters-sqlite`) on
//! the object's own `SqlStorage`.
//!
//! * The `App` is built once, in `new`: unlike the Hyperdrive socket, nothing the `durable` set holds belongs to one
//!   request, so there is no per-request rebuild.
//! * Migrations run in `new`, inside `blockConcurrencyWhile` (the docs reserve it for exactly this): no request is
//!   delivered until they are applied, and a migration that fails resets the object, so every request fails until the
//!   file is fixed (`research/0011`, migration story).
//! * `Background` is `State::wait_until`.
//! * The object is created with the location hint `enam`; only the first `get()` honours it and an object does not
//!   move afterwards, so the first deploy fixes its region.
//!
//! `/_do/*` is the Worker-to-object protocol and never reachable from outside: the front Worker answers 404 to a client
//! path that starts with it.

use std::sync::Arc;

use serde_json::json;
use thrust_adapters_cloudflare::*;
use thrust_adapters_sqlite::control::{commit_atomicity_case, TARGETS};
use thrust_adapters_sqlite::fault::{FaultSql, Unatomic};
use thrust_adapters_sqlite::{Sql, SqliteDatabase};
use thrust_api::ApiConfig;
use thrust_contracts::*;
use thrust_core::{ActivityConsumer, App, Permission, Ports, SystemScope};
use thrust_ports::*;
use worker::worker_sys::web_sys;
use worker::*;

use crate::{api_config, core_config};

/// The Durable Object binding and the one name the whole instance lives under.
pub const BINDING: &str = "APP";
pub const INSTANCE: &str = "instance";
/// Eastern North America, next to the origin region the spike measures from.
pub const LOCATION_HINT: &str = "enam";

/// `Background` on the object's `waitUntil`.
struct StateBackground(State);

// SAFETY: an isolate is single-threaded and this never leaves it (the argument of `WaitUntilBackground`).
unsafe impl Send for StateBackground {}
unsafe impl Sync for StateBackground {}

impl Background for StateBackground {
    fn spawn(&self, fut: BoxFuture<()>) {
        self.0.wait_until(fut);
    }
}

/// The stub of the one object, with the location hint.
pub fn stub(env: &Env) -> Result<Stub> {
    env.durable_object(BINDING)?.get_by_name_with_location_hint(INSTANCE, LOCATION_HINT)
}

struct Built {
    app: App,
    sql: Arc<DoSql>,
    env: Env,
}

#[durable_object]
pub struct AppDo {
    built: std::result::Result<Built, String>,
}

impl DurableObject for AppDo {
    fn new(state: State, env: Env) -> Self {
        let sql = Arc::new(DoSql::new(state.storage()));
        let db = Arc::new(SqliteDatabase::new(sql.clone()));

        // Migrations: no event is delivered to the object until this completes. `Err` resets the object.
        let migrating = db.clone();
        let _migrated = state.block_concurrency_while(async move {
            migrating.migrate().map(|_| ()).map_err(|e| Error::RustError(format!("migration failed: {e}")))
        });

        let built = (|| -> Result<Built> {
            let ports = Ports {
                db,
                queue: Arc::new(CfQueue(env.queue("EVENTS")?)),
                blobs: Arc::new(R2BlobStore(env.bucket("BLOBS")?)),
                limiter: Arc::new(DoRateLimiter(env.durable_object("RATE_LIMITER")?)),
                clock: Arc::new(JsClock),
                ids: Arc::new(JsIdGen),
                background: Arc::new(StateBackground(state)),
            };
            // This composition root mints the proof for the relay, sweeper, consumer and seed.
            let app = App::new(ports, core_config(&env), SystemScope::mint());
            Ok(Built { app, sql, env })
        })()
        .map_err(|e| e.to_string());
        Self { built }
    }

    async fn fetch(&self, req: Request) -> Result<Response> {
        let built = match &self.built {
            Ok(b) => b,
            Err(e) => return Response::error(format!("the durable adapter set could not be built: {e}"), 500),
        };
        let path = req.path();
        if let Some(rest) = path.strip_prefix("/_do/") {
            let rest = rest.to_string();
            return self.internal(built, &rest, req).await;
        }
        let api: ApiConfig = api_config(&built.env, "durable");
        let web: web_sys::Request = req.try_into()?;
        let resp = crate::serve(web, built.app.clone(), api).await?;
        Ok(resp.into())
    }
}

impl AppDo {
    /// The Worker-to-object protocol.
    async fn internal(&self, b: &Built, name: &str, mut req: Request) -> Result<Response> {
        match name {
            // A queue message, forwarded by the front Worker's `queue` handler. 200 means ack.
            "consume" => {
                let envelope: Envelope = serde_json::from_str(&req.text().await?)
                    .map_err(|e| Error::RustError(format!("undecodable envelope: {e}")))?;
                match ActivityConsumer(b.app.clone()).handle(&envelope).await {
                    Ok(()) => Response::ok("ok"),
                    Err(e) => Response::error(e, 500),
                }
            }
            // The cron handler's sweep.
            "sweep" => match thrust_core::sweep_outbox(&b.app, 100).await {
                Ok(rep) => Response::from_json(&rep),
                Err(e) => Response::error(e.to_string(), 500),
            },
            // The R1 control on the real `SqlStorage`: test routes and the admin token are checked by the front.
            "tx-control" => {
                let naive = req.url()?.query_pairs().any(|(k, v)| k == "mode" && v == "naive");
                Response::from_json(&self.tx_control(b, naive).await?)
            }
            other => Response::error(format!("unknown object route '{other}'"), 404),
        }
    }

    /// Spike 0002 risk R1, on the real runtime: fail every statement a `Tx::commit` issues, in turn, and check that the
    /// database is exactly as it was and that the same transaction then commits. `naive` runs the same scenario with
    /// `atomically` replaced by no transaction at all (`Unatomic`), which MUST report violations: it is the control that
    /// shows the scenario can tell a transaction from a script.
    async fn tx_control(&self, b: &Built, naive: bool) -> Result<serde_json::Value> {
        let inner: Arc<dyn Sql> = if naive { Arc::new(Unatomic(b.sql.clone())) } else { b.sql.clone() };
        let fault = Arc::new(FaultSql::new(inner));
        let db = Arc::new(SqliteDatabase::new(fault.clone()));
        let ports = Ports { db: db.clone(), ..b.app.ports.clone() };
        let app = App::new(ports, b.app.cfg.clone(), SystemScope::mint());

        // A workspace, project and member of its own, put in place and authorised through the real seam.
        let ids = &app.ports.ids;
        let (ws, project, user) = (WorkspaceId(ids.new_uuid()), ProjectId(ids.new_uuid()), UserId(ids.new_uuid()));
        let slug = WorkspaceSlug::new(format!("tx-control-{}", ws.0.simple()));
        let fixture = SeedFixture {
            workspaces: vec![SeedWorkspace { id: ws, slug: slug.clone() }],
            users: vec![user],
            workspace_members: vec![SeedWorkspaceMember { workspace_id: ws, user_id: user }],
            projects: vec![SeedProject { id: project, workspace_id: ws, name: "tx-control".into() }],
            memberships: vec![SeedMembership { project_id: project, workspace_id: ws, user_id: user, role: Role::Member }],
            sessions: vec![],
        };
        thrust_core::seed(&app, &fixture).await.map_err(|e| Error::RustError(e.to_string()))?;
        let scope = thrust_core::authorize(&app, &Principal { user_id: user }, &slug, project, Permission::CreateWorkItem)
            .await
            .map_err(|e| Error::RustError(e.to_string()))?;

        let (mut cases, mut fired, mut violations) = (0usize, 0usize, Vec::<String>::new());
        let mut next_id = || app.ports.ids.new_uuid();
        for creates in 1..=3usize {
            for target in TARGETS {
                for nth in 0..3usize {
                    cases += 1;
                    let now = app.ports.clock.now();
                    match commit_atomicity_case(&db, &fault, &scope, &mut next_id, now, creates, target, nth).await {
                        Ok(true) => fired += 1,
                        Ok(false) => {}
                        Err(e) => violations.push(e),
                    }
                }
            }
        }
        Ok(json!({ "mode": if naive { "naive" } else { "atomic" }, "cases": cases, "fired": fired, "violations": violations }))
    }
}
