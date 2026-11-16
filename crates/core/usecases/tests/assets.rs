//! Assets (issue #44, the asset stage; ADR-0047) through the use-cases, on the memory adapters: the lifecycle (create, upload,
//! confirm, read, attach), who may do each step, and the one-time grant. A sequence of steps is checked against a reference model
//! (`model_`), and the cases the ADR or the pin names are examples.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "test code: a panic is how a test fails, and clippy.toml exempts only #[test] functions and #[cfg(test)] modules, not the helpers around them"
)]

mod common;

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

use common::*;
use futures_executor::block_on;
use proptest::prelude::*;
use proptest::strategy::ValueTree;
use proptest::test_runner::{Config, TestCaseError, TestRunner};
use rstest::rstest;
use thrust_adapters_memory::{ConsumerSlot, MemoryQueue};
use thrust_contracts::chrono::TimeDelta;
use thrust_contracts::*;
use thrust_core::*;
use thrust_ports::*;

const LIMIT: usize = 8;

fn cfg() -> CoreConfig {
    CoreConfig { outbox_fastpath: false, max_attachment_bytes: LIMIT, ..CoreConfig::default() }
}

fn ws_scope(w: &World, a: Actor) -> Result<WorkspaceScope, CoreError> {
    block_on(authorize_workspace(&w.app, &w.principal(a), &w.slug_a))
}

fn new_cover(size: Option<i64>) -> NewUpload {
    NewUpload {
        name: Some("cover.png".into()),
        content_type: "image/png".into(),
        size,
        entity: AssetEntity::ProjectCover,
    }
}

fn make(w: &World, a: Actor, size: i64) -> UploadTicket {
    block_on(create_asset(&w.app, &ws_scope(w, a).unwrap(), new_cover(Some(size)))).unwrap()
}

fn upload(w: &World, t: &UploadTicket, bytes: &[u8]) -> Result<(), AssetError> {
    block_on(receive_upload(&w.app, t.asset.id, &t.token, Some("image/png"), bytes.to_vec()))
}

fn confirm(w: &World, a: Actor, id: AssetId) -> Result<AssetRow, AssetError> {
    block_on(confirm_upload(&w.app, &ws_scope(w, a).map_err(AssetError::Core)?, id))
}

/// The project's cover, as an admin of its workspace reads it.
fn cover_of_project_a(w: &World) -> Option<Uuid> {
    let scope = ws_scope(w, Actor::AdminA).unwrap();
    block_on(w.app.ports.db.project_detail(&scope, w.project_a)).unwrap().unwrap().listing.project.cover_image_asset
}

// (b) ADR-0047's own lifecycle: create -> PUT -> confirm -> read, then S31 attaches the cover
#[test]
fn an_asset_runs_the_lifecycle_of_adr_0047() {
    let w = world_config(cfg());
    let t = make(&w, Actor::MemberA, 4);
    assert_eq!(t.asset.state, AssetState::Pending);
    assert_eq!(t.asset.project_id, None);
    assert_eq!(t.asset.size, 4);
    assert!(t.asset.object_key.starts_with(&format!("{}/", w.ws_a)) && t.asset.object_key.ends_with("-cover.png"));
    // the row holds the digest of the token, never the token
    let grant = t.asset.upload.unwrap();
    assert_eq!(grant.token_hash, hash_token(&t.token));
    assert!(!format!("{:?}", t.asset).contains(&t.token));

    // nothing can be read before the bytes are there and confirmed
    let admin = ws_scope(&w, Actor::AdminA).unwrap();
    assert!(matches!(block_on(open_asset(&w.app, &admin, t.asset.id)), Err(AssetError::NotUploaded)));
    assert!(matches!(block_on(public_asset(&w.app, t.asset.id)), Err(AssetError::NotUploaded)));
    assert!(matches!(confirm(&w, Actor::MemberA, t.asset.id), Err(AssetError::ObjectMissing)));

    upload(&w, &t, &[1, 2, 3, 4]).unwrap();
    let done = confirm(&w, Actor::MemberA, t.asset.id).unwrap();
    assert_eq!((done.state, done.upload.is_some()), (AssetState::Uploaded, false));
    // confirming again changes nothing
    assert_eq!(confirm(&w, Actor::MemberA, t.asset.id).unwrap().uploaded_at, done.uploaded_at);

    let opened = block_on(open_asset(&w.app, &admin, t.asset.id)).unwrap();
    assert_eq!(block_on(read_asset(&w.app, &opened)).unwrap().bytes, vec![1, 2, 3, 4]);
    assert_eq!(block_on(public_asset(&w.app, t.asset.id)).unwrap().id, t.asset.id);

    // S31: the creator attaches it to project A, which makes it the cover
    assert_eq!(cover_of_project_a(&w), None);
    let cover =
        block_on(attach_project_cover(&w.app, &w.principal(Actor::MemberA), &w.slug_a, w.project_a, &[t.asset.id]))
            .unwrap();
    assert_eq!(cover, t.asset.id);
    assert_eq!(cover_of_project_a(&w), Some(t.asset.id.0));
    assert_eq!(block_on(open_asset(&w.app, &admin, t.asset.id)).unwrap().project_id, Some(w.project_a));
}

