//! The API key hash hatch (Q-0052.3): keys are plain SHA-256 and stay so, and a second digest scheme can be introduced
//! later WITHOUT invalidating a key. These tests prove the mechanism with a STAND-IN scheme, version 2, an HMAC-SHA256
//! under a fixed test pepper. It exists only in this module (`api_key_hash` reaches it under `cfg(test)`), so no build
//! that ships has a second scheme and no build that ships has a pepper.
//!
//! The rules, written out as the spec the tests own:
//!
//! * a key is looked up under every version the process reads (its supported set, and always its current one) and is
//!   found only when the stored row is at the version whose digest matched;
//! * a key that is found and is stored under another version than the current one is moved to the current one after the
//!   response, through `Background`, by a compare-and-set that does nothing when the row has moved; a failed move is
//!   ignored; a key that is not used is never touched and keeps working while its version is read;
//! * a key stored under a version the process does not read does not authenticate (dropping a version retires its keys,
//!   on purpose), and neither does one that is revoked, expired or has an inactive owner;
//! * a key is created under the current version, and the shipped default is version 1, the plain SHA-256 that
//!   `hash_token` makes.

use std::collections::BTreeSet;
use std::sync::Arc;

use futures_executor::block_on;
use proptest::prelude::*;
use rstest::rstest;
use sha2::{Digest, Sha256};
use thrust_contracts::chrono::TimeDelta;

use super::*;
use crate::{hash_token, CoreConfig};

#[path = "../../tests/common/mod.rs"]
mod common;

use common::{world_with, Actor, DeferredBackground, RecordingQueue, World};

// --- the stand-in scheme --------------------------------------------------------------------------------------------

/// The one version this build knows besides 1, in tests only.
pub(super) const STAND_IN_VERSION: i16 = 2;

/// A fixed, public, throwaway pepper. Not a secret: nothing here protects anything.
const STAND_IN_PEPPER: &[u8] = b"test-only stand-in pepper; no build that ships has one";

/// HMAC-SHA256 (RFC 2104), written out so the test needs no crate the build does not already have; the vectors below
/// (RFC 4231) show it is the real thing.
fn hmac_sha256(key: &[u8], msg: &[u8]) -> [u8; 32] {
    const BLOCK: usize = 64;
    let mut k = [0u8; BLOCK];
    if key.len() > BLOCK {
        k[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        k[..key.len()].copy_from_slice(key);
    }
    let mut inner = Sha256::new();
    inner.update(k.map(|b| b ^ 0x36));
    inner.update(msg);
    let inner = inner.finalize();
    let mut outer = Sha256::new();
    outer.update(k.map(|b| b ^ 0x5c));
    outer.update(inner);
    outer.finalize().into()
}

pub(super) fn stand_in_digest(token: &str) -> TokenHash {
    TokenHash(hmac_sha256(STAND_IN_PEPPER, token.as_bytes()))
}

/// RFC 4231 test cases 1, 2 and 6 (the last has a key longer than the block): independent output the stand-in must
/// reproduce, or it is a hash of its own and proves nothing about a keyed one.
#[rstest]
#[case::rfc4231_case_1(&[0x0b; 20], b"Hi There", "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7")]
#[case::rfc4231_case_2(b"Jefe", b"what do ya want for nothing?", "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843")]
#[case::rfc4231_case_6(
    &[0xaa; 131],
    b"Test Using Larger Than Block-Size Key - Hash Key First",
    "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54"
)]
fn parity_the_stand_in_hmac_reproduces_the_rfc_4231_vectors(#[case] key: &[u8], #[case] msg: &[u8], #[case] want: &str) {
    assert_eq!(hex(&hmac_sha256(key, msg)), want);
}

// --- the scheme table -------------------------------------------------------------------------------------------------

#[test]
fn the_shipped_default_is_version_1_only() {
    assert_eq!((API_KEY_HASH_V1, API_KEY_HASH_CURRENT, API_KEY_HASH_SUPPORTED), (1, 1, &[1][..]));
    let c = CoreConfig::default();
    assert_eq!((c.api_key_hash_current, c.api_key_hash_supported), (1, vec![1]));
}

