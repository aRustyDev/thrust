//! Workspaces (issue #44): the slug and name rules, making a workspace, the caller's list and the one the app opens, and the
//! workspace-level reads behind `authorize_workspace`, against the memory adapters.
//!
//! What must hold: a slug is a short run of letters, digits, hyphens and underscores that is not reserved and not taken; a
//! name has a letter or number and no URL; a refused creation writes nothing; the creator is the admin and the only member;
//! a person sees their own workspaces, newest first, and only while they are active members; the workspace the app opens is
//! the last one they used if they still belong to it and the first one they joined otherwise; and a workspace's members,
//! the caller's membership and the caller's project roles come only through a scope that proves membership of THAT
//! workspace, never another's.

mod common;

use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;

use common::*;
use futures_executor::block_on;
use proptest::prelude::*;
use rstest::rstest;
use thrust_contracts::chrono::DateTime;
use thrust_contracts::*;
use thrust_core::*;
use thrust_ports::*;

const PASSWORD: &str = "correct horse battery staple";

/// A clock that moves one second on every reading, so workspaces made one after another differ in `created_at`.
struct TickClock(AtomicI64);
impl Clock for TickClock {
    fn now(&self) -> Timestamp {
        DateTime::from_timestamp(1_760_000_000 + self.0.fetch_add(1, Ordering::Relaxed), 0).unwrap()
    }
}

/// The default world with a ticking clock, and a signed-up user to act as.
fn ticking() -> (World, App, Principal) {
    let w = world();
    let mut app = w.app.clone();
    app.ports.clock = Arc::new(TickClock(AtomicI64::new(0)));
    let who = signed_up(&app, "w@example.test");
    (w, app, who)
}

fn signed_up(app: &App, email: &str) -> Principal {
    Principal { user_id: block_on(sign_up(app, email, PASSWORD)).unwrap().user_id }
}

fn request(name: &str, slug: &str) -> NewWorkspaceRequest {
    NewWorkspaceRequest { name: name.into(), slug: slug.into(), organization_size: None, company_role: None, timezone: None }
}

fn make(app: &App, who: &Principal, name: &str, slug: &str) -> WorkspaceListing {
    block_on(create_workspace(app, who, request(name, slug))).unwrap()
}

// --- the slug and name rules ---------------------------------------------------------------------------------------------

const SLUG_CHARS: &str = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-_";

fn slug() -> impl Strategy<Value = String> {
    prop::collection::vec(prop::sample::select(SLUG_CHARS.chars().collect::<Vec<_>>()), 1..=48).prop_map(|cs| cs.into_iter().collect())
}

proptest! {
    #![proptest_config(proptest_config(256))]

    #[test]
    fn prop_a_slug_of_the_right_characters_is_valid(s in slug()) {
        prop_assert!(slug_has_valid_characters(&s), "{s}");
    }

    /// Control: one character outside the set, anywhere, makes any of them invalid.
    #[test]
    fn prop_one_foreign_character_anywhere_makes_a_slug_invalid(
        s in slug(),
        at in 0usize..60,
        bad in prop::sample::select(vec![' ', '.', '/', '@', '!', '\u{e9}', '\n', '\u{4e2d}', '+']),
    ) {
        let mut cs: Vec<char> = s.chars().collect();
        cs.insert(at % (cs.len() + 1), bad);
        let broken: String = cs.into_iter().collect();
        prop_assert!(!slug_has_valid_characters(&broken), "{broken:?}");
    }

    /// A name that holds a letter or a digit of any script is a name, wherever it is; one of only symbols and spaces is not.
    #[test]
    fn prop_a_name_needs_a_letter_or_number_of_any_script(
        pad in "[ !-/:-@\\[-`{-~]{0,6}",
        tail in "[ !-/:-@\\[-`{-~]{0,6}",
        keep in prop::sample::select(vec!['a', 'Z', '7', '\u{e9}', '\u{4e2d}', '\u{436}', '\u{663}', '\u{d55c}']),
    ) {
        prop_assert!(name_has_alphanumeric(&format!("{pad}{keep}{tail}")), "{pad:?}{keep}{tail:?}");
        prop_assert!(!name_has_alphanumeric(&format!("{pad}{tail}")), "{pad:?}{tail:?}");
    }
}