#[rstest]
fn a_file_type_outside_the_pins_list_is_refused(
    #[values(
        "image/jpeg",
        "image/png",
        "image/webp",
        "image/jpg",
        "image/gif",
        "image/svg+xml",
        "text/html",
        "application/pdf",
        "",
        "IMAGE/PNG",
        "image/png; charset=x"
    )]
    content_type: &str,
) {
    let w = world_config(cfg());
    let r = block_on(create_asset(
        &w.app,
        &ws_scope(&w, Actor::MemberA).unwrap(),
        NewUpload { content_type: content_type.into(), ..new_cover(Some(4)) },
    ));
    assert_eq!(r.is_ok(), IMAGE_TYPES.contains(&content_type), "{content_type:?}");
    if let Err(e) = r {
        assert!(matches!(e, AssetError::BadType), "{e:?}");
    }
}

#[rstest]
#[case::no_size_means_the_limit(None, Some(LIMIT as i64))]
#[case::under(Some(3), Some(3))]
#[case::at_the_limit(Some(LIMIT as i64), Some(LIMIT as i64))]
#[case::over_is_clamped_in_silence(Some(LIMIT as i64 + 1), Some(LIMIT as i64))]
#[case::zero(Some(0), None)]
#[case::negative(Some(-5), None)]
fn the_size_is_held_to_the_limit_and_must_be_positive(#[case] asked: Option<i64>, #[case] kept: Option<i64>) {
    let w = world_config(cfg());
    let r = block_on(create_asset(&w.app, &ws_scope(&w, Actor::MemberA).unwrap(), new_cover(asked)));
    match kept {
        Some(n) => assert_eq!(r.unwrap().asset.size, n),
        None => assert!(matches!(r, Err(AssetError::BadSize))),
    }
}

#[rstest]
#[case::the_pins_example("../../etc/passwd", "passwd")]
#[case::nothing_left(" ..", "unnamed")]
#[case::none_given("", "unnamed")]
fn the_name_is_sanitised_and_the_key_has_one_segment_for_it(#[case] given: &str, #[case] kept: &str) {
    let w = world_config(cfg());
    let t = block_on(create_asset(
        &w.app,
        &ws_scope(&w, Actor::MemberA).unwrap(),
        NewUpload { name: Some(given.into()), ..new_cover(Some(4)) },
    ))
    .unwrap();
    assert_eq!(t.asset.name, kept);
    assert_eq!(t.asset.object_key.matches('/').count(), 1, "{}", t.asset.object_key);
    // the one without a name at all
    let t = block_on(create_asset(
        &w.app,
        &ws_scope(&w, Actor::MemberA).unwrap(),
        NewUpload { name: None, ..new_cover(Some(4)) },
    ))
    .unwrap();
    assert_eq!(t.asset.name, "unnamed");
}

#[test]
fn a_name_over_the_limit_is_refused() {
    let w = world_config(cfg());
    let scope = ws_scope(&w, Actor::MemberA).unwrap();
    let name = |n: usize| NewUpload { name: Some("a".repeat(n)), ..new_cover(Some(4)) };
    assert!(block_on(create_asset(&w.app, &scope, name(MAX_ASSET_NAME))).is_ok());
    assert!(matches!(block_on(create_asset(&w.app, &scope, name(MAX_ASSET_NAME + 1))), Err(AssetError::BadName)));
}

/// A clock a test moves.
struct Moving(Mutex<Timestamp>);
impl Clock for Moving {
    fn now(&self) -> Timestamp {
        *self.0.lock().unwrap()
    }
}