/// Independent output (`printf abc | shasum -a 256`): version 1 is the plain SHA-256 of the token, which is also what
/// `hash_token` makes, so a key stored before the hatch existed is a version 1 key.
#[test]
fn parity_version_1_is_the_plain_sha256_of_the_token() {
    assert_eq!(
        hex(&api_key_hash(1, "abc").unwrap().0),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    assert_eq!(api_key_hash(1, "thrk_x"), Some(hash_token("thrk_x")));
}

#[rstest]
fn a_version_that_has_no_scheme_has_no_digest(#[values(i16::MIN, -1, 0, 3, 4, i16::MAX)] version: i16) {
    assert_eq!(api_key_hash(version, "thrk_x"), None);
}

proptest! {
    #![proptest_config(crate::test_support::proptest_config(256))]

    /// Two schemes never agree on a token (so a digest made under one is not a match under the other), and each is a
    /// function of the token alone.
    #[test]
    fn prop_the_schemes_disagree_on_every_token_and_each_is_deterministic(token in "\\PC{0,80}") {
        let (v1, v2) = (api_key_hash(1, &token).unwrap(), api_key_hash(2, &token).unwrap());
        prop_assert_ne!(v1, v2);
        prop_assert_eq!(v1, api_key_hash(1, &token).unwrap());
        prop_assert_eq!(v2, api_key_hash(2, &token).unwrap());
    }

    /// Different tokens have different digests in each scheme.
    #[test]
    fn prop_a_scheme_gives_different_tokens_different_digests(a in "\\PC{0,40}", b in "\\PC{0,40}") {
        prop_assume!(a != b);
        for v in [1, 2] {
            prop_assert_ne!(api_key_hash(v, &a).unwrap(), api_key_hash(v, &b).unwrap(), "v{}", v);
        }
    }
}

/// The versions a lookup tries: the current one first (the common case costs one lookup), then the other supported ones
/// in ascending order, each once, the current one included whether it is listed or not. Every supported set over
/// {1, 2, 3} against every current version.
#[rstest]
fn lookup_versions_are_the_current_one_first_then_the_supported_ones_once_each(
    #[values(0u8, 1, 2, 3, 4, 5, 6, 7)] mask: u8,
    #[values(1i16, 2, 3)] current: i16,
) {
    let supported: Vec<i16> = (1..=3).filter(|v| mask & (1 << (v - 1)) != 0).collect();
    let cfg = CoreConfig { api_key_hash_current: current, api_key_hash_supported: supported.clone(), ..CoreConfig::default() };
    let got = api_key_lookup_versions(&cfg);
    let want_set: BTreeSet<i16> = supported.iter().copied().chain([current]).collect();
    assert_eq!(got.first(), Some(&current), "{supported:?} current={current}");
    assert_eq!(got.iter().copied().collect::<BTreeSet<_>>(), want_set, "{supported:?} current={current}");
    assert_eq!(got.len(), want_set.len(), "no version twice: {got:?}");
    assert!(got[1..].windows(2).all(|w| w[0] < w[1]), "the rest ascending: {got:?}");
}

// --- worlds and configurations ----------------------------------------------------------------------------------------

const OWNER: Actor = Actor::MemberA;

fn world_and_background() -> (World, Arc<DeferredBackground>) {
    let background = Arc::new(DeferredBackground::default());
    let queue: Arc<dyn Queue> = Arc::new(RecordingQueue::default());
    let w = world_with(CoreConfig { outbox_fastpath: false, ..CoreConfig::default() }, queue, background.clone());
    (w, background)
}

/// What a process can be configured with. The first is the shipped default.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Cfg {
    /// current 1, reads {1}: what ships.
    V1Only,
    /// current 1, reads {1, 2}: a build that knows the second scheme but has not switched to it.
    V1ReadingBoth,
    /// current 2, reads {1, 2}: the switch, with the old keys still honoured.
    V2ReadingBoth,
    /// current 2, reads {2}: version 1 dropped on purpose.
    V2Only,
    /// current 2, lists only {1}: a misconfiguration that must not lock out the keys the build itself writes.
    V2ListingOnlyV1,
}

const CFGS: [Cfg; 5] = [Cfg::V1Only, Cfg::V1ReadingBoth, Cfg::V2ReadingBoth, Cfg::V2Only, Cfg::V2ListingOnlyV1];

