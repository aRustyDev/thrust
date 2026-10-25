//! Shared helpers for the compat parity tests: a world built from the memory adapters (the reference implementation of
//! every port), the in-process driver, the projection of what the pin answered onto what Thrust answers, and the replay
//! of a recorded journey.
#![allow(dead_code)]

pub mod normalise;

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Method, Request};
use axum::{Extension, Router};
use serde_json::{json, Map, Value};
use thrust_adapters_memory::*;
use thrust_contracts::chrono::DateTime;
use thrust_contracts::*;
use thrust_core::*;
use thrust_ports::*;
use tower::ServiceExt;

/// Proptest config for integration tests (from `.agents/skills/rust-test-logic/references/examples/`, as in thrust-core).
/// * `PROPTEST_CASES` raises or lowers the case count without editing tests.
/// * Counterexamples are saved beside the test file (`tests/<file>.proptest-regressions`).
/// * `PROPTEST_DISABLE_FAILURE_PERSISTENCE=1` turns saving off (use it for mutation runs).
pub fn proptest_config(default_cases: u32) -> proptest::test_runner::Config {
    use proptest::test_runner::{Config, FailurePersistence, FileFailurePersistence};
    let cases = std::env::var("PROPTEST_CASES").ok().and_then(|v| v.parse().ok()).unwrap_or(default_cases);
    let failure_persistence: Option<Box<dyn FailurePersistence>> =
        if std::env::var_os("PROPTEST_DISABLE_FAILURE_PERSISTENCE").is_some() {
            None
        } else {
            Some(Box::new(FileFailurePersistence::WithSource("proptest-regressions")))
        };
    Config { cases, failure_persistence, ..Config::default() }
}

/// Time is injected (AGENTS.md testing rule 6): every call returns the same instant.
pub struct FixedClock(pub Timestamp);
impl Clock for FixedClock {
    fn now(&self) -> Timestamp {
        self.0
    }
}

/// Deterministic, unique ids.
#[derive(Default)]
pub struct CounterIds(AtomicU64);
impl IdGen for CounterIds {
    fn new_uuid(&self) -> Uuid {
        Uuid::from_u128((1u128 << 64) | u128::from(self.0.fetch_add(1, Ordering::Relaxed)))
    }
}

/// A deterministic stream of distinct bytes: every call returns different bytes, so two tokens made one after the other
/// differ and a run can be repeated.
#[derive(Default)]
pub struct CounterRng(AtomicU64);
impl Rng for CounterRng {
    fn fill(&self, buf: &mut [u8]) -> Result<(), RngError> {
        for chunk in buf.chunks_mut(8) {
            // splitmix64 over a counter
            let mut z = self.0.fetch_add(0x9E37_79B9_7F4A_7C15, Ordering::Relaxed).wrapping_add(0x9E37_79B9_7F4A_7C15);
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^= z >> 31;
            let bytes = z.to_le_bytes();
            chunk.copy_from_slice(&bytes[..chunk.len()]);
        }
        Ok(())
    }
}

/// The default world runs with the outbox fast path off, so nothing should ever be spawned. Fail loudly if it is.
pub struct NoBackground;
impl Background for NoBackground {
    fn spawn(&self, _fut: BoxFuture<()>) {
        panic!("a compat parity test spawned background work; keep CoreConfig::outbox_fastpath = false");
    }
}

/// The compat routes over an empty memory world, with a fixed clock and counting ids. Configured as the composition roots
/// are (CSRF not enforced while the stock frontend exists, Q-0050.8).
pub fn app() -> Router {
    app_with_db().0
}

/// [`app`], and the memory database behind it, for a test that needs rows no route makes (a guest, a second project).
pub fn app_with_db() -> (Router, Arc<MemoryDatabase>) {
    let db = Arc::new(MemoryDatabase::new());
    let background: Arc<dyn Background> = Arc::new(NoBackground);
    let queue: Arc<dyn Queue> = Arc::new(MemoryQueue::new(background.clone(), ConsumerSlot::default()));
    let clock: Arc<dyn Clock> = Arc::new(FixedClock(DateTime::from_timestamp(1_760_000_000, 0).unwrap()));
    let cfg = CoreConfig {
        outbox_fastpath: false,
        csrf_enforce: false,
        // a cheap argon2id: the cost is pinned by a core test, not by these
        argon2: Argon2Params { m_cost_kib: 32, t_cost: 1, p_cost: 1 },
        ..CoreConfig::default()
    };
    let hasher = Arc::new(Argon2idHasher::new(cfg.argon2));
    let app = App::new(
        Ports {
            db: db.clone(),
            queue,
            blobs: Arc::new(MemoryBlobStore::new()),
            limiter: Arc::new(MemoryRateLimiter::new(clock.clone())),
            clock,
            ids: Arc::new(CounterIds::default()),
            background,
            rng: Arc::new(CounterRng::default()),
            hasher,
            oidc: None,
        },
        cfg,
        SystemScope::mint(),
    );
    (thrust_compat_plane::routes().layer(Extension(app)), db)
}

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/parity/fixtures")
}

