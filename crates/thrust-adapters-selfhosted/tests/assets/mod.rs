//! Issue #44, migration 0016 and the asset methods of `Database` (ADR-0047) against real Postgres: the columns of the `assets` table
//! and the constraints that keep it well formed, the migration run again, a grant spent by exactly one of many callers, and a
//! model: random sequences of the five methods, each answer checked against a reference written from the port's documentation
//! (not from either adapter), and every row of every asset read back after every step.
//!
//! The spec, written out (`thrust-ports`, `Database`):
//!
//! * `asset` is the live asset of the scope's workspace, and nothing for another workspace's or a deleted one;
//! * `mark_asset_uploaded` makes a live asset of the scope's workspace `Uploaded` at the time given, drops its grant, and leaves one
//!   that is uploaded already as it is (`true` for both); `false` for any other;
//! * `attach_project_cover` gives the project the caller's own live uploads of the scope's workspace that belong to no project or to
//!   this one, among those named, and makes the oldest of them its cover, stamping the project with the caller and the time; with
//!   none of them it answers `None` and writes nothing;
//! * `take_upload_grant` answers a live, pending asset whose grant holds the token and has not expired at `now` (an expiry equal to
//!   `now` has), and spends the grant, so the same call answers nothing after; and it answers nothing for any other asset or token;
//! * `uploaded_asset` is a live, uploaded asset by its id alone.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};

use proptest::prelude::*;
use proptest::strategy::ValueTree;
use proptest::test_runner::{Config, TestCaseError, TestRunner};

use super::*;

const BASE: i64 = 1_800_000_000;

fn at(secs: i64) -> Timestamp {
    Utc.timestamp_opt(BASE + secs, 0).unwrap()
}

/// The columns of the table, written out: a column added later has to be argued for.
const COLUMNS: [&str; 15] = [
    "id",
    "workspace_id",
    "created_by",
    "entity_type",
    "project_id",
    "name",
    "content_type",
    "size",
    "object_key",
    "state",
    "upload_token_hash",
    "upload_expires_at",
    "created_at",
    "uploaded_at",
    "deleted_at",
];

fn pending(ws: WorkspaceId, by: UserId, n: u8) -> AssetRow {
    let id = AssetId(new_id());
    AssetRow {
        id,
        workspace_id: ws,
        created_by: by,
        entity: AssetEntity::ProjectCover,
        project_id: None,
        name: format!("cover-{n}.png"),
        content_type: "image/png".into(),
        size: 4,
        object_key: format!("{ws}/{}-cover-{n}.png", id.0.simple()),
        state: AssetState::Pending,
        upload: Some(UploadGrant { token_hash: TokenHash([n; 32]), expires_at: at(3600) }),
        created_at: at(i64::from(n)),
        uploaded_at: None,
        deleted_at: None,
    }
}

/// A workspace with an admin and a project the admin is in; the admin's scope, the workspace's.
async fn world(env: &Env, tag: &str) -> (WorkspaceId, UserId, ProjectId, WorkspaceScope) {
    let owner = user_without_login(env).await;
    let w = workspace_for(owner, tag);
    env.db.insert_workspace(&w).await.unwrap();
    let ws = w.workspace.id;
    let scope = WorkspaceScope::mint(ws, owner, Role::Admin);
    let project = project_for(ws, owner, "Assets", "AST");
    let id = project.project.id;
    env.db.insert_project(&scope, &project).await.unwrap();
    (ws, owner, id, scope)
}