#[rstest]
#[case::just_before(3599, true)]
#[case::at_the_instant(3600, false)]
#[case::after(7200, false)]
fn the_grant_lasts_an_hour(#[case] secs: i64, #[case] works: bool) {
    let start = thrust_contracts::chrono::DateTime::from_timestamp(1_760_000_000, 0).unwrap();
    let clock = Arc::new(Moving(Mutex::new(start)));
    let background: Arc<dyn Background> = Arc::new(NoBackground);
    let queue: Arc<dyn Queue> = Arc::new(MemoryQueue::new(background.clone(), ConsumerSlot::default()));
    let w = world_built(cfg(), queue, background, clock.clone());
    let t = make(&w, Actor::MemberA, 4);
    *clock.0.lock().unwrap() = start + TimeDelta::seconds(secs);
    assert_eq!(upload(&w, &t, &[1, 2, 3, 4]).is_ok(), works, "{secs}s after it was made");
}

#[test]
fn a_grant_is_for_one_asset_and_one_use() {
    let w = world_config(cfg());
    let (a, b) = (make(&w, Actor::MemberA, 4), make(&w, Actor::MemberA, 4));
    // a token is not another asset's
    let r = block_on(receive_upload(&w.app, a.asset.id, &b.token, Some("image/png"), vec![0; 4]));
    assert!(matches!(r, Err(AssetError::NoGrant)), "{r:?}");
    upload(&w, &a, &[0; 4]).unwrap();
    assert!(matches!(upload(&w, &a, &[0; 4]), Err(AssetError::NoGrant)));
    // and the other's own is still good
    upload(&w, &b, &[0; 4]).unwrap();
}

#[rstest]
#[case::wrong_size(vec![0; 5], "image/png")]
#[case::short(vec![0; 3], "image/png")]
#[case::empty(vec![], "image/png")]
#[case::wrong_type(vec![0; 4], "image/gif")]
fn an_upload_that_is_not_the_asset_is_refused_and_spends_the_grant(#[case] bytes: Vec<u8>, #[case] content_type: &str) {
    let w = world_config(cfg());
    let t = make(&w, Actor::MemberA, 4);
    let r = block_on(receive_upload(&w.app, t.asset.id, &t.token, Some(content_type), bytes));
    assert!(matches!(r, Err(AssetError::UploadMismatch)), "{r:?}");
    assert!(matches!(upload(&w, &t, &[0; 4]), Err(AssetError::NoGrant)), "a refused upload spends the grant");
    assert!(matches!(confirm(&w, Actor::MemberA, t.asset.id), Err(AssetError::ObjectMissing)), "nothing was stored");
}

#[test]
fn an_upload_with_no_type_is_refused() {
    let w = world_config(cfg());
    let t = make(&w, Actor::MemberA, 4);
    let r = block_on(receive_upload(&w.app, t.asset.id, &t.token, None, vec![0; 4]));
    assert!(matches!(r, Err(AssetError::UploadMismatch)), "{r:?}");
}

// confirm looks at the object where the pin trusts the client; an object put by another route is checked the same way
#[rstest]
#[case::another_size(vec![0; 9], Some("image/png"), false)]
#[case::another_type(vec![0; 4], Some("image/gif"), false)]
#[case::no_type(vec![0; 4], None, false)]
#[case::the_right_one(vec![0; 4], Some("image/png"), true)]
#[case::the_type_with_a_parameter(vec![0; 4], Some("IMAGE/PNG; x=y"), true)]
fn confirm_checks_the_object_it_finds(#[case] bytes: Vec<u8>, #[case] content_type: Option<&str>, #[case] ok: bool) {
    let w = world_config(cfg());
    let t = make(&w, Actor::MemberA, 4);
    block_on(w.app.ports.blobs.put(&t.asset.object_key, bytes, content_type.map(str::to_string))).unwrap();
    let r = confirm(&w, Actor::MemberA, t.asset.id);
    assert_eq!(r.is_ok(), ok, "{r:?}");
    if !ok {
        assert!(matches!(r, Err(AssetError::ObjectMismatch)), "{r:?}");
    }
}