/// Rule 8 for the slug properties: the strategy reaches the length bounds and every kind of character.
#[test]
fn prop_slug_strategy_reaches_every_input_class() {
    use proptest::strategy::ValueTree;
    use proptest::test_runner::TestRunner;
    let mut runner = TestRunner::deterministic();
    let (mut one, mut long, mut hyphen, mut underscore, mut digit, mut upper) = (false, false, false, false, false, false);
    for _ in 0..512 {
        let s = slug().new_tree(&mut runner).unwrap().current();
        one |= s.len() == 1;
        long |= s.len() >= 40;
        hyphen |= s.contains('-');
        underscore |= s.contains('_');
        digit |= s.chars().any(|c| c.is_ascii_digit());
        upper |= s.chars().any(|c| c.is_ascii_uppercase());
    }
    assert!([one, long, hyphen, underscore, digit, upper].iter().all(|r| *r), "[one, long, hyphen, underscore, digit, upper]: {:?}", [one, long, hyphen, underscore, digit, upper]);
}

/// Spec examples: the reserved names the fixtures use, and what is not reserved.
#[rstest]
#[case("onboarding", true)]
#[case("api", true)]
#[case("404", true)]
#[case("create-workspace", true)]
#[case("m", true)]
#[case("Onboarding", false)]
#[case("onboarding1", false)]
#[case("api-", false)]
#[case("journey-ws", false)]
#[case("", false)]
fn reserved_slugs(#[case] slug: &str, #[case] reserved: bool) {
    assert_eq!(is_reserved_slug(slug), reserved, "{slug:?}");
}

#[rstest]
#[case("", false)]
#[case("a b", false)]
#[case("a.b", false)]
#[case("a-b_c", true)]
#[case("A1", true)]
#[case("caf\u{e9}", false)]
fn slug_characters(#[case] slug: &str, #[case] ok: bool) {
    assert_eq!(slug_has_valid_characters(slug), ok, "{slug:?}");
}

// --- making a workspace --------------------------------------------------------------------------------------------------

#[test]
fn the_creator_is_the_admin_and_the_only_member_and_the_slug_is_then_taken() {
    let (_w, app, who) = ticking();
    let made = block_on(create_workspace(
        &app,
        &who,
        NewWorkspaceRequest {
            name: "Acme".into(),
            slug: "acme".into(),
            organization_size: Some("2-10".into()),
            company_role: Some("engineer".into()),
            timezone: Some("Asia/Kolkata".into()),
        },
    ))
    .unwrap();
    assert_eq!((made.role, made.total_members), (Role::Admin, 1));
    let w = &made.workspace;
    assert_eq!((w.name.as_str(), w.slug.as_str(), w.owner_id, w.organization_size.as_deref(), w.timezone.as_str()), ("Acme", "acme", Some(who.user_id), Some("2-10"), "Asia/Kolkata"));
    assert!(w.background_color.starts_with('#') && w.background_color.len() == 7, "{}", w.background_color);
    // it is in the creator's list with the same role, and the creator's scope sees themselves as the only member
    let mine = block_on(my_workspaces(&app, &who)).unwrap();
    assert_eq!(mine, vec![made.clone()]);
    let scope = block_on(authorize_workspace(&app, &who, &WorkspaceSlug::new("acme"))).unwrap();
    assert_eq!(scope.role(), Role::Admin);
    let me = block_on(my_membership(&app, &scope)).unwrap();
    assert_eq!((me.user_id, me.role, me.is_active, me.company_role.as_deref()), (who.user_id, Role::Admin, true, Some("engineer")));
    let all = block_on(members(&app, &scope)).unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].user.email.as_deref(), Some("w@example.test"));
    // and the slug is taken, in this spelling only
    assert!(!block_on(slug_available(&app, "acme")).unwrap());
    assert!(block_on(slug_available(&app, "Acme")).unwrap(), "slugs are case sensitive");
}