#[tokio::test]
async fn the_assets_table_has_the_columns_it_says_and_the_constraints_that_keep_it_well_formed() {
    let env = env().await;
    let (ws, owner, project, scope) = world(&env, "asset-table").await;
    let raw = env.conn.raw().await.unwrap();

    let columns: Vec<String> = raw
        .query(
            "SELECT column_name FROM information_schema.columns WHERE table_schema = 'public' AND table_name = 'assets' \
             ORDER BY ordinal_position",
            &[],
        )
        .await
        .unwrap()
        .iter()
        .map(|r| r.get(0))
        .collect();
    assert_eq!(columns, COLUMNS);

    // a good one goes in, whole
    let a = pending(ws, owner, 1);
    env.db.insert_asset(&scope, &a).await.unwrap();
    assert_eq!(env.db.asset(&scope, a.id).await.unwrap(), Some(a.clone()));
    // the same id again is the primary key
    assert!(matches!(env.db.insert_asset(&scope, &a).await, Err(DbError::Conflict(_))));
    // a row that is not new, or is not the scope's, is refused before it reaches the table
    let stranger = WorkspaceScope::mint(ws, UserId(new_id()), Role::Admin);
    assert!(matches!(env.db.insert_asset(&stranger, &pending(ws, owner, 2)).await, Err(DbError::Backend(_))));
    let uploaded =
        AssetRow { state: AssetState::Uploaded, upload: None, uploaded_at: Some(at(5)), ..pending(ws, owner, 3) };
    assert!(matches!(env.db.insert_asset(&scope, &uploaded).await, Err(DbError::Backend(_))));
    let no_grant = AssetRow { upload: None, ..pending(ws, owner, 4) };
    assert!(matches!(env.db.insert_asset(&scope, &no_grant).await, Err(DbError::Backend(_))));

    // each constraint, by name, on a row that is otherwise good
    let other_ws = world(&env, "asset-table-other").await;
    let id = a.id.0;
    let nobody = new_id();
    let hash31 = vec![0u8; 31];
    let cases: Vec<(&str, &str, Vec<&(dyn tokio_postgres::types::ToSql + Sync)>)> = vec![
        ("assets_entity_type_valid", "UPDATE assets SET entity_type = 'NOPE' WHERE id = $1", vec![&id]),
        ("assets_state_valid", "UPDATE assets SET state = 'deleted' WHERE id = $1", vec![&id]),
        ("assets_size_positive", "UPDATE assets SET size = 0 WHERE id = $1", vec![&id]),
        ("assets_token_hash_len", "UPDATE assets SET upload_token_hash = $2 WHERE id = $1", vec![&id, &hash31]),
        ("assets_grant_whole", "UPDATE assets SET upload_expires_at = NULL WHERE id = $1", vec![&id]),
        ("assets_uploaded_consistent", "UPDATE assets SET state = 'uploaded' WHERE id = $1", vec![&id]),
        ("assets_uploaded_consistent", "UPDATE assets SET uploaded_at = now() WHERE id = $1", vec![&id]),
        ("assets_created_by_fkey", "UPDATE assets SET created_by = $2 WHERE id = $1", vec![&id, &nobody]),
        ("assets_workspace_id_fkey", "UPDATE assets SET workspace_id = $2 WHERE id = $1", vec![&id, &nobody]),
        // a project of the workspace, or none: the composite key looks at both, so a project that does not exist is refused ...
        ("assets_workspace_project_fkey", "UPDATE assets SET project_id = $2 WHERE id = $1", vec![&id, &nobody]),
    ];
    for (constraint, sql, params) in &cases {
        let err = raw.execute(*sql, params).await.expect_err(constraint);
        let got = err.as_db_error().and_then(|d| d.constraint()).unwrap_or("?").to_string();
        assert_eq!(&got, constraint, "{sql}");
    }
    // ... and so is a project that exists in another workspace
    let theirs = other_ws.2 .0;
    let err = raw.execute("UPDATE assets SET project_id = $2 WHERE id = $1", &[&id, &theirs]).await.unwrap_err();
    assert_eq!(err.as_db_error().and_then(|d| d.constraint()), Some("assets_workspace_project_fkey"));
    // and none of that changed the row
    assert_eq!(env.db.asset(&scope, a.id).await.unwrap(), Some(a.clone()));

    // the project's cover is an asset that exists
    let mut row = env.db.project_detail(&scope, project).await.unwrap().unwrap().listing.project;
    row.cover_image_asset = Some(new_id());
    let refused = env.db.update_project(&scope, &row).await;
    assert!(
        matches!(&refused, Err(DbError::Backend(m)) if m.contains("projects_cover_image_asset_fkey")),
        "{refused:?}"
    );
    row.cover_image_asset = Some(a.id.0);
    assert!(env.db.update_project(&scope, &row).await.unwrap());
    let seen = env.db.project_detail(&scope, project).await.unwrap().unwrap().listing.project;
    assert_eq!(seen.cover_image_asset, Some(a.id.0));
}