/// A committed fixture, recorded from the pinned Plane by tools/compat-record/record.py.
pub fn load_fixture(row: &str) -> Value {
    let path = fixtures_dir().join(format!("{row}.json"));
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// The row ids that have a fixture file, sorted.
pub fn fixture_rows() -> Vec<String> {
    let mut rows: Vec<String> = std::fs::read_dir(fixtures_dir())
        .expect("fixtures dir")
        .filter_map(|e| e.ok()?.file_name().to_str()?.strip_suffix(".json").map(str::to_string))
        .collect();
    rows.sort();
    rows
}

// ---------------------------------------------------------------------------------------------------------------------
// The projection. The pin answers its sign-in and sign-up form posts with a 302; Thrust's copy of the auth client posts
// JSON and reads JSON (D-022). The fixtures keep the pin's raw answer, so the recording is the pin's and nothing is edited;
// the comparison projects a recorded exchange onto the Thrust shape, by these rules and no others:
//   request  `{"form": {..}}`  becomes the JSON object `{..}`;
//   response 302 whose Location carries `error_code`
//                              becomes 400 (429 for 5900) with the query's pairs as a JSON object, `error_code` a
//                              number, `next_path` left out (it is the client's own);
//   response 302 otherwise     becomes 200 `{"redirect_path": <Location minus the origin, "/" when empty>}`.
// ---------------------------------------------------------------------------------------------------------------------

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < b.len() && b[i + 1].is_ascii_hexdigit() && b[i + 2].is_ascii_hexdigit() => {
                out.push(u8::from_str_radix(&s[i + 1..i + 3], 16).expect("hex"));
                i += 3;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn query_pairs(q: &str) -> Vec<(String, String)> {
    q.split('&')
        .filter(|p| !p.is_empty())
        .map(|p| {
            let (k, v) = p.split_once('=').unwrap_or((p, ""));
            (percent_decode(k), percent_decode(v))
        })
        .collect()
}

/// A recorded exchange as Thrust is expected to answer it. Anything that is not a form request or a 302 is unchanged.
pub fn project(ex: &Value) -> Value {
    let mut ex = ex.clone();
    if let Some(form) = ex["request"]["body"].get("form").cloned() {
        ex["request"]["body"] = form;
    }
    if ex["response"]["status"] == 302 {
        let loc = ex["response"]["headers"]["location"].as_str().expect("a 302 with a location").to_string();
        let rest = loc.strip_prefix("<base_url>").unwrap_or_else(|| panic!("location {loc:?} must start with <base_url>"));
        let query = rest.split_once('?').map_or("", |(_, q)| q);
        let pairs = query_pairs(query);
        let (status, body) = match pairs.iter().find(|(k, _)| k == "error_code") {
            Some((_, code)) => {
                let n: u64 = code.parse().unwrap_or_else(|_| panic!("error_code {code:?}"));
                let mut m = Map::new();
                for (k, v) in &pairs {
                    match k.as_str() {
                        "next_path" => {}
                        "error_code" => {
                            m.insert(k.clone(), json!(n));
                        }
                        _ => {
                            m.insert(k.clone(), json!(v));
                        }
                    }
                }
                (if n == 5900 { 429 } else { 400 }, Value::Object(m))
            }
            None => (200, json!({"redirect_path": if rest.is_empty() { "/" } else { rest }})),
        };
        ex["response"] = json!({"status": status, "headers": {"content-type": "application/json"}, "body": body});
    }
    ex
}

// ---------------------------------------------------------------------------------------------------------------------
// Driving the router
// ---------------------------------------------------------------------------------------------------------------------

/// The sessions the journey has made so far: the session token of the last successful sign-in or sign-up of each email
/// (trimmed, lower case). A request recorded `as` an email is sent with that email's session, as the recorder's browser
/// sent the session cookie of the user it had signed in.
pub type Jar = std::collections::BTreeMap<String, String>;

pub fn jar_key(email: &str) -> String {
    email.trim().to_lowercase()
}

/// Send one request to `router` and return what it answered, as a raw exchange (the shape the normaliser takes). A header
/// that came more than once (`set-cookie`) is an array. A request with an `as` carries that user's session from `jar`.
pub fn send(router: &Router, name: &Value, req: &Value, jar: &Jar) -> Value {
    let method: Method = req["method"].as_str().expect("method").parse().expect("method");
    let mut builder = Request::builder().method(method).uri(req["path"].as_str().expect("path"));
    if let Some(who) = req.get("as").and_then(Value::as_str) {
        let token = jar
            .get(&jar_key(who))
            .unwrap_or_else(|| panic!("{name}: no session for {who}: an earlier step of the journey must sign them in"));
        builder = builder.header("cookie", format!("__Host-thrust_session={token}"));
    }
    let body = match req.get("body").filter(|b| !b.is_null()) {
        Some(b) if b.get("form").is_some() => panic!("a form body must be projected before it is sent; found one in {name}"),
        Some(b) => {
            builder = builder.header("content-type", "application/json");
            Body::from(b.to_string())
        }
        None => Body::empty(),
    };
    let resp = futures_executor::block_on(router.clone().oneshot(builder.body(body).expect("request"))).expect("infallible");
    let status = resp.status().as_u16();
    let mut headers = Map::new();
    for (k, v) in resp.headers() {
        let v = Value::String(v.to_str().unwrap_or("<non-utf8>").to_string());
        match headers.get_mut(k.as_str()) {
            None => {
                headers.insert(k.as_str().to_string(), v);
            }
            Some(Value::Array(a)) => a.push(v),
            Some(first) => *first = Value::Array(vec![first.clone(), v]),
        }
    }
    let bytes = futures_executor::block_on(axum::body::to_bytes(resp.into_body(), usize::MAX)).expect("body");
    let parsed = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned()))
    };
    json!({
        "name": name,
        "request": req,
        "response": {"status": status, "headers": headers, "body": parsed},
    })
}