#[test]
fn an_asset_of_one_workspace_is_not_there_for_another() {
    let w = world_config(cfg());
    let t = make(&w, Actor::Dual, 4);
    upload(&w, &t, &[0; 4]).unwrap();
    // the same user, through the other workspace
    let b = block_on(authorize_workspace(&w.app, &w.principal(Actor::Dual), &w.slug_b)).unwrap();
    assert!(matches!(block_on(open_asset(&w.app, &b, t.asset.id)), Err(AssetError::NotFound)));
    assert!(matches!(block_on(confirm_upload(&w.app, &b, t.asset.id)), Err(AssetError::NotFound)));
    // and one who is not in workspace A at all does not even get a scope
    assert!(matches!(ws_scope(&w, Actor::MemberB), Err(CoreError::NotFound)));
    assert!(matches!(confirm(&w, Actor::MemberB, t.asset.id), Err(AssetError::Core(CoreError::NotFound))));
}

// who may name an asset as a project's cover: S31's rule, and a save that repeats the cover the project has
#[test]
fn a_cover_is_attachable_by_its_maker_before_it_has_a_project_and_by_anyone_once_it_is_the_projects() {
    let w = world_config(cfg());
    let t = make(&w, Actor::MemberA, 4);
    let id = t.asset.id;
    let attachable = |a: Actor, project: Option<ProjectId>| {
        block_on(is_attachable_cover(&w.app, &ws_scope(&w, a).unwrap(), id, project)).unwrap()
    };
    // pending: not yet
    assert!(!attachable(Actor::MemberA, None));
    upload(&w, &t, &[0; 4]).unwrap();
    confirm(&w, Actor::MemberA, id).unwrap();
    assert!(attachable(Actor::MemberA, None), "the maker, for a project being made");
    assert!(!attachable(Actor::GuestA, None), "not another member's upload");
    assert!(!attachable(Actor::AdminA, Some(w.project_a)), "not an admin's either, while it has no project");
    block_on(attach_project_cover(&w.app, &w.principal(Actor::MemberA), &w.slug_a, w.project_a, &[id])).unwrap();
    assert!(attachable(Actor::AdminA, Some(w.project_a)), "once it is the project's, a save that repeats it is fine");
    assert!(!attachable(Actor::AdminA, Some(ProjectId(uuid(0xDEAD)))), "but not for another project");
    assert!(!attachable(Actor::AdminA, None), "nor for a project being made");
    let nothing =
        block_on(is_attachable_cover(&w.app, &ws_scope(&w, Actor::MemberA).unwrap(), AssetId(uuid(0x404)), None));
    assert!(!nothing.unwrap(), "an id that is no asset");
}

#[test]
fn a_bulk_attach_names_assets_and_a_project_the_caller_may_use() {
    let w = world_config(cfg());
    let mine = make(&w, Actor::MemberA, 4);
    let theirs = make(&w, Actor::GuestA, 4);
    let attach = |a: Actor, ids: &[AssetId]| {
        block_on(attach_project_cover(&w.app, &w.principal(a), &w.slug_a, w.project_a, ids))
    };
    assert!(matches!(attach(Actor::MemberA, &[]), Err(AssetError::NoIds)));
    // another member's upload is left out, and when nothing else is named, none is found
    assert!(matches!(attach(Actor::MemberA, &[theirs.asset.id]), Err(AssetError::NotFound)));
    assert!(matches!(attach(Actor::MemberA, &[AssetId(uuid(0x404))]), Err(AssetError::NotFound)));
    // not a member of the project: refused before any asset is looked at
    assert!(matches!(attach(Actor::OutsiderA, &[mine.asset.id]), Err(AssetError::Core(CoreError::Forbidden))));
    assert!(matches!(attach(Actor::MemberB, &[mine.asset.id]), Err(AssetError::Core(CoreError::NotFound))));
    assert_eq!(cover_of_project_a(&w), None, "nothing was written by any of those");
    // named with one that is not theirs, the caller's own goes through
    assert_eq!(attach(Actor::MemberA, &[theirs.asset.id, mine.asset.id]).unwrap(), mine.asset.id);
    assert_eq!(cover_of_project_a(&w), Some(mine.asset.id.0));
}

#[test]
fn of_several_uploads_the_oldest_is_the_cover_the_pin_ends_with() {
    let w = world_config(cfg());
    let (first, second) = (make(&w, Actor::MemberA, 4), make(&w, Actor::MemberA, 4));
    // newest named first: the order of the request does not matter
    let r = block_on(attach_project_cover(
        &w.app,
        &w.principal(Actor::MemberA),
        &w.slug_a,
        w.project_a,
        &[second.asset.id, first.asset.id],
    ));
    assert_eq!(r.unwrap(), first.asset.id);
    assert_eq!(cover_of_project_a(&w), Some(first.asset.id.0));
}