#[tokio::test]
async fn migration_0016_is_idempotent_and_keeps_its_rows() {
    let env = env().await;
    let (ws, owner, project, scope) = world(&env, "asset-migration").await;
    let a = pending(ws, owner, 1);
    env.db.insert_asset(&scope, &a).await.unwrap();
    let mut row = env.db.project_detail(&scope, project).await.unwrap().unwrap().listing.project;
    row.cover_image_asset = Some(a.id.0);
    assert!(env.db.update_project(&scope, &row).await.unwrap());

    let raw = env.conn.raw().await.unwrap();
    for _ in 0..2 {
        thrust_adapters_postgres::migrate(&raw).await.unwrap();
    }
    assert_eq!(env.db.asset(&scope, a.id).await.unwrap(), Some(a.clone()));
    let seen = env.db.project_detail(&scope, project).await.unwrap().unwrap().listing.project;
    assert_eq!(seen.cover_image_asset, Some(a.id.0));
    // and the foreign key is still there after the file has dropped and made it again
    let n: i64 = raw
        .query_one("SELECT count(*) FROM pg_constraint WHERE conname = 'projects_cover_image_asset_fkey'", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(n, 1);
}

#[tokio::test]
async fn a_grant_is_spent_by_exactly_one_of_many_racing_callers() {
    let env = env().await;
    let (ws, owner, _, scope) = world(&env, "asset-race").await;
    let a = pending(ws, owner, 7);
    env.db.insert_asset(&scope, &a).await.unwrap();
    let token = TokenHash([7; 32]);
    let tasks: Vec<_> = (0..16)
        .map(|_| {
            let db = env.db.clone();
            let id = a.id;
            tokio::spawn(async move { db.take_upload_grant(id, &token, at(10)).await.unwrap() })
        })
        .collect();
    let mut won = 0;
    for t in tasks {
        if t.await.unwrap().is_some() {
            won += 1;
        }
    }
    assert_eq!(won, 1);
    assert_eq!(env.db.asset(&scope, a.id).await.unwrap().unwrap().upload, None);
}

// ---- the model -------------------------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
enum Op {
    Insert { who: usize },
    Get { idx: usize, other_workspace: bool },
    Mark { idx: usize, other_workspace: bool },
    Take { idx: usize, right_token: bool, when: usize },
    Attach { who: usize, a: usize, b: usize },
    Public { idx: usize },
    Delete { idx: usize },
}

/// Seconds from an asset's creation to the moment a grant is asked for: either side of its hour, and its edge.
const WHEN: [i64; 5] = [0, 3599, 3600, 3601, 7200];

fn ops() -> impl Strategy<Value = Vec<Op>> {
    // few enough that steps land on the same asset again and again: a marked asset is marked again, a spent grant is asked for again
    let idx = || 0usize..4;
    let op = prop_oneof![
        5 => (0usize..3).prop_map(|who| Op::Insert { who }),
        3 => (idx(), any::<bool>()).prop_map(|(idx, other_workspace)| Op::Get { idx, other_workspace }),
        3 => (idx(), proptest::bool::weighted(0.15)).prop_map(|(idx, other_workspace)| Op::Mark { idx, other_workspace }),
        4 => (idx(), proptest::bool::weighted(0.8), 0..WHEN.len()).prop_map(|(idx, right_token, when)| Op::Take { idx, right_token, when }),
        4 => (0usize..2, idx(), idx()).prop_map(|(who, a, b)| Op::Attach { who, a, b }),
        3 => idx().prop_map(|idx| Op::Public { idx }),
        1 => idx().prop_map(|idx| Op::Delete { idx }),
    ];
    proptest::collection::vec(op, 1..28)
}

/// What the model knows of an asset: the row it should read back as, less what never changes.
#[derive(Debug, Clone)]
struct M {
    row: AssetRow,
    deleted: bool,
}

struct Fix {
    ws_a: WorkspaceId,
    ws_b: WorkspaceId,
    project: ProjectId,
    /// Two in workspace A and its project, one in workspace B.
    users: [UserId; 3],
}

impl Fix {
    fn ws(&self, who: usize) -> WorkspaceId {
        if who == 2 {
            self.ws_b
        } else {
            self.ws_a
        }
    }
    fn scope(&self, ws: WorkspaceId, who: usize) -> WorkspaceScope {
        WorkspaceScope::mint(ws, self.users[who], Role::Member)
    }
}

async fn fixture(env: &Env) -> Fix {
    let (ws_a, ws_b, project, users) = (
        WorkspaceId(new_id()),
        WorkspaceId(new_id()),
        ProjectId(new_id()),
        [UserId(new_id()), UserId(new_id()), UserId(new_id())],
    );
    env.db
        .seed(
            &SystemScope::mint(),
            &SeedFixture {
                workspaces: vec![
                    SeedWorkspace { id: ws_a, slug: slug_of(ws_a) },
                    SeedWorkspace { id: ws_b, slug: slug_of(ws_b) },
                ],
                users: users.to_vec(),
                workspace_members: vec![
                    SeedWorkspaceMember { workspace_id: ws_a, user_id: users[0] },
                    SeedWorkspaceMember { workspace_id: ws_a, user_id: users[1] },
                    SeedWorkspaceMember { workspace_id: ws_b, user_id: users[2] },
                ],
                projects: vec![SeedProject { id: project, workspace_id: ws_a, name: "p".into() }],
                memberships: vec![
                    SeedMembership { project_id: project, workspace_id: ws_a, user_id: users[0], role: Role::Member },
                    SeedMembership { project_id: project, workspace_id: ws_a, user_id: users[1], role: Role::Member },
                ],
                sessions: vec![],
                ..SeedFixture::default()
            },
        )
        .await
        .unwrap();
    Fix { ws_a, ws_b, project, users }
}

/// Run the steps against Postgres and the model, step by step. `seen` collects what the sequences got to.
async fn run_case(env: &Env, ops: Vec<Op>, seen: &RefCell<BTreeSet<&'static str>>) -> Result<(), TestCaseError> {
    let f = fixture(env).await;
    let raw = env.conn.raw().await.unwrap();
    let mut model: Vec<M> = vec![];
    let mut cover: Option<usize> = None;
    let mut stamped: Option<(UserId, Timestamp)> = None;
    let see = |s: &'static str| {
        seen.borrow_mut().insert(s);
    };
    for (step, op) in ops.iter().copied().enumerate() {
        let now = 100 + i64::try_from(step).unwrap();
        match op {
            Op::Insert { who } => {
                let n = u8::try_from(model.len() + 1).unwrap();
                let mut row = pending(f.ws(who), f.users[who], n);
                // a time of its own, so that "the oldest" is a fact of the data, and an expiry an hour after it
                row.created_at = at(i64::try_from(model.len()).unwrap());
                row.upload = Some(UploadGrant {
                    token_hash: TokenHash([n; 32]),
                    expires_at: at(i64::try_from(model.len()).unwrap() + 3600),
                });
                env.db.insert_asset(&f.scope(f.ws(who), who), &row).await.unwrap();
                see("inserted");
                model.push(M { row, deleted: false });
            }
            Op::Get { idx, other_workspace } if !model.is_empty() => {
                let m = &model[idx % model.len()];
                let ws = if other_workspace { f.ws_b } else { f.ws_a };
                let got = env.db.asset(&f.scope(ws, 0), m.row.id).await.unwrap();
                let want = (m.row.workspace_id == ws && !m.deleted).then(|| m.row.clone());
                prop_assert_eq!(&got, &want, "{:?}", op);
                if other_workspace && m.row.workspace_id != ws {
                    see("read-through-another-workspace");
                }
            }
            Op::Mark { idx, other_workspace } if !model.is_empty() => {
                let i = idx % model.len();
                let m = &mut model[i];
                let ws = if other_workspace { f.ws_b } else { f.ws_a };
                let got = env.db.mark_asset_uploaded(&f.scope(ws, 0), m.row.id, at(now)).await.unwrap();
                let want = m.row.workspace_id == ws && !m.deleted;
                prop_assert_eq!(got, want, "{:?}", op);
                if want {
                    if m.row.state == AssetState::Uploaded {
                        see("marked-again");
                    } else {
                        m.row.state = AssetState::Uploaded;
                        m.row.uploaded_at = Some(at(now));
                    }
                    m.row.upload = None;
                } else if m.deleted {
                    see("mark-of-a-deleted-one");
                }
            }
            Op::Take { idx, right_token, when } if !model.is_empty() => {
                let i = idx % model.len();
                let m = &mut model[i];
                let token =
                    if right_token { TokenHash([u8::try_from(i + 1).unwrap(); 32]) } else { TokenHash([0xEE; 32]) };
                let asked = m.row.created_at + chrono_secs(WHEN[when]);
                let got = env.db.take_upload_grant(m.row.id, &token, asked).await.unwrap();
                let live_grant = m.row.upload.filter(|g| right_token && g.expires_at > asked);
                let want = (live_grant.is_some() && !m.deleted && m.row.state == AssetState::Pending)
                    .then(|| AssetRow { upload: None, ..m.row.clone() });
                prop_assert_eq!(&got, &want, "{:?}", op);
                if want.is_some() {
                    m.row.upload = None;
                    see("grant-spent");
                } else if !right_token {
                    see("grant-wrong-token");
                } else if m.row.upload.is_none() {
                    see("grant-already-spent");
                } else {
                    see("grant-expired");
                }
            }
            Op::Attach { who, a, b } if !model.is_empty() => {
                let named: Vec<usize> =
                    [a % model.len(), b % model.len()].into_iter().collect::<BTreeSet<_>>().into_iter().collect();
                let ids: Vec<AssetId> = named.iter().map(|i| model[*i].row.id).collect();
                let scope = Scope::mint(f.ws_a, f.project, f.users[who]);
                let got = env.db.attach_project_cover(&scope, &ids, at(now)).await.unwrap();
                let eligible: Vec<usize> = named
                    .iter()
                    .copied()
                    .filter(|i| {
                        let m = &model[*i];
                        !m.deleted && m.row.workspace_id == f.ws_a && m.row.created_by == f.users[who]
                    })
                    .collect();
                let want = eligible.iter().copied().min().map(|i| model[i].row.id);
                prop_assert_eq!(got, want, "{:?}", op);
                if eligible.is_empty() {
                    see("attach-none");
                } else {
                    if eligible.len() < named.len() {
                        see("attach-some-left-out");
                    }
                    if cover.is_some_and(|c| Some(c) != eligible.iter().copied().min()) {
                        see("cover-replaced");
                    }
                    for i in &eligible {
                        model[*i].row.project_id = Some(f.project);
                    }
                    cover = eligible.iter().copied().min();
                    stamped = Some((f.users[who], at(now)));
                }
            }
            Op::Public { idx } if !model.is_empty() => {
                let m = &model[idx % model.len()];
                let got = env.db.uploaded_asset(m.row.id).await.unwrap();
                let want = (!m.deleted && m.row.state == AssetState::Uploaded).then(|| m.row.clone());
                prop_assert_eq!(&got, &want, "{:?}", op);
                see(if want.is_some() { "public-uploaded" } else { "public-nothing" });
            }
            Op::Delete { idx } if !model.is_empty() => {
                let i = idx % model.len();
                let m = &mut model[i];
                raw.execute("UPDATE assets SET deleted_at = $2 WHERE id = $1", &[&m.row.id.0, &at(now)]).await.unwrap();
                m.deleted = true;
                see("deleted");
            }
            _ => {}
        }

        // after every step each asset reads back as the model says, and the project has the cover and the stamp it should
        for m in &model {
            let got = env.db.asset(&f.scope(m.row.workspace_id, 0), m.row.id).await.unwrap();
            prop_assert_eq!(&got, &(!m.deleted).then(|| m.row.clone()), "{:?} after {:?}", m.row.id, op);
        }
        let p = env.db.project_detail(&f.scope(f.ws_a, 0), f.project).await.unwrap().unwrap().listing.project;
        prop_assert_eq!(p.cover_image_asset, cover.map(|i| model[i].row.id.0), "the project's cover after {:?}", op);
        if let Some((by, when)) = stamped {
            prop_assert_eq!((p.updated_by, p.updated_at), (Some(by), when), "the project's stamp after {:?}", op);
        }
    }
    Ok(())
}