#[derive(Debug, Clone, Copy)]
enum Refusal {
    EmptyName,
    LongName,
    UrlInName,
    SymbolsOnlyName,
    EmptySlug,
    LongSlug,
    SlugWithSpace,
    ReservedSlug,
    LongSize,
    UnknownZone,
}

#[rstest]
fn a_refused_creation_writes_nothing(
    #[values(
        Refusal::EmptyName, Refusal::LongName, Refusal::UrlInName, Refusal::SymbolsOnlyName, Refusal::EmptySlug,
        Refusal::LongSlug, Refusal::SlugWithSpace, Refusal::ReservedSlug, Refusal::LongSize, Refusal::UnknownZone
    )]
    why: Refusal,
) {
    let (_w, app, who) = ticking();
    let mut r = request("Fine", "fine-slug");
    match why {
        Refusal::EmptyName => r.name = String::new(),
        Refusal::LongName => r.name = "x".repeat(MAX_WORKSPACE_NAME + 1),
        Refusal::UrlInName => r.name = "see example.com".into(),
        Refusal::SymbolsOnlyName => r.name = "!!!".into(),
        Refusal::EmptySlug => r.slug = String::new(),
        Refusal::LongSlug => r.slug = "s".repeat(MAX_WORKSPACE_SLUG + 1),
        Refusal::SlugWithSpace => r.slug = "a b".into(),
        Refusal::ReservedSlug => r.slug = "onboarding".into(),
        Refusal::LongSize => r.organization_size = Some("x".repeat(MAX_ORGANIZATION_SIZE + 1)),
        Refusal::UnknownZone => r.timezone = Some("Mars/Olympus".into()),
    }
    let slug = r.slug.clone();
    assert!(matches!(block_on(create_workspace(&app, &who, r)), Err(CoreError::Invalid(_))), "{why:?}");
    assert!(block_on(my_workspaces(&app, &who)).unwrap().is_empty(), "{why:?}: nothing was made");
    if !slug.is_empty() && !is_reserved_slug(&slug) {
        assert!(block_on(slug_available(&app, &slug)).unwrap() || slug.contains(' '), "{why:?}: the slug is still free");
    }
    // control: the same request with the fault taken out is accepted
    assert!(block_on(create_workspace(&app, &who, request("Fine", "fine-slug"))).is_ok());
}

#[test]
fn the_limits_are_exact() {
    let (_w, app, who) = ticking();
    let exact = block_on(create_workspace(
        &app,
        &who,
        NewWorkspaceRequest {
            name: "n".repeat(MAX_WORKSPACE_NAME),
            slug: "s".repeat(MAX_WORKSPACE_SLUG),
            organization_size: Some("o".repeat(MAX_ORGANIZATION_SIZE)),
            company_role: None,
            timezone: None,
        },
    ));
    assert!(exact.is_ok(), "{exact:?}");
    assert_eq!(MAX_WORKSPACE_NAME, 80);
    assert_eq!(MAX_WORKSPACE_SLUG, 48);
    assert_eq!(MAX_ORGANIZATION_SIZE, 20);
    // characters, not bytes
    assert!(block_on(create_workspace(&app, &who, request(&"\u{e9}".repeat(MAX_WORKSPACE_NAME), "multibyte"))).is_ok());
}

#[test]
fn a_slug_that_is_taken_is_a_conflict_and_the_first_workspace_is_untouched() {
    let (_w, app, who) = ticking();
    let other = signed_up(&app, "o@example.test");
    let first = make(&app, &who, "First", "taken");
    let again = block_on(create_workspace(&app, &other, request("Second", "taken")));
    assert!(matches!(again, Err(CoreError::Conflict(_))), "{again:?}");
    assert!(block_on(my_workspaces(&app, &other)).unwrap().is_empty(), "the loser has no workspace and no membership");
    assert_eq!(block_on(my_workspaces(&app, &who)).unwrap(), vec![first], "the winner's is as it was");
    // the same person taking their own slug again is refused too
    assert!(matches!(block_on(create_workspace(&app, &who, request("Third", "taken"))), Err(CoreError::Conflict(_))));
}