// ---- the model: any sequence of steps behaves as the reference says ---------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Sent {
    Right,
    WrongSize,
    WrongType,
}

#[derive(Debug, Clone)]
enum Op {
    Create {
        actor: Actor,
        png: bool,
        size: i64,
    },
    Upload {
        asset: usize,
        token_ok: bool,
        sent: Sent,
    },
    Confirm {
        actor: Actor,
        asset: usize,
    },
    Open {
        actor: Actor,
        asset: usize,
    },
    /// An `actor` of `None` is whoever made the first asset named: most attaches are meant to go through, and only those reach
    /// what follows one (a project's asset read by someone who is not in the project).
    Attach {
        actor: Option<Actor>,
        assets: Vec<usize>,
    },
}

/// Who acts: every one of the world's but the one who is in both workspaces, who adds nothing here.
const WHO: [Actor; 5] = [Actor::AdminA, Actor::MemberA, Actor::GuestA, Actor::OutsiderA, Actor::MemberB];

fn actor() -> impl Strategy<Value = Actor> {
    proptest::sample::select(WHO.to_vec())
}

fn ops() -> impl Strategy<Value = Vec<Op>> {
    let usually = |n: u32| prop_oneof![n => Just(true), 1 => Just(false)];
    let sent = prop_oneof![5 => Just(Sent::Right), 1 => Just(Sent::WrongSize), 1 => Just(Sent::WrongType)];
    let attacher = prop_oneof![3 => Just(None), 1 => actor().prop_map(Some)];
    let op = prop_oneof![
        4 => (actor(), usually(5), -1i64..=12).prop_map(|(actor, png, size)| Op::Create { actor, png, size }),
        4 => (any::<usize>(), usually(5), sent).prop_map(|(asset, token_ok, sent)| Op::Upload { asset, token_ok, sent }),
        4 => (actor(), any::<usize>()).prop_map(|(actor, asset)| Op::Confirm { actor, asset }),
        4 => (actor(), any::<usize>()).prop_map(|(actor, asset)| Op::Open { actor, asset }),
        4 => (attacher, proptest::collection::vec(any::<usize>(), 0..3))
            .prop_map(|(actor, assets)| Op::Attach { actor, assets }),
    ];
    proptest::collection::vec(op, 1..30)
}

/// What the reference knows of an asset.
#[derive(Debug, Clone)]
struct Model {
    owner: Actor,
    size: usize,
    grant: bool,
    stored: bool,
    uploaded: bool,
    in_project_a: bool,
}

fn in_workspace_a(a: Actor) -> bool {
    a != Actor::MemberB
}

fn in_project_a(a: Actor) -> bool {
    matches!(a, Actor::AdminA | Actor::MemberA | Actor::GuestA)
}

/// The same label for what the use-cases answer and what the reference says.
fn label<T>(r: &Result<T, AssetError>) -> &'static str {
    match r {
        Ok(_) => "ok",
        Err(AssetError::Core(CoreError::NotFound)) => "no-workspace",
        Err(AssetError::Core(CoreError::Forbidden)) => "not-in-project",
        Err(AssetError::Core(_)) => "other",
        Err(AssetError::NotFound) => "no-asset",
        Err(AssetError::NoAccess) => "no-access",
        Err(AssetError::NotUploaded) => "not-uploaded",
        Err(AssetError::BadType) => "bad-type",
        Err(AssetError::BadSize) => "bad-size",
        Err(AssetError::BadName) => "bad-name",
        Err(AssetError::NoGrant) => "no-grant",
        Err(AssetError::UploadMismatch) => "mismatch",
        Err(AssetError::ObjectMissing) => "object-missing",
        Err(AssetError::ObjectMismatch) => "object-mismatch",
        Err(AssetError::NoIds) => "no-ids",
    }
}