fn chrono_secs(n: i64) -> thrust_contracts::chrono::TimeDelta {
    thrust_contracts::chrono::TimeDelta::seconds(n)
}

/// What the sequences must get to, or the model proves less than it seems to (AGENTS.md rule 8).
const SITUATIONS: [&str; 14] = [
    "inserted",
    "read-through-another-workspace",
    "marked-again",
    "mark-of-a-deleted-one",
    "grant-spent",
    "grant-wrong-token",
    "grant-already-spent",
    "grant-expired",
    "attach-none",
    "attach-some-left-out",
    "cover-replaced",
    "public-uploaded",
    "public-nothing",
    "deleted",
];

fn config() -> Config {
    // each case seeds a workspace and reads every asset back after every step, against a container: tens of cases, not hundreds
    let cases = std::env::var("THRUST_MODEL_CASES").ok().and_then(|v| v.parse().ok()).unwrap_or(24);
    Config { cases, max_shrink_iters: 256, max_shrink_time: 60_000, ..Config::default() }
}

/// Three sequences that between them get to every one of `SITUATIONS`, by construction. Rule 8's control for the model: it is run
/// first, through the same `run_case` and against the same Postgres, so that the model's reach does not depend on a random draw. (It
/// used to: the check at the end of the random run needed, say, a delete (one step in twenty) and then a mark of that very asset
/// among 24 random sequences, and failed CI on a fair share of runs, each time for a different situation.) The indices are into the
/// assets a sequence has inserted so far, in order; users 0 and 1 are in workspace A and user 2 is in workspace B.
fn pinned() -> [Vec<Op>; 3] {
    [
        // an asset read through another workspace, not public until it is uploaded, uploaded twice, public, deleted, and then marked
        vec![
            Op::Insert { who: 0 },
            Op::Insert { who: 0 },
            Op::Insert { who: 2 },
            Op::Get { idx: 0, other_workspace: true },
            Op::Public { idx: 0 },
            Op::Mark { idx: 0, other_workspace: false },
            Op::Mark { idx: 0, other_workspace: false },
            Op::Public { idx: 0 },
            Op::Delete { idx: 0 },
            Op::Mark { idx: 0, other_workspace: false },
        ],
        // a grant spent and asked for again; the wrong token; and an expiry equal to the time asked, which has expired
        vec![
            Op::Insert { who: 0 },
            Op::Take { idx: 0, right_token: true, when: 0 },
            Op::Take { idx: 0, right_token: true, when: 0 },
            Op::Insert { who: 0 },
            Op::Take { idx: 1, right_token: false, when: 0 },
            Op::Take { idx: 1, right_token: true, when: 2 },
        ],
        // a cover named from another workspace's asset (none eligible), one named with an asset of someone else's (left out), and a
        // cover replaced by the oldest eligible of another caller's
        vec![
            Op::Insert { who: 0 },
            Op::Insert { who: 1 },
            Op::Insert { who: 2 },
            Op::Attach { who: 0, a: 2, b: 2 },
            Op::Attach { who: 0, a: 0, b: 1 },
            Op::Attach { who: 1, a: 1, b: 1 },
        ],
    ]
}