#[test]
fn a_caller_whose_user_is_gone_cannot_make_a_workspace() {
    let (_w, app, _who) = ticking();
    let ghost = Principal { user_id: UserId(Uuid::from_u128(0xDEAD)) };
    assert!(matches!(block_on(create_workspace(&app, &ghost, request("Ghost", "ghost"))), Err(CoreError::Unauthenticated)));
    assert!(block_on(slug_available(&app, "ghost")).unwrap(), "nothing was made for them: the slug is still free");
}

// --- listing, and which workspace the app opens --------------------------------------------------------------------------

#[test]
fn the_list_is_newest_first_and_each_person_sees_only_their_own() {
    let (_w, app, who) = ticking();
    let other = signed_up(&app, "o@example.test");
    for (name, slug) in [("Alpha", "alpha"), ("Beta", "beta"), ("Gamma", "gamma")] {
        make(&app, &who, name, slug);
    }
    make(&app, &other, "Theirs", "theirs");
    let slugs = |p: &Principal| block_on(my_workspaces(&app, p)).unwrap().into_iter().map(|l| l.workspace.slug.to_string()).collect::<Vec<_>>();
    assert_eq!(slugs(&who), ["gamma", "beta", "alpha"]);
    assert_eq!(slugs(&other), ["theirs"]);
}

#[test]
fn workspaces_made_in_the_same_second_are_ordered_by_id_newest_first() {
    // the fixed clock gives every workspace one `created_at`, so only the tie-break orders them
    let w = world();
    let who = signed_up(&w.app, "t@example.test");
    for slug in ["a1", "a2", "a3"] {
        make(&w.app, &who, slug, slug);
    }
    let list = block_on(my_workspaces(&w.app, &who)).unwrap();
    assert_eq!(list.iter().map(|l| l.workspace.slug.to_string()).collect::<Vec<_>>(), ["a3", "a2", "a1"]);
}

fn choice_slugs(app: &App, who: &Principal) -> (Option<String>, Option<String>) {
    let me = block_on(thrust_core::me(app, who)).unwrap();
    let c = block_on(workspace_choice(app, &me)).unwrap();
    (c.last.map(|w| w.slug.to_string()), c.fallback.map(|w| w.slug.to_string()))
}

fn set_last(app: &App, who: &Principal, id: Option<WorkspaceId>) {
    block_on(update_me(app, who, ProfilePatch { last_workspace_id: Some(id), ..ProfilePatch::default() })).unwrap();
}

#[test]
fn the_app_opens_the_last_workspace_if_still_a_member_and_the_first_joined_otherwise() {
    let (_w, app, who) = ticking();
    // no workspace at all
    assert_eq!(choice_slugs(&app, &who), (None, None));
    let first = make(&app, &who, "Alpha", "alpha");
    let second = make(&app, &who, "Beta", "beta");
    // none used yet: the earliest made is the fallback, whichever is newer in the list
    assert_eq!(choice_slugs(&app, &who), (None, Some("alpha".into())));
    // the last one used, and it is also the fallback
    set_last(&app, &who, Some(second.workspace.id));
    assert_eq!(choice_slugs(&app, &who), (Some("beta".into()), Some("beta".into())));
    set_last(&app, &who, Some(first.workspace.id));
    assert_eq!(choice_slugs(&app, &who), (Some("alpha".into()), Some("alpha".into())));
    // a last workspace nobody belongs to, or that does not exist, is as good as none
    let stranger = signed_up(&app, "s@example.test");
    let theirs = make(&app, &stranger, "Theirs", "theirs");
    set_last(&app, &who, Some(theirs.workspace.id));
    assert_eq!(choice_slugs(&app, &who), (None, Some("alpha".into())), "not a member of the one it names");
    set_last(&app, &who, Some(WorkspaceId(Uuid::from_u128(0x404))));
    assert_eq!(choice_slugs(&app, &who), (None, Some("alpha".into())), "it names nothing");
    // and a member who has been deactivated no longer has it
    set_last(&app, &who, Some(second.workspace.id));
    block_on(app.ports.db.set_workspace_member_active(&system(&app), second.workspace.id, who.user_id, false)).unwrap();
    assert_eq!(choice_slugs(&app, &who), (None, Some("alpha".into())), "deactivated in the one it names");
}