impl Cfg {
    fn current(self) -> i16 {
        match self {
            Cfg::V1Only | Cfg::V1ReadingBoth => 1,
            _ => 2,
        }
    }

    fn listed(self) -> Vec<i16> {
        match self {
            Cfg::V1Only | Cfg::V2ListingOnlyV1 => vec![1],
            Cfg::V1ReadingBoth | Cfg::V2ReadingBoth => vec![1, 2],
            Cfg::V2Only => vec![2],
        }
    }

    /// The spec: what the process reads is what it lists and what it writes.
    fn reads(self, version: i16) -> bool {
        self.listed().contains(&version) || self.current() == version
    }

    /// "The process restarts with this configuration": the same database, another `App`.
    fn app(self, w: &World) -> App {
        let mut app = w.app.clone();
        app.cfg.api_key_hash_current = self.current();
        app.cfg.api_key_hash_supported = self.listed();
        app
    }
}

fn secs(n: i64) -> TimeDelta {
    TimeDelta::seconds(n)
}

fn key_credentials(key: &str) -> Credentials<'_> {
    Credentials { api_key: Some(key), ..Credentials::default() }
}

fn use_key(app: &App, token: &str) -> Result<Authenticated, CoreError> {
    block_on(authenticate_request(app, &key_credentials(token)))
}