#[test]
fn model_asset_methods_match_the_reference_against_postgres() {
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
    let env = rt.block_on(env());
    let seen = RefCell::new(BTreeSet::new());

    // the pinned sequences first: each agrees with the model, and together they get to every situation
    for ops in pinned() {
        if let Err(e) = rt.block_on(run_case(&env, ops, &seen)) {
            panic!("a pinned sequence disagrees with the model: {e}");
        }
    }
    {
        let seen = seen.borrow();
        for s in SITUATIONS {
            assert!(seen.contains(s), "no pinned sequence gets to {s:?}; they get to {seen:?}");
        }
    }

    // then the random ones, which look for what no one thought to pin
    let config = config();
    assert!(config.cases > 0, "THRUST_MODEL_CASES=0 would check nothing");
    let mut runner = TestRunner::new(config);
    let result = runner.run(&ops(), |ops| rt.block_on(run_case(&env, ops, &seen)));
    rt.block_on(async move { drop(env) });
    if let Err(e) = result {
        panic!("{e}");
    }
    eprintln!("ASSET-MODEL-OBSERVATION: the pinned and the random sequences got to {:?}", seen.into_inner());
}

/// The strategy reaches every kind of step: a sequence that never deletes would pass on an adapter that gets deletion wrong.
#[test]
fn model_asset_strategy_reaches_every_step_class() {
    let mut runner = TestRunner::deterministic();
    let mut seen = BTreeMap::new();
    for _ in 0..300 {
        for op in ops().new_tree(&mut runner).unwrap().current() {
            let name = match op {
                Op::Insert { .. } => "insert",
                Op::Get { .. } => "get",
                Op::Mark { .. } => "mark",
                Op::Take { .. } => "take",
                Op::Attach { .. } => "attach",
                Op::Public { .. } => "public",
                Op::Delete { .. } => "delete",
            };
            *seen.entry(name).or_insert(0usize) += 1;
        }
    }
    assert_eq!(seen.len(), 7, "{seen:?}");
}