/// Run the steps against the use-cases and the reference, step by step; `seen` collects what the sequences got to.
fn run_model(ops: Vec<Op>, seen: &mut BTreeSet<&'static str>) -> Result<(), TestCaseError> {
    let w = world_config(cfg());
    let mut tickets: Vec<UploadTicket> = vec![];
    let mut model: Vec<Model> = vec![];
    let mut cover: Option<usize> = None;
    for op in ops {
        // a clone, so that `op` is still whole for the messages of the assertions
        match op.clone() {
            Op::Create { actor, png, size } => {
                let content_type = if png { "image/png" } else { "image/bmp" };
                let real = ws_scope(&w, actor).map_err(AssetError::Core).and_then(|scope| {
                    let req = NewUpload { content_type: content_type.into(), ..new_cover(Some(size)) };
                    block_on(create_asset(&w.app, &scope, req))
                });
                let kept = size.min(LIMIT as i64);
                let want = if !in_workspace_a(actor) {
                    "no-workspace"
                } else if !png {
                    "bad-type"
                } else if kept < 1 {
                    "bad-size"
                } else {
                    "ok"
                };
                prop_assert_eq!(label(&real), want, "{:?}", op);
                if let Ok(t) = real {
                    seen.insert("created");
                    let size = usize::try_from(kept).unwrap();
                    model.push(Model {
                        owner: actor,
                        size,
                        grant: true,
                        stored: false,
                        uploaded: false,
                        in_project_a: false,
                    });
                    tickets.push(t);
                }
            }
            Op::Upload { asset, token_ok, sent } if !model.is_empty() => {
                let i = asset % model.len();
                let token = if token_ok { tickets[i].token.clone() } else { "not-the-token".to_string() };
                let (len, content_type) = match sent {
                    Sent::Right => (model[i].size, "image/png"),
                    Sent::WrongSize => (model[i].size + 1, "image/png"),
                    Sent::WrongType => (model[i].size, "image/gif"),
                };
                let real =
                    block_on(receive_upload(&w.app, tickets[i].asset.id, &token, Some(content_type), vec![7; len]));
                let want = if !(model[i].grant && token_ok) {
                    "no-grant"
                } else if sent != Sent::Right {
                    "mismatch"
                } else {
                    "ok"
                };
                prop_assert_eq!(label(&real), want, "{:?}", op);
                if token_ok && !model[i].grant {
                    seen.insert("replayed-grant");
                }
                if want == "mismatch" || want == "ok" {
                    seen.insert(if want == "ok" { "stored" } else { "mismatch" });
                    model[i].grant = false;
                    model[i].stored = model[i].stored || want == "ok";
                }
            }
            Op::Confirm { actor, asset } if !model.is_empty() => {
                let i = asset % model.len();
                let real = confirm(&w, actor, tickets[i].asset.id);
                let want = if !in_workspace_a(actor) {
                    "no-workspace"
                } else if model[i].in_project_a && !in_project_a(actor) {
                    "no-access"
                } else if model[i].uploaded || model[i].stored {
                    "ok"
                } else {
                    "object-missing"
                };
                prop_assert_eq!(label(&real), want, "{:?}", op);
                seen.insert(match want {
                    "ok" if model[i].uploaded => "confirmed-again",
                    "ok" => "confirmed",
                    "no-access" => "confirm-refused-by-project",
                    "object-missing" => "confirm-without-object",
                    _ => "confirm-by-stranger",
                });
                if want == "ok" {
                    model[i].uploaded = true;
                }
            }
            Op::Open { actor, asset } if !model.is_empty() => {
                let i = asset % model.len();
                let real = ws_scope(&w, actor)
                    .map_err(AssetError::Core)
                    .and_then(|s| block_on(open_asset(&w.app, &s, tickets[i].asset.id)));
                let want = if !in_workspace_a(actor) {
                    "no-workspace"
                } else if model[i].in_project_a && !in_project_a(actor) {
                    "no-access"
                } else if !model[i].uploaded {
                    "not-uploaded"
                } else {
                    "ok"
                };
                prop_assert_eq!(label(&real), want, "{:?}", op);
                seen.insert(match want {
                    "ok" if model[i].in_project_a => "opened-in-project",
                    "ok" => "opened",
                    "no-access" => "open-refused-by-project",
                    "not-uploaded" => "open-before-upload",
                    _ => "open-by-stranger",
                });
            }
            Op::Attach { actor, assets } => {
                let named: Vec<usize> =
                    if model.is_empty() { vec![] } else { assets.iter().map(|a| a % model.len()).collect() };
                let actor = actor.or_else(|| named.first().map(|i| model[*i].owner)).unwrap_or(Actor::MemberA);
                let ids: Vec<AssetId> = named.iter().map(|i| tickets[*i].asset.id).collect();
                let real = block_on(attach_project_cover(&w.app, &w.principal(actor), &w.slug_a, w.project_a, &ids));
                let eligible: Vec<usize> = named.iter().copied().filter(|i| model[*i].owner == actor).collect();
                let want = if !in_workspace_a(actor) {
                    "no-workspace"
                } else if !in_project_a(actor) {
                    "not-in-project"
                } else if ids.is_empty() {
                    "no-ids"
                } else if eligible.is_empty() {
                    "no-asset"
                } else {
                    "ok"
                };
                prop_assert_eq!(label(&real), want, "{:?}", op);
                seen.insert(match want {
                    "ok" if cover.is_some_and(|c| Some(c) != eligible.iter().copied().min()) => "cover-replaced",
                    "ok" if eligible.len() < named.len() => "attached-some-left-out",
                    "ok" => "attached",
                    "no-asset" => "attach-found-nothing",
                    "not-in-project" => "attach-by-non-member",
                    _ => "attach-refused",
                });
                if want == "ok" {
                    for i in &eligible {
                        model[*i].in_project_a = true;
                    }
                    // the oldest of those it attached is the cover
                    cover = eligible.iter().copied().min();
                }
            }
            // a step that names an asset when none has been made yet does nothing
            Op::Upload { .. } | Op::Confirm { .. } | Op::Open { .. } => {}
        }

        // after every step the rows say what the reference says
        let admin = ws_scope(&w, Actor::AdminA).unwrap();
        for (i, m) in model.iter().enumerate() {
            let row = block_on(w.app.ports.db.asset(&admin, tickets[i].asset.id)).unwrap().unwrap();
            prop_assert_eq!(row.state == AssetState::Uploaded, m.uploaded, "asset {} state", i);
            prop_assert_eq!(row.upload.is_some(), m.grant && !m.uploaded, "asset {} grant", i);
            prop_assert_eq!(row.project_id == Some(w.project_a), m.in_project_a, "asset {} project", i);
            let public = block_on(public_asset(&w.app, tickets[i].asset.id));
            prop_assert_eq!(public.is_ok(), m.uploaded, "asset {} shown to anyone", i);
            if m.uploaded {
                let bytes = block_on(read_asset(&w.app, &public.unwrap())).unwrap().bytes;
                prop_assert_eq!(bytes.len(), m.size, "asset {} bytes", i);
            }
        }
        prop_assert_eq!(cover_of_project_a(&w), cover.map(|i| tickets[i].asset.id.0), "the project's cover");
    }
    Ok(())
}