/// Which stored version, if any, a live key is at: the versions whose digest of `token` the database resolves.
fn stored_versions(w: &World, token: &str) -> Vec<i16> {
    let now = w.app.ports.clock.now();
    (1..=2)
        .filter(|v| {
            let got = block_on(w.db.api_key(&api_key_hash(*v, token).unwrap(), now)).unwrap();
            got.is_some_and(|k| {
                assert_eq!(k.hash_version, *v, "a row found by the version {v} digest must be stored under {v}");
                true
            })
        })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Life {
    Live,
    Revoked,
    Expired,
    OwnerInactive,
}

/// Insert a key by hand, at `stored`, in `life`: this is how a key that an earlier process made looks to this one.
fn insert_key(w: &World, token: &str, stored: i16, life: Life) -> ApiKeyId {
    let now = w.app.ports.clock.now();
    let owner = w.principal(OWNER).user_id;
    let id = ApiKeyId(Uuid::from_u128(0xC0DE_0000 + u128::from(stored.unsigned_abs())));
    let digest = api_key_hash(stored, token).unwrap_or(TokenHash([stored as u8; 32]));
    block_on(w.db.insert_api_key(&NewApiKey {
        id,
        user_id: owner,
        token_hash: digest,
        hash_version: stored,
        prefix: token.chars().take(9).collect(),
        label: "t".into(),
        created_at: now,
        expires_at: match life {
            Life::Expired => Some(now - secs(1)),
            _ => Some(now + secs(3600)),
        },
    }))
    .unwrap();
    match life {
        Life::Revoked => assert!(block_on(w.db.revoke_api_key(owner, id, now)).unwrap()),
        Life::OwnerInactive => assert!(block_on(set_user_active(&w.app, owner, false)).unwrap()),
        Life::Live | Life::Expired => {}
    }
    id
}

const TOKEN: &str = "thrk_key-under-test";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Use {
    /// Never used: the row must not be touched.
    Idle,
    /// One request, then the background work.
    Once,
    /// A request, its background work, then another request (and its background work).
    Twice,
    /// Two requests before either one's background work runs: two re-hashes race, the second must be a no-op.
    Burst,
}

// --- the mechanism: stored version x configuration x state x use ------------------------------------------------------

#[rstest]
fn a_key_is_found_under_every_version_the_process_reads_and_moved_to_the_current_one_when_used(
    #[values(1, 2)] stored: i16,
    #[values(Cfg::V1Only, Cfg::V1ReadingBoth, Cfg::V2ReadingBoth, Cfg::V2Only, Cfg::V2ListingOnlyV1)] cfg: Cfg,
    #[values(Life::Live, Life::Revoked, Life::Expired, Life::OwnerInactive)] life: Life,
    #[values(Use::Idle, Use::Once, Use::Twice, Use::Burst)] used: Use,
) {
    let (w, background) = world_and_background();
    insert_key(&w, TOKEN, stored, life);
    let app = cfg.app(&w);
    let label = format!("stored v{stored} {cfg:?} {life:?} {used:?}");

    // a key is found when it is live and its stored version is one this process reads
    let found = life == Life::Live && cfg.reads(stored);
    let check = |got: Result<Authenticated, CoreError>, when: &str| {
        if found {
            let a = got.unwrap_or_else(|e| panic!("{label} ({when}): {e}"));
            assert_eq!(a.principal, w.principal(OWNER), "{label} ({when})");
            assert!(matches!(a.via, Via::ApiKey(_)), "{label} ({when})");
        } else {
            assert!(matches!(got, Err(CoreError::InvalidApiKey)), "{label} ({when}): {got:?}");
        }
    };
    let requests = match used {
        Use::Idle => 0,
        Use::Once => 1,
        Use::Twice | Use::Burst => 2,
    };
    let mut ran = 0;
    for i in 0..requests {
        check(use_key(&app, TOKEN), &format!("request {i}"));
        if used != Use::Burst {
            ran += background.run_all();
        }
    }
    ran += background.run_all();

    // what stays and what moves
    let moved = found && used != Use::Idle && stored != cfg.current();
    if life == Life::Live {
        let at = stored_versions(&w, TOKEN);
        let want = if moved { cfg.current() } else { stored };
        assert_eq!(at, vec![want], "{label}: the one digest the database resolves");
    }
    if !found {
        assert_eq!(ran, 0, "{label}: a key that is refused spawns no background work");
    }
    if used == Use::Idle {
        assert_eq!(ran, 0, "{label}: an unused key is not touched");
    }
    // whatever happened, a live key that the process reads still works afterwards, and so does it under the config that
    // wrote it (when that reads its new version): nothing was lost by the move
    check(use_key(&app, TOKEN), "afterwards");
    background.run_all();
    if found && moved {
        // the moved key is not found under a process that reads only the old version: the digests differ
        let old_only = Cfg::V1Only.app(&w);
        let got = use_key(&old_only, TOKEN);
        let wants_old = cfg.current() == 1;
        assert_eq!(got.is_ok(), wants_old, "{label}: a process that reads only v1");
    }
}

/// Once moved, a key is at the current version and fresh: its next use finds it at once and writes nothing.
#[rstest]
fn a_moved_key_costs_no_background_write_on_its_next_use(#[values(Cfg::V2ReadingBoth, Cfg::V2ListingOnlyV1)] cfg: Cfg) {
    let (w, background) = world_and_background();
    insert_key(&w, TOKEN, 1, Life::Live);
    let app = cfg.app(&w);
    // `V2ListingOnlyV1` reads {1, 2} too (the current version is always read), so both move
    use_key(&app, TOKEN).unwrap();
    assert_eq!(background.run_all(), 1, "the first use stamps and moves in one background task");
    assert_eq!(stored_versions(&w, TOKEN), vec![2]);
    use_key(&app, TOKEN).unwrap();
    assert_eq!(background.run_all(), 0, "the second use writes nothing");
}

/// Two requests decide to move the same key before either has written: the second write finds the row moved and does
/// nothing, and the key is intact.
#[test]
fn two_racing_moves_leave_one_move_and_a_working_key() {
    let (w, background) = world_and_background();
    insert_key(&w, TOKEN, 1, Life::Live);
    let app = Cfg::V2ReadingBoth.app(&w);
    use_key(&app, TOKEN).unwrap();
    use_key(&app, TOKEN).unwrap();
    assert_eq!(background.run_all(), 2, "both requests spawned their write");
    assert_eq!(stored_versions(&w, TOKEN), vec![2]);
    use_key(&app, TOKEN).unwrap();
}

/// A write that fails is ignored, like `last_used_at`: the response was already sent, and the key stays at version 1 and
/// works on the next request. The failure is made with a row that holds the digest the move wants.
#[test]
fn a_failed_move_is_ignored_and_the_key_keeps_working() {
    let (w, background) = world_and_background();
    insert_key(&w, TOKEN, 1, Life::Live);
    // a revoked row already holds the version 2 digest of the token (a collision no real token produces): it is not found,
    // so the real key is, at version 1, and the UNIQUE constraint then refuses the move to version 2
    let owner = w.principal(OWNER).user_id;
    block_on(w.db.insert_api_key(&NewApiKey {
        id: ApiKeyId(Uuid::from_u128(0xDEC0)),
        user_id: owner,
        token_hash: api_key_hash(2, TOKEN).unwrap(),
        hash_version: 2,
        prefix: "thrk_deco".into(),
        label: "decoy".into(),
        created_at: w.app.ports.clock.now(),
        expires_at: None,
    }))
    .unwrap();
    let app = Cfg::V2ReadingBoth.app(&w);
    assert!(block_on(w.db.revoke_api_key(owner, ApiKeyId(Uuid::from_u128(0xDEC0)), w.app.ports.clock.now())).unwrap());
    let a = use_key(&app, TOKEN).unwrap();
    assert!(matches!(a.via, Via::ApiKey(id) if id == ApiKeyId(Uuid::from_u128(0xC0DE_0001))), "the real key, not the decoy");
    assert_eq!(background.run_all(), 1);
    assert_eq!(stored_versions(&w, TOKEN), vec![1], "the move failed, the key is where it was");
    use_key(&app, TOKEN).unwrap();
}

// --- creating --------------------------------------------------------------------------------------------------------

#[rstest]
fn a_new_key_is_stored_under_the_current_version_and_only_that_one(
    #[values(Cfg::V1Only, Cfg::V1ReadingBoth, Cfg::V2ReadingBoth, Cfg::V2Only, Cfg::V2ListingOnlyV1)] cfg: Cfg,
) {
    let (w, background) = world_and_background();
    let app = cfg.app(&w);
    let who = w.principal(OWNER);
    let k = block_on(create_api_key(&app, &who, CreateApiKeyRequest::default())).unwrap();
    assert_eq!(stored_versions(&w, &k.token), vec![cfg.current()], "{cfg:?}");
    // it works at once, and needs no move
    use_key(&app, &k.token).unwrap();
    background.run_all();
    assert_eq!(stored_versions(&w, &k.token), vec![cfg.current()], "{cfg:?}");
}

/// The default is unchanged: a key made with the shipped configuration is stored as `hash_token(token)`, version 1.
#[test]
fn the_default_configuration_stores_the_plain_sha256_at_version_1() {
    let (w, _) = world_and_background();
    let k = block_on(create_api_key(&w.app, &w.principal(OWNER), CreateApiKeyRequest::default())).unwrap();
    let live = block_on(w.db.api_key(&hash_token(&k.token), w.app.ports.clock.now())).unwrap().unwrap();
    assert_eq!((live.id, live.hash_version), (k.id, 1));
}

/// A configuration with a current version this build has no scheme for cannot create a key, and says so rather than
/// storing an unusable one.
#[rstest]
fn a_current_version_without_a_scheme_refuses_to_create_a_key(#[values(0, 3, -1)] current: i16) {
    let (w, _) = world_and_background();
    let mut app = w.app.clone();
    app.cfg.api_key_hash_current = current;
    let got = block_on(create_api_key(&app, &w.principal(OWNER), CreateApiKeyRequest::default()));
    assert!(matches!(got, Err(CoreError::Internal(_))), "{got:?}");
    assert!(block_on(list_api_keys(&app, &w.principal(OWNER))).unwrap().is_empty());
}

// --- a digest belongs to the scheme that made it ------------------------------------------------------------------------

/// A row found by a digest must be stored under the version that digest was made with. A row that says version 2 but
/// holds the version 1 digest of a token (or the reverse, or a version this build does not know) is not a match.
#[rstest]
fn a_row_matches_only_when_its_stored_version_is_the_scheme_that_made_its_digest(
    #[values(1, 2)] digest_of: i16,
    #[values(1, 2, 3)] stored: i16,
    #[values(Cfg::V1Only, Cfg::V1ReadingBoth, Cfg::V2ReadingBoth, Cfg::V2Only, Cfg::V2ListingOnlyV1)] cfg: Cfg,
) {
    let (w, background) = world_and_background();
    let owner = w.principal(OWNER).user_id;
    let now = w.app.ports.clock.now();
    let id = ApiKeyId(Uuid::from_u128(0xC0DE_1000));
    block_on(w.db.insert_api_key(&NewApiKey {
        id,
        user_id: owner,
        token_hash: api_key_hash(digest_of, TOKEN).unwrap(),
        hash_version: stored,
        prefix: "thrk_mism".into(),
        label: "t".into(),
        created_at: now,
        expires_at: None,
    }))
    .unwrap();
    let got = use_key(&cfg.app(&w), TOKEN);
    let want = stored == digest_of && cfg.reads(stored);
    assert_eq!(got.is_ok(), want, "digest of v{digest_of}, stored v{stored}, {cfg:?}: {got:?}");
    if !want {
        assert!(matches!(got, Err(CoreError::InvalidApiKey)));
        assert_eq!(background.run_all(), 0, "a refused key moves nothing");
    }
}

// --- a model of the whole life ---------------------------------------------------------------------------------------

const KEYS: usize = 3;

#[derive(Debug, Clone, Copy)]
enum Op {
    /// Create key `key` under the process's current configuration (a no-op when it exists already).
    Create { key: usize },
    /// Restart the process with this configuration.
    Restart { cfg: Cfg },
    Use { key: usize },
    Revoke { key: usize },
    /// Run the background work that requests have spawned so far.
    Background,
}

fn cfg() -> impl Strategy<Value = Cfg> {
    prop::sample::select(CFGS.to_vec())
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        4 => (0..KEYS).prop_map(|key| Op::Create { key }),
        3 => cfg().prop_map(|cfg| Op::Restart { cfg }),
        6 => (0..KEYS).prop_map(|key| Op::Use { key }),
        1 => (0..KEYS).prop_map(|key| Op::Revoke { key }),
        3 => Just(Op::Background),
    ]
}