/// A recorded request with every `<uuid:N>` in its path and body replaced by this replay's own id (see `Ids::resolve`).
fn resolve_ids(v: &Value, ids: &normalise::Ids) -> Value {
    match v {
        Value::String(s) => Value::String(ids.resolve(s)),
        Value::Array(a) => Value::Array(a.iter().map(|x| resolve_ids(x, ids)).collect()),
        Value::Object(m) => Value::Object(m.iter().map(|(k, x)| (k.clone(), resolve_ids(x, ids))).collect()),
        other => other.clone(),
    }
}

/// The session token a raw response sets (not a cookie that clears it), if it sets one.
pub fn session_token(raw: &Value) -> Option<String> {
    let h = &raw["response"]["headers"]["set-cookie"];
    let all: Vec<&str> = match h {
        Value::String(s) => vec![s.as_str()],
        Value::Array(a) => a.iter().filter_map(Value::as_str).collect(),
        _ => vec![],
    };
    all.iter().find_map(|c| {
        let value = c.strip_prefix("__Host-thrust_session=")?.split(';').next()?;
        (!value.is_empty() && !c.contains("Max-Age=0")).then(|| value.to_string())
    })
}

/// Replay every exchange of `fixture` against `router`, normalise what comes back and compare it with the recording as
/// projected onto Thrust's shape. Returns one message per diverging exchange (empty means parity). A sign-in or sign-up
/// that the recording says worked must also set the session cookie, and one that failed must not: cookies are transport
/// the normaliser drops, so they are checked here.
pub fn divergences(router: &Router, fixture: &Value) -> Vec<String> {
    divergences_with(router, fixture, &mut Jar::new())
}

