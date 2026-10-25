//! The caller's profile (issue #44): `contains_url`, the time zone list, and the use-cases that make, read and change a
//! profile, against the memory adapters.
//!
//! What must hold: `contains_url` says what the pinned Plane's own regex says, line by line (a differential test over
//! generated ASCII text, with the pattern as the oracle); an account has a profile from the moment it is made, and an older
//! one gets its profile on first read; a change sets exactly the fields it names and leaves the rest; two changes to
//! different fields both survive; a refused change writes nothing; signing in records when.

mod common;

use common::*;
use futures_executor::block_on;
use proptest::prelude::*;
use rstest::rstest;
use thrust_contracts::*;
use thrust_core::*;
use thrust_ports::*;

// --- contains_url, against the pin's regex -----------------------------------------------------------------------------

/// The four alternatives of `URL_PATTERN` (`apps/api/plane/utils/url.py` at the pin), each on its own, so a test can see
/// which one a string matches. Written in Rust regex syntax; nothing in them differs from Python's.
const ALTERNATIVES: [&str; 4] = [
    r"https?://[^\s]+",
    r"www\.[a-zA-Z0-9](?:[a-zA-Z0-9-]{0,61}[a-zA-Z0-9])?(?:\.[a-zA-Z0-9](?:[a-zA-Z0-9-]{0,61}[a-zA-Z0-9])?)*",
    r"(?:[a-zA-Z0-9](?:[a-zA-Z0-9-]{0,61}[a-zA-Z0-9])?\.)+[a-zA-Z]{2,6}",
    r"(?:(?:25[0-5]|2[0-4][0-9]|[01]?[0-9][0-9]?)\.){3}(?:25[0-5]|2[0-4][0-9]|[01]?[0-9][0-9]?)",
];

fn pattern() -> regex::Regex {
    regex::Regex::new(&format!("(?i)(?:{})", ALTERNATIVES.join("|"))).unwrap()
}

/// `contains_url` as the pin computes it: over 1000 characters is never a URL, each line is cut to 500, any line matching.
fn pin_contains_url(value: &str) -> bool {
    let re = pattern();
    if value.chars().count() > 1000 {
        return false;
    }
    value.split('\n').any(|line| re.is_match(&line.chars().take(500).collect::<String>()))
}

/// ASCII text built from the pieces the pattern cares about, joined at random, so that every alternative is matched by
/// some strings and just missed by others: scheme and `www` pieces, dots, hyphens, digit runs of every length, letters.
fn text() -> impl Strategy<Value = String> {
    let piece = prop_oneof![
        3 => "[a-zA-Z]{1,4}",
        3 => "[0-9]{1,4}",
        2 => Just(".".to_string()),
        // four dotted runs of up to three digits: an address when each is at most 255, a near miss when one is not
        2 => "[0-9]{1,3}\\.[0-9]{1,3}\\.[0-9]{1,3}\\.[0-9]{1,3}",
        // a scheme and a host with no dot in it, and `www.` with one character after it: each matched by one alternative
        1 => "https?://[a-z]{1,5}",
        1 => "[wW]{3}\\.[0-9a-z]",
        // a dot with no label before it and two letters after: not a domain
        1 => "[ -]\\.[a-z]{2,3}",
        1 => Just("-".to_string()),
        1 => Just("://".to_string()),
        1 => prop::sample::select(vec!["http", "https", "HTTP", "www", "WWW", "ftp", "com", "io", "256", "255", "999", "0"]).prop_map(str::to_string),
        1 => Just(" ".to_string()),
        1 => Just("\n".to_string()),
    ];
    prop::collection::vec(piece, 0..14).prop_map(|ps| ps.concat())
}

proptest! {
    #![proptest_config(proptest_config(1024))]

    #[test]
    fn prop_contains_url_says_what_the_pins_regex_says(s in text()) {
        prop_assert_eq!(contains_url(&s), pin_contains_url(&s), "{:?}", s);
    }

    /// Long lines and long values are cut as the pin cuts them: a URL after the 500th character of a line is not seen, and
    /// a value of more than 1000 characters is never one.
    #[test]
    fn prop_contains_url_cuts_lines_and_long_values_as_the_pin_does(pad in 0usize..1100, tail in prop::sample::select(vec!["http://x.test", "www.example", "a.bc", "1.2.3.4", "plain"])) {
        let s = format!("{}{}", "a".repeat(pad), tail);
        prop_assert_eq!(contains_url(&s), pin_contains_url(&s), "{} a's then {}", pad, tail);
    }
}