fn ops() -> impl Strategy<Value = Vec<Op>> {
    prop::collection::vec(op(), 1..=24)
}

/// What the planner saw. The generator coverage control requires every one of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Class {
    Created,
    /// A key made under version 2.
    CreatedAtV2,
    /// A use that found a key at the current version.
    UsedCurrent,
    /// A use that found a key at an older version and decided to move it.
    UsedOlderAndMoves,
    /// A use that did not find a live key because its version is not read.
    UnreadVersionRefused,
    UsedRevokedRefused,
    UsedUnknownRefused,
    /// Background work that moved a key.
    Moved,
    /// Background work that found its row already moved: the compare-and-set did nothing.
    MoveLostTheRace,
    /// Background work for a key that was revoked in the meantime: the move still lands, the key stays revoked.
    MovedARevokedKey,
}

impl Class {
    const ALL: [Class; 10] = [
        Class::Created,
        Class::CreatedAtV2,
        Class::UsedCurrent,
        Class::UsedOlderAndMoves,
        Class::UnreadVersionRefused,
        Class::UsedRevokedRefused,
        Class::UsedUnknownRefused,
        Class::Moved,
        Class::MoveLostTheRace,
        Class::MovedARevokedKey,
    ];
}

#[derive(Debug, Clone, Copy)]
struct KeyModel {
    version: i16,
    revoked: bool,
}