/// `SystemScope` is minted by the composition roots; the test stands in for one, as `thrust-core`'s own tests do.
fn system(_app: &App) -> SystemScope {
    SystemScope::mint()
}

// --- the scope, and the reads behind it ------------------------------------------------------------------------------------

#[test]
fn a_scope_needs_an_active_membership_of_that_very_workspace() {
    let (_w, app, who) = ticking();
    let other = signed_up(&app, "o@example.test");
    let mine = make(&app, &who, "Mine", "mine");
    let slug = |s: &str| WorkspaceSlug::new(s);
    // a member: a scope with their role
    let scope = block_on(authorize_workspace(&app, &who, &slug("mine"))).unwrap();
    assert_eq!((scope.workspace_id(), scope.user_id(), scope.role()), (mine.workspace.id, who.user_id, Role::Admin));
    // a slug that names nothing, and a workspace that is someone else's, are the same answer
    for s in ["no-such-workspace", "Mine", ""] {
        assert!(matches!(block_on(authorize_workspace(&app, &who, &slug(s))), Err(CoreError::NotFound)), "{s:?}");
    }
    assert!(matches!(block_on(authorize_workspace(&app, &other, &slug("mine"))), Err(CoreError::NotFound)));
    // a deactivated member is no longer one, and a reactivated one is again
    let sys = SystemScope::mint();
    assert!(block_on(app.ports.db.set_workspace_member_active(&sys, mine.workspace.id, who.user_id, false)).unwrap());
    assert!(matches!(block_on(authorize_workspace(&app, &who, &slug("mine"))), Err(CoreError::NotFound)));
    assert!(block_on(my_workspaces(&app, &who)).unwrap().is_empty(), "and it leaves their list");
    assert!(block_on(app.ports.db.set_workspace_member_active(&sys, mine.workspace.id, who.user_id, true)).unwrap());
    assert!(block_on(authorize_workspace(&app, &who, &slug("mine"))).is_ok());
    // a membership that does not exist cannot be switched
    assert!(!block_on(app.ports.db.set_workspace_member_active(&sys, mine.workspace.id, other.user_id, false)).unwrap());
}

#[test]
fn a_deactivated_member_still_shows_in_the_member_list_but_is_not_counted() {
    let (_w, app, who) = ticking();
    let mine = make(&app, &who, "Mine", "mine");
    let scope = block_on(authorize_workspace(&app, &who, &WorkspaceSlug::new("mine"))).unwrap();
    let sys = SystemScope::mint();
    // a second member, by seed-style insertion through the adapter
    let other = signed_up(&app, "o@example.test");
    let seed = SeedFixture {
        workspace_members: vec![SeedWorkspaceMember { workspace_id: mine.workspace.id, user_id: other.user_id }],
        ..SeedFixture::default()
    };
    block_on(app.ports.db.seed(&sys, &seed)).unwrap();
    assert_eq!(block_on(my_workspaces(&app, &who)).unwrap()[0].total_members, 2);
    block_on(app.ports.db.set_workspace_member_active(&sys, mine.workspace.id, other.user_id, false)).unwrap();
    assert_eq!(block_on(my_workspaces(&app, &who)).unwrap()[0].total_members, 1, "not counted");
    let list = block_on(members(&app, &scope)).unwrap();
    assert_eq!(list.len(), 2, "but listed");
    assert!(list.iter().any(|r| r.member.user_id == other.user_id && !r.member.is_active));
}