/// [`divergences`] on a journey: `jar` holds the sessions the earlier steps made and gets the ones this fixture makes.
pub fn divergences_with(router: &Router, fixture: &Value, jar: &mut Jar) -> Vec<String> {
    let rules = normalise::Rules::load();
    let mut ids = normalise::Ids::new(&rules);
    // the ids an earlier fixture made that this one's requests name: the recorder numbered them first, in this order
    for name in fixture["uses"].as_array().into_iter().flatten().map(|n| n.as_str().expect("a name")) {
        let id = jar.get(&format!("bind:{name}")).unwrap_or_else(|| panic!("the fixture uses {name:?}, which no earlier fixture binds"));
        ids.preseed(id);
    }
    let mut out = Vec::new();
    for recorded in fixture["exchanges"].as_array().expect("exchanges") {
        let want = project(recorded);
        let raw = send(router, &want["name"], &resolve_ids(&want["request"], &ids), jar);
        // an id this exchange made that later fixtures use (`binds`: {name: {"exchange": .., "path": [..]}})
        for (name, bind) in fixture["binds"].as_object().into_iter().flatten() {
            if bind["exchange"] == want["name"] {
                let mut at = &raw["response"]["body"];
                for key in bind["path"].as_array().expect("a path") {
                    at = &at[key.as_str().expect("a key")];
                }
                if let Some(id) = at.as_str() {
                    jar.insert(format!("bind:{name}"), id.to_string());
                }
            }
        }
        let got = normalise::exchange(&rules, &mut ids, &raw);
        let mut lines = Vec::new();
        if got != want {
            diff_paths(&want, &got, "", &mut lines);
        }
        let signs_in = matches!(want["request"]["path"].as_str(), Some("/auth/sign-in/" | "/auth/sign-up/"));
        if signs_in {
            let expected = want["response"]["status"] == 200;
            let token = session_token(&raw);
            if token.is_some() != expected {
                lines.push(format!(
                    "  session cookie: the recording says it {} be set, the answer {} it",
                    if expected { "must" } else { "must not" },
                    if expected { "did not set" } else { "set" }
                ));
            }
            // the user signed in or up now holds this session for the rest of the journey
            if let (Some(token), Some(email)) = (token, want["request"]["body"]["email"].as_str()) {
                jar.insert(jar_key(email), token);
            }
        }
        if !lines.is_empty() {
            out.push(format!("exchange {}:\n{}", want["name"], lines.join("\n")));
        }
    }
    out
}

/// Replay the journey up to `target` on one world, so `target`'s requests find the state the recording had (a user made
/// by an earlier step), then replay `target` itself, after `edit` has had its way with the recording. Only `target`'s
/// divergences are returned: each earlier step has a test of its own.
pub fn journey(rows: &[&str], target: &str, edit: impl FnOnce(&mut Value)) -> Vec<String> {
    let router = app();
    let mut jar = Jar::new();
    for row in rows.iter().take_while(|r| **r != target) {
        let _ = divergences_with(&router, &load_fixture(row), &mut jar);
    }
    let mut fixture = load_fixture(target);
    edit(&mut fixture);
    divergences_with(&router, &fixture, &mut jar)
}