#[derive(Debug)]
struct Model {
    cfg: Cfg,
    keys: [Option<KeyModel>; KEYS],
    /// Moves that requests decided on and that have not run: `(key, from, to)`.
    pending: Vec<(usize, i16, i16)>,
}

/// What the real process must answer for an op.
#[derive(Debug, PartialEq, Eq)]
enum Expect {
    Nothing,
    Authenticates,
    Refused,
}

impl Model {
    fn new() -> Self {
        Model { cfg: Cfg::V1Only, keys: [None; KEYS], pending: Vec::new() }
    }

    fn apply(&mut self, op: Op, classes: &mut Vec<Class>) -> Expect {
        match op {
            Op::Create { key } => {
                if self.keys[key].is_none() {
                    self.keys[key] = Some(KeyModel { version: self.cfg.current(), revoked: false });
                    classes.push(Class::Created);
                    if self.cfg.current() == 2 {
                        classes.push(Class::CreatedAtV2);
                    }
                }
                Expect::Nothing
            }
            Op::Restart { cfg } => {
                self.cfg = cfg;
                Expect::Nothing
            }
            Op::Revoke { key } => {
                if let Some(k) = &mut self.keys[key] {
                    k.revoked = true;
                }
                Expect::Nothing
            }
            Op::Use { key } => match self.keys[key] {
                None => {
                    classes.push(Class::UsedUnknownRefused);
                    Expect::Refused
                }
                Some(k) if k.revoked => {
                    classes.push(Class::UsedRevokedRefused);
                    Expect::Refused
                }
                Some(k) if !self.cfg.reads(k.version) => {
                    classes.push(Class::UnreadVersionRefused);
                    Expect::Refused
                }
                Some(k) => {
                    if k.version == self.cfg.current() {
                        classes.push(Class::UsedCurrent);
                    } else {
                        classes.push(Class::UsedOlderAndMoves);
                        self.pending.push((key, k.version, self.cfg.current()));
                    }
                    Expect::Authenticates
                }
            },
            Op::Background => {
                for (key, from, to) in std::mem::take(&mut self.pending) {
                    let Some(k) = &mut self.keys[key] else { continue };
                    if k.version == from {
                        k.version = to;
                        classes.push(if k.revoked { Class::MovedARevokedKey } else { Class::Moved });
                    } else {
                        classes.push(Class::MoveLostTheRace);
                    }
                }
                Expect::Nothing
            }
        }
    }
}