/// The two limits meet at the boundaries only on several lines (a line is cut at 500, a value is dropped over 1000), so they
/// are built that way: a URL at the start of the last line, and a total length from 995 to 1005 and well past it.
#[test]
fn contains_url_at_the_edges_of_its_two_limits() {
    let url = "http://x.test";
    // line 1: 499 letters and a newline (500 characters); line 2: the URL and filler up to the total
    let value = |total: usize| format!("{}\n{}{}", "a".repeat(499), url, "c".repeat(total - 500 - url.len()));
    for total in (995..=1005).chain([1500, 2000]) {
        let s = value(total);
        assert_eq!(s.chars().count(), total);
        let want = total <= 1000;
        assert_eq!(contains_url(&s), want, "{total} characters");
        assert_eq!(pin_contains_url(&s), want, "the pin at {total} characters");
    }
    // a URL starting at the 500th character of its line is seen only if some of it is: `http://` needs seven more
    for pad in [490, 492, 493, 494, 499, 500] {
        let s = format!("{}{url}", "a".repeat(pad));
        assert_eq!(contains_url(&s), pin_contains_url(&s), "{pad} characters then a URL");
    }
}

/// Rule 8 for the differential test: sampled at its own case count with a fixed seed, the generator must produce strings
/// that match each alternative, and strings that match none, or the test would agree on nothing.
#[test]
fn prop_contains_url_strategy_reaches_every_alternative_and_the_misses() {
    use proptest::strategy::ValueTree;
    use proptest::test_runner::TestRunner;
    let alternatives: Vec<regex::Regex> = ALTERNATIVES.iter().map(|a| regex::Regex::new(&format!("(?i)(?:{a})")).unwrap()).collect();
    let mut runner = TestRunner::deterministic();
    let mut only = [false; 4];
    let (mut any, mut none) = (false, false);
    for _ in 0..1024 {
        let s = text().new_tree(&mut runner).unwrap().current();
        let hits: Vec<bool> = alternatives.iter().map(|r| r.is_match(&s)).collect();
        for (i, only_i) in only.iter_mut().enumerate() {
            // matched by this alternative and by no other: the string a wrong version of this alternative would miss
            *only_i |= hits[i] && hits.iter().filter(|h| **h).count() == 1;
        }
        any |= hits.iter().any(|h| *h);
        none |= !hits.iter().any(|h| *h);
    }
    assert!(any && none, "[some match, some do not]: {:?}", [any, none]);
    assert!(only.iter().all(|o| *o), "alternatives never matched alone (scheme, www, dotted, ipv4): {only:?}");
}

/// Spec examples. The first two are the names the fixtures of #44 send; the rest are the pin's habits that surprise: it
/// refuses `Jane.Doe` (a dot between letters is a domain) and accepts `St. John`.
#[rstest]
#[case::the_fixtures_url_in_a_first_name("see example.com", true)]
#[case::the_fixtures_url_in_a_last_name("http://example.test/x", true)]
#[case::an_initial_and_a_dot_is_not_a_url("J.R. Smith", false)]
#[case::a_dot_between_letters_is_a_domain("Jane.Doe", true)]
#[case::a_dot_and_a_space("St. John", false)]
#[case::a_label_must_end_before_the_dot_hyphen("a-.bc", false)]
#[case::a_label_must_end_before_the_dot_space(" .ab", false)]
#[case::a_label_must_end_before_the_dot_start(".ab", false)]
#[case::a_label_of_one_character("x.ab", true)]
#[case::a_plain_name("Journey", false)]
#[case::an_apostrophe("O'Brien", false)]
#[case::an_ip_address("host 10.0.0.255 up", true)]
#[case::a_middle_octet_over_255_is_not_one("1.256.3.4", false)]
#[case::a_last_octet_is_only_a_prefix_so_256_still_matches("1.2.3.256", true)]
#[case::an_octet_of_four_digits_in_the_middle_is_not_one("1.2.3333.4", false)]
#[case::a_last_octet_of_four_digits_still_matches("1.2.3.4567", true)]
#[case::www_alone_is_not_one("www.", false)]
#[case::www_and_a_letter("www.x", true)]
#[case::a_scheme_and_nothing("http://", false)]
#[case::a_scheme_in_capitals("HTTPS://Example", true)]
#[case::non_ascii_never_matches("caf\u{e9}.\u{e9}t\u{e9}", false)]
fn contains_url_on_the_cases_that_matter(#[case] value: &str, #[case] url: bool) {
    assert_eq!(contains_url(value), url, "{value:?}");
    // and the pin says the same, for the ASCII ones
    if value.is_ascii() {
        assert_eq!(pin_contains_url(value), url, "the pin on {value:?}");
    }
}