/// The default world has two tenants: workspace A with five members and one project, workspace B with two members and one.
#[test]
fn the_reads_behind_a_scope_show_that_workspace_and_no_other() {
    let w = world();
    let admin_a = w.principal(Actor::AdminA);
    let scope_a = block_on(authorize_workspace(&w.app, &admin_a, &w.slug_a)).unwrap();
    // a member list holds exactly workspace A's people
    let people: std::collections::BTreeSet<UserId> = block_on(members(&w.app, &scope_a)).unwrap().iter().map(|r| r.member.user_id).collect();
    let want_a: std::collections::BTreeSet<UserId> = [Actor::AdminA, Actor::MemberA, Actor::GuestA, Actor::OutsiderA, Actor::Dual].iter().map(|a| w.principal(*a).user_id).collect();
    assert_eq!(people, want_a);
    assert!(!people.contains(&w.principal(Actor::MemberB).user_id), "workspace B's member is not in A's list");
    // the caller's own roles are the roles in THIS workspace's projects: Dual is a member of A's project and a guest of B's
    let dual = w.principal(Actor::Dual);
    let in_a = block_on(authorize_workspace(&w.app, &dual, &w.slug_a)).unwrap();
    let in_b = block_on(authorize_workspace(&w.app, &dual, &w.slug_b)).unwrap();
    assert_eq!(block_on(my_project_roles(&w.app, &in_a)).unwrap(), vec![(w.project_a, Role::Member)]);
    assert_eq!(block_on(my_project_roles(&w.app, &in_b)).unwrap(), vec![(w.project_b, Role::Guest)]);
    // a member of the workspace who belongs to no project has no roles
    let outsider = w.principal(Actor::OutsiderA);
    let scope = block_on(authorize_workspace(&w.app, &outsider, &w.slug_a)).unwrap();
    assert!(block_on(my_project_roles(&w.app, &scope)).unwrap().is_empty());
    // the caller's membership row is theirs
    let me = block_on(my_membership(&w.app, &in_a)).unwrap();
    assert_eq!((me.user_id, me.workspace_id), (dual.user_id, w.ws_a));
    // a seeded membership is an active member, with the seed's name for the workspace
    assert!(me.is_active);
    assert_eq!(block_on(my_workspaces(&w.app, &dual)).unwrap().len(), 2, "Dual belongs to both");
}

/// A world with nothing in it, not even the two seeded workspaces of `world()`.
fn empty_app() -> App {
    use thrust_adapters_memory::*;
    let background: Arc<dyn Background> = Arc::new(NoBackground);
    let queue: Arc<dyn Queue> = Arc::new(MemoryQueue::new(background.clone(), ConsumerSlot::default()));
    let clock: Arc<dyn Clock> = Arc::new(FixedClock(DateTime::from_timestamp(1_760_000_000, 0).unwrap()));
    let cfg = CoreConfig { outbox_fastpath: false, ..CoreConfig::default() };
    App::new(
        Ports {
            db: Arc::new(MemoryDatabase::new()),
            queue,
            blobs: Arc::new(MemoryBlobStore::new()),
            limiter: Arc::new(MemoryRateLimiter::new(clock.clone())),
            clock,
            ids: Arc::new(CounterIds::default()),
            background,
            rng: Arc::new(CounterRng::default()),
            hasher: Arc::new(CountingHasher::new()),
            oidc: None,
        },
        cfg,
        SystemScope::mint(),
    )
}

#[test]
fn any_workspace_exists_is_false_until_one_is_made() {
    let app = empty_app();
    assert!(!block_on(any_workspace_exists(&app)).unwrap(), "an empty database has none");
    let who = signed_up(&app, "first@example.test");
    assert!(!block_on(any_workspace_exists(&app)).unwrap(), "an account is not a workspace");
    make(&app, &who, "Acme", "acme");
    assert!(block_on(any_workspace_exists(&app)).unwrap());
    // and the default world, which is seeded with two, has some from the start
    assert!(block_on(any_workspace_exists(&world().app)).unwrap());
}