proptest! {
    #![proptest_config(crate::test_support::proptest_config(256))]

    /// The real process, against the memory adapters, agrees with the model after every operation: who authenticates, and
    /// under which version each live key is stored.
    #[test]
    fn model_api_key_versions_follow_the_reference_model(ops in ops()) {
        let (w, background) = world_and_background();
        let owner = w.principal(OWNER);
        let mut model = Model::new();
        let mut app = Cfg::V1Only.app(&w);
        let mut tokens: [Option<(String, ApiKeyId)>; KEYS] = [None, None, None];
        for (i, op) in ops.iter().copied().enumerate() {
            let before = format!("{:?}", model);
            let expect = model.apply(op, &mut Vec::new());
            let label = format!("op {i} {op:?}, model before: {before}");
            match op {
                Op::Create { key } => {
                    if tokens[key].is_none() {
                        let k = block_on(create_api_key(&app, &owner, CreateApiKeyRequest::default())).unwrap();
                        tokens[key] = Some((k.token, k.id));
                    }
                }
                Op::Restart { cfg } => app = cfg.app(&w),
                Op::Revoke { key } => {
                    if let Some((_, id)) = &tokens[key] {
                        block_on(revoke_api_key(&app, &owner, *id)).unwrap();
                    }
                }
                Op::Use { key } => {
                    let token = tokens[key].as_ref().map_or("thrk_never-made", |t| t.0.as_str());
                    let got = use_key(&app, token);
                    match expect {
                        Expect::Authenticates => prop_assert!(got.is_ok(), "{}: {:?}", label, got),
                        Expect::Refused => prop_assert!(matches!(got, Err(CoreError::InvalidApiKey)), "{}: {:?}", label, got),
                        Expect::Nothing => unreachable!(),
                    }
                }
                Op::Background => {
                    background.run_all();
                }
            }
            // every key that is not revoked is stored under exactly the version the model says
            for (key, m) in model.keys.iter().enumerate() {
                let (Some(m), Some((token, _))) = (m, &tokens[key]) else { continue };
                if m.revoked {
                    continue;
                }
                prop_assert_eq!(stored_versions(&w, token), vec![m.version], "{} key {}", label, key);
            }
        }
    }
}

/// Rule 8 for the property above: the strategy, at the property's own case budget, reaches every class of step, and
/// every class that the real process's answer depends on.
#[test]
fn model_strategy_reaches_every_step_class() {
    use proptest::strategy::ValueTree;
    use proptest::test_runner::TestRunner;
    const CASES: usize = 256;
    let mut runner = TestRunner::deterministic();
    let mut seen = BTreeSet::new();
    for _ in 0..CASES {
        let sequence = ops().new_tree(&mut runner).unwrap().current();
        let mut model = Model::new();
        let mut classes = Vec::new();
        for op in sequence {
            model.apply(op, &mut classes);
        }
        seen.extend(classes);
    }
    let missing: Vec<_> = Class::ALL.iter().filter(|c| !seen.contains(*c)).collect();
    assert!(missing.is_empty(), "never generated in {CASES} sequences: {missing:?}");
}