#[test]
fn the_time_zones_the_fixtures_use() {
    for (tz, known) in [("UTC", true), ("Asia/Kolkata", true), ("America/New_York", true), ("utc", false), ("Mars/Olympus", false), ("", false), ("Asia/Kolkata ", false)] {
        assert_eq!(is_known_timezone(tz), known, "{tz:?}");
    }
}

// --- the use-cases -------------------------------------------------------------------------------------------------------

const PASSWORD: &str = "correct horse battery staple";

fn signed_up(w: &World, email: &str) -> (Principal, SignedIn) {
    let s = block_on(sign_up(&w.app, email, PASSWORD)).unwrap();
    (Principal { user_id: s.user_id }, s)
}

#[test]
fn an_account_has_a_profile_from_sign_up_with_the_defaults() {
    let w = world();
    let (who, s) = signed_up(&w, "New.Person@Example.test");
    let me = block_on(thrust_core::me(&w.app, &who)).unwrap();
    assert_eq!(me.user.email.as_deref(), Some("new.person@example.test"));
    assert_eq!(me.profile.user_id, s.user_id);
    assert_eq!(me.profile.display_name, "new.person", "the local part of the email");
    assert_eq!(me.profile.username.len(), 32);
    assert!(me.profile.username.chars().all(|c| c.is_ascii_hexdigit()));
    assert_ne!(me.profile.id, s.user_id.0, "the profile has its own id");
    assert_eq!((me.profile.first_name.as_str(), me.profile.last_name.as_str(), me.profile.timezone.as_str()), ("", "", "UTC"));
    assert!(!me.profile.is_onboarded && !me.profile.is_tour_completed);
    assert_eq!(me.profile.last_workspace_id, None);
    assert_eq!(me.profile.onboarding_step["profile_complete"], false);
    assert_eq!(me.profile.onboarding_step.as_object().unwrap().len(), 4);
    assert_eq!(me.profile.preferences, serde_json::json!({}));
    // signing up signs in: when is recorded
    assert_eq!(me.profile.last_login_at, Some(w.app.ports.clock.now()));
}

#[test]
fn an_account_older_than_profiles_gets_one_on_first_read_and_keeps_it() {
    let w = world();
    // a user a test seeded: a row in `users` and nothing else, so no profile
    let id = UserId(Uuid::from_u128(0xABCD));
    block_on(w.db.insert_user(&UserRecord { id, email: Some("old@example.test".into()), password_hash: None, is_active: true })).unwrap();
    assert!(block_on(w.db.profile(id)).unwrap().is_none());
    let who = Principal { user_id: id };
    let first = block_on(thrust_core::me(&w.app, &who)).unwrap();
    assert_eq!(first.profile.display_name, "old");
    let again = block_on(thrust_core::me(&w.app, &who)).unwrap();
    assert_eq!(again.profile.id, first.profile.id, "the second read finds the profile the first made");
}

#[test]
fn a_caller_whose_user_is_gone_is_not_signed_in() {
    let w = world();
    let who = Principal { user_id: UserId(Uuid::from_u128(0xDEAD)) };
    assert!(matches!(block_on(thrust_core::me(&w.app, &who)), Err(CoreError::Unauthenticated)));
    assert!(matches!(block_on(update_me(&w.app, &who, ProfilePatch::default())), Err(CoreError::Unauthenticated)));
}

#[test]
fn a_change_sets_the_fields_it_names_and_leaves_the_rest() {
    let w = world();
    let (who, _) = signed_up(&w, "p@example.test");
    let before = block_on(thrust_core::me(&w.app, &who)).unwrap().profile;
    let ws = WorkspaceId(Uuid::from_u128(77));
    let patch = ProfilePatch {
        first_name: Some("Pat".into()),
        last_name: Some("Q".into()),
        timezone: Some("Asia/Kolkata".into()),
        is_onboarded: Some(true),
        last_workspace_id: Some(Some(ws)),
        onboarding_step: Some(serde_json::json!({"profile_complete": true})),
        cover_image: Some(Some("https://example.test/c.png".into())),
        preferences: serde_json::json!({"role": "developer", "theme": {"theme": "dark"}}).as_object().unwrap().clone(),
        ..ProfilePatch::default()
    };
    let after = block_on(update_me(&w.app, &who, patch)).unwrap().profile;
    assert_eq!((after.first_name.as_str(), after.last_name.as_str(), after.timezone.as_str()), ("Pat", "Q", "Asia/Kolkata"));
    assert!(after.is_onboarded);
    assert_eq!(after.last_workspace_id, Some(ws));
    assert_eq!(after.onboarding_step, serde_json::json!({"profile_complete": true}), "replaced whole, not merged");
    assert_eq!(after.cover_image.as_deref(), Some("https://example.test/c.png"));
    assert_eq!(after.preferences, serde_json::json!({"role": "developer", "theme": {"theme": "dark"}}));
    // what the patch did not name is as it was
    assert_eq!((after.display_name.as_str(), after.avatar.as_str(), after.is_tour_completed), (before.display_name.as_str(), "", false));
    assert_eq!((after.id, after.username.as_str(), after.date_joined), (before.id, before.username.as_str(), before.date_joined));
    assert!(after.updated_at >= before.updated_at);
    // a later patch of ONE preference keeps the others (jsonb `||`), and a field can be cleared
    let later = ProfilePatch {
        cover_image: Some(None),
        last_workspace_id: Some(None),
        preferences: serde_json::json!({"role": "designer"}).as_object().unwrap().clone(),
        ..ProfilePatch::default()
    };
    let p = block_on(update_me(&w.app, &who, later)).unwrap().profile;
    assert_eq!(p.preferences, serde_json::json!({"role": "designer", "theme": {"theme": "dark"}}));
    assert_eq!((p.cover_image, p.last_workspace_id), (None, None));
    assert_eq!(p.first_name, "Pat", "unrelated fields survive");
}