/// The JSON paths at which `recorded` and `answered` differ, one line each.
fn diff_paths(recorded: &Value, answered: &Value, path: &str, out: &mut Vec<String>) {
    match (recorded, answered) {
        (Value::Object(r), Value::Object(a)) => {
            let mut keys: Vec<&String> = r.keys().chain(a.keys()).collect();
            keys.sort();
            keys.dedup();
            for k in keys {
                let p = if path.is_empty() { k.clone() } else { format!("{path}.{k}") };
                match (r.get(k), a.get(k)) {
                    (Some(rv), Some(av)) => diff_paths(rv, av, &p, out),
                    (Some(rv), None) => out.push(format!("  {p}: recorded {rv}, answer has no such field")),
                    (None, Some(av)) => out.push(format!("  {p}: answer has {av}, the recording has no such field")),
                    (None, None) => unreachable!(),
                }
            }
        }
        (Value::Array(r), Value::Array(a)) if r.len() == a.len() => {
            for (i, (rv, av)) in r.iter().zip(a).enumerate() {
                diff_paths(rv, av, &format!("{path}[{i}]"), out);
            }
        }
        _ if recorded != answered => out.push(format!("  {path}: recorded {recorded}, answered {answered}")),
        _ => {}
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// A world of signed-up users and the rows no route makes, for the tests that state a rule over what the recorded journey
// never has (a guest, a second project, a deactivated member).
// ---------------------------------------------------------------------------------------------------------------------

pub const STRONG: &str = "Workspaces-Sample-Pa55word-Zebra-81";

/// A world with a signed-up account for each email, and the jar that holds their sessions.
pub fn world(emails: &[&str]) -> (Router, Arc<MemoryDatabase>, Jar) {
    let (router, db) = app_with_db();
    let mut jar = Jar::new();
    for email in emails {
        let raw = send(&router, &json!("sign-up"), &json!({"method": "POST", "path": "/auth/sign-up/", "body": {"email": email, "password": STRONG}}), &jar);
        assert_eq!(raw["response"]["status"], 200, "{raw}");
        jar.insert(jar_key(email), session_token(&raw).expect("a session"));
    }
    (router, db, jar)
}

/// One request as `who` (an email in the jar, or `""` for no session); the response (`status`, `headers`, `body`).
pub fn call(router: &Router, jar: &Jar, who: &str, method: &str, path: &str, body: Value) -> Value {
    let mut req = json!({"method": method, "path": path});
    if !who.is_empty() {
        req["as"] = json!(who);
    }
    if !body.is_null() {
        req["body"] = body;
    }
    send(router, &json!("call"), &req, jar)["response"].clone()
}

pub fn user_id(db: &MemoryDatabase, email: &str) -> UserId {
    futures_executor::block_on(db.user_by_email(email)).unwrap().expect("the account exists").id
}

pub fn at(n: i64) -> Timestamp {
    DateTime::from_timestamp(1_760_000_000 + n, 0).unwrap()
}

/// A workspace whose only member is `user`, with `role`.
pub fn make_workspace(db: &MemoryDatabase, id: u128, slug: &str, user: UserId, role: Role) -> WorkspaceId {
    let id = WorkspaceId(Uuid::from_u128(id));
    let workspace = Workspace {
        id,
        slug: WorkspaceSlug::new(slug),
        name: slug.to_owned(),
        owner_id: Some(user),
        organization_size: None,
        timezone: "UTC".into(),
        background_color: "#808080".into(),
        created_at: at(0),
        updated_at: at(0),
    };
    let owner = WorkspaceMember {
        id: Uuid::from_u128(id.0.as_u128() ^ 0xFFFF),
        workspace_id: id,
        user_id: user,
        role,
        company_role: None,
        is_active: true,
        props: json!({}),
        created_at: at(0),
        updated_at: at(0),
    };
    futures_executor::block_on(db.insert_workspace(&NewWorkspace { workspace, owner })).unwrap();
    id
}

pub fn seed(db: &MemoryDatabase, f: SeedFixture) {
    futures_executor::block_on(db.seed(&SystemScope::mint(), &f)).unwrap();
}

/// Plane's role numbers: Guest 5, Member 15, Admin 20.
pub fn role_number(r: Role) -> u8 {
    match r {
        Role::Guest => 5,
        Role::Member => 15,
        Role::Admin => 20,
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// The cast: a workspace with a caller of every kind and one project, shared by the tests of the routes that name a project.
// ---------------------------------------------------------------------------------------------------------------------

/// Who asks. The workspace's roles are admin, member and guest; the project's are admin, member and guest, and those three are
/// members (guest among them) of the workspace; the other three are in the workspace and in no project.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Caller {
    Anonymous,
    /// Signed in, in no workspace of this one.
    Stranger,
    WsAdmin,
    WsMember,
    WsGuest,
    ProjectAdmin,
    ProjectMember,
    ProjectGuest,
}

pub const CALLERS: [Caller; 8] = [
    Caller::Anonymous,
    Caller::Stranger,
    Caller::WsAdmin,
    Caller::WsMember,
    Caller::WsGuest,
    Caller::ProjectAdmin,
    Caller::ProjectMember,
    Caller::ProjectGuest,
];

impl Caller {
    pub fn email(self) -> &'static str {
        match self {
            Caller::Anonymous => "",
            Caller::Stranger => "stranger@example.test",
            Caller::WsAdmin => "ws-admin@example.test",
            Caller::WsMember => "ws-member@example.test",
            Caller::WsGuest => "ws-guest@example.test",
            Caller::ProjectAdmin => "p-admin@example.test",
            Caller::ProjectMember => "p-member@example.test",
            Caller::ProjectGuest => "p-guest@example.test",
        }
    }
    pub fn in_workspace(self) -> bool {
        !matches!(self, Caller::Anonymous | Caller::Stranger)
    }
    pub fn in_project(self) -> bool {
        matches!(self, Caller::ProjectAdmin | Caller::ProjectMember | Caller::ProjectGuest)
    }
    /// The role the workspace gives them: the project's people are members of the workspace, the guest a guest.
    pub fn workspace_role(self) -> Option<Role> {
        match self {
            Caller::Anonymous | Caller::Stranger => None,
            Caller::WsAdmin => Some(Role::Admin),
            Caller::WsGuest | Caller::ProjectGuest => Some(Role::Guest),
            Caller::WsMember | Caller::ProjectAdmin | Caller::ProjectMember => Some(Role::Member),
        }
    }
}

pub const WS: u128 = 0xA0;
pub const PROJECT: u128 = 0xA1;
pub const SLUG: &str = "cast";

pub struct Cast {
    pub router: axum::Router,
    pub db: std::sync::Arc<thrust_adapters_memory::MemoryDatabase>,
    pub jar: Jar,
}

impl Cast {
    pub fn call(&self, who: Caller, method: &str, path: &str, body: Value) -> Value {
        call(&self.router, &self.jar, who.email(), method, path, body)
    }
    pub fn user(&self, who: Caller) -> UserId {
        user_id(&self.db, who.email())
    }
    pub fn workspace_scope(&self) -> WorkspaceScope {
        WorkspaceScope::mint(WorkspaceId(Uuid::from_u128(WS)), self.user(Caller::WsAdmin), Role::Admin)
    }
    /// The seeded project as the table holds it, and a way to save a changed copy.
    pub fn project_row(&self) -> ProjectRow {
        futures_executor::block_on(self.db.project_detail(&self.workspace_scope(), ProjectId(Uuid::from_u128(PROJECT)))).unwrap().unwrap().listing.project
    }
    pub fn save(&self, row: &ProjectRow) {
        assert!(futures_executor::block_on(self.db.update_project(&self.workspace_scope(), row)).unwrap());
    }
}

/// A workspace `cast` with a caller of every kind, and one project (`PROJECT`, public, the key `PRJ`) that the three project callers
/// are in with their three roles.
pub fn cast() -> Cast {
    let emails: Vec<&str> = CALLERS.iter().filter(|c| **c != Caller::Anonymous).map(|c| c.email()).collect();
    let (router, db, jar) = world(&emails);
    let c = Cast { router, db, jar };
    let ws = WorkspaceId(Uuid::from_u128(WS));
    let project = ProjectId(Uuid::from_u128(PROJECT));
    make_workspace(&c.db, WS, SLUG, c.user(Caller::WsAdmin), Role::Admin);
    seed(
        &c.db,
        SeedFixture {
            workspace_roles: CALLERS
                .iter()
                .filter(|x| x.in_workspace() && **x != Caller::WsAdmin)
                .map(|x| SeedWorkspaceRole { workspace_id: ws, user_id: c.user(*x), role: x.workspace_role().unwrap() })
                .collect(),
            projects: vec![SeedProject { id: project, workspace_id: ws, name: "Cast Project".into() }],
            project_details: vec![SeedProjectDetails { identifier: Some("PRJ".into()), created_at: Some(at(1)), ..SeedProjectDetails::of(project) }],
            // a default state and the intake state: the project has been made as the pin makes one, in short
            states: [(0x5001, "Backlog", 15000.0, StateGroup::Backlog, true), (0x5002, "Triage", 65000.0, StateGroup::Triage, false)]
                .iter()
                .map(|(n, name, sequence, group, is_default)| SeedState {
                    id: Uuid::from_u128(*n),
                    project_id: project,
                    workspace_id: ws,
                    name: (*name).into(),
                    color: "#000000".into(),
                    sequence: *sequence,
                    group: *group,
                    is_default: *is_default,
                })
                .collect(),
            memberships: [(Caller::ProjectAdmin, Role::Admin), (Caller::ProjectMember, Role::Member), (Caller::ProjectGuest, Role::Guest)]
                .iter()
                .map(|(x, role)| SeedMembership { project_id: project, workspace_id: ws, user_id: c.user(*x), role: *role })
                .collect(),
            ..SeedFixture::default()
        },
    );
    c
}

pub fn projects_path() -> String {
    format!("/api/workspaces/{SLUG}/projects/")
}

/// The seeded project's route, `tail` after it.
pub fn project_path(tail: &str) -> String {
    format!("{}{}/{tail}", projects_path(), Uuid::from_u128(PROJECT))
}