/// What the sequences must get to, or the model proves less than it seems to (AGENTS.md rule 8): each is a situation where a
/// check of the use-cases is the only thing between the caller and a wrong answer.
const SITUATIONS: [&str; 17] = [
    "created",
    "stored",
    "mismatch",
    "replayed-grant",
    "confirmed",
    "confirmed-again",
    "confirm-without-object",
    "confirm-refused-by-project",
    "opened",
    "opened-in-project",
    "open-before-upload",
    "open-refused-by-project",
    "attached",
    "attached-some-left-out",
    "cover-replaced",
    "attach-found-nothing",
    "attach-by-non-member",
];

#[test]
fn model_assets_match_the_reference_over_any_sequence() {
    let seen = RefCell::new(BTreeSet::new());
    let mut runner = TestRunner::new(Config { source_file: Some(file!()), ..proptest_config(256) });
    if let Err(e) = runner.run(&ops(), |ops| run_model(ops, &mut seen.borrow_mut())) {
        panic!("{e}");
    }
    // only a run with enough cases is asked to have got everywhere (a mutation run may set `PROPTEST_CASES` low)
    if runner.config().cases >= 128 {
        let seen = seen.into_inner();
        for s in SITUATIONS {
            assert!(seen.contains(s), "no sequence got to {s:?}; they got to {seen:?}");
        }
    }
}

// the strategy reaches every kind of step
#[test]
fn model_strategy_reaches_every_step_class() {
    let mut runner = TestRunner::deterministic();
    let mut seen = BTreeSet::new();
    for _ in 0..200 {
        for op in ops().new_tree(&mut runner).unwrap().current() {
            seen.insert(match op {
                Op::Create { .. } => "create",
                Op::Upload { .. } => "upload",
                Op::Confirm { .. } => "confirm",
                Op::Open { .. } => "open",
                Op::Attach { .. } => "attach",
            });
        }
    }
    assert_eq!(seen.len(), 5, "{seen:?}");
}