#[test]
fn two_changes_to_different_fields_both_survive() {
    // the onboarding page sends several at once; each is applied as a patch, never as read-modify-write of the whole row
    let w = world();
    let (who, _) = signed_up(&w, "q@example.test");
    let names = ProfilePatch { first_name: Some("A".into()), ..ProfilePatch::default() };
    let consent = ProfilePatch { preferences: serde_json::json!({"has_marketing_email_consent": true}).as_object().unwrap().clone(), ..ProfilePatch::default() };
    block_on(update_me(&w.app, &who, names)).unwrap();
    block_on(update_me(&w.app, &who, consent)).unwrap();
    let p = block_on(thrust_core::me(&w.app, &who)).unwrap().profile;
    assert_eq!(p.first_name, "A");
    assert_eq!(p.preferences["has_marketing_email_consent"], true);
}

#[derive(Debug, Clone, Copy)]
enum Refused {
    UrlInFirstName,
    UrlInLastName,
    UnknownTimezone,
}

#[rstest]
fn a_refused_change_writes_nothing(#[values(Refused::UrlInFirstName, Refused::UrlInLastName, Refused::UnknownTimezone)] why: Refused) {
    let w = world();
    let (who, _) = signed_up(&w, "r@example.test");
    let before = block_on(thrust_core::me(&w.app, &who)).unwrap().profile;
    // a good field travels with the bad one: it must not be written either
    let mut patch = ProfilePatch { display_name: Some("Should Not Land".into()), ..ProfilePatch::default() };
    match why {
        Refused::UrlInFirstName => patch.first_name = Some("example.com".into()),
        Refused::UrlInLastName => patch.last_name = Some("https://example.test".into()),
        Refused::UnknownTimezone => patch.timezone = Some("Mars/Olympus".into()),
    }
    assert!(matches!(block_on(update_me(&w.app, &who, patch)), Err(CoreError::Invalid(_))), "{why:?}");
    assert_eq!(block_on(thrust_core::me(&w.app, &who)).unwrap().profile, before, "{why:?}");
    // control: the good field alone is accepted
    let good = ProfilePatch { display_name: Some("Landed".into()), ..ProfilePatch::default() };
    assert_eq!(block_on(update_me(&w.app, &who, good)).unwrap().profile.display_name, "Landed");
}

#[test]
fn signing_in_records_when_and_a_failed_sign_in_does_not() {
    let w = world();
    let (who, _) = signed_up(&w, "s@example.test");
    // move the clock's reading of "last login" to something else, then sign in again: the field is rewritten
    let stale = ProfilePatch { last_login_at: Some(w.app.ports.clock.now() - thrust_contracts::chrono::TimeDelta::days(30)), ..ProfilePatch::default() };
    block_on(w.db.update_profile(who.user_id, &stale, w.app.ports.clock.now())).unwrap();
    assert_ne!(block_on(thrust_core::me(&w.app, &who)).unwrap().profile.last_login_at, Some(w.app.ports.clock.now()));
    assert!(block_on(sign_in(&w.app, "s@example.test", "not the password")).is_err());
    assert_ne!(
        block_on(thrust_core::me(&w.app, &who)).unwrap().profile.last_login_at,
        Some(w.app.ports.clock.now()),
        "a refused sign-in does not count"
    );
    block_on(sign_in(&w.app, "s@example.test", PASSWORD)).unwrap();
    assert_eq!(block_on(thrust_core::me(&w.app, &who)).unwrap().profile.last_login_at, Some(w.app.ports.clock.now()));
}
