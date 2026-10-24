//! Sign-up (issue #44): the email and password rules, `email_exists`, and the use-case that makes an account and signs
//! it in, against the memory adapters with the real argon2id at a cheap cost and a hasher that counts its work.
//!
//! What must hold: an email is valid exactly when it has the usual ASCII shape (generated valid ones pass, and one stray
//! forbidden character anywhere makes any of them fail); a password is strong when it is long enough, varied enough and
//! not a common one in disguise; a sign-up that is refused changes nothing and costs no hash work; one that succeeds
//! makes exactly one user, hashes exactly once and returns a session that authenticates; the same email, however it is
//! spelled, is the same account.

mod common;

use common::*;
use futures_executor::block_on;
use proptest::prelude::*;
use rstest::rstest;
use thrust_core::*;

const PASSWORD: &str = "correct horse battery staple";

// --- emails ----------------------------------------------------------------------------------------------------------

const ATOM_CHARS: &str = "abcdefghijklmnopqrstuvwxyz0123456789!#$%&'*+/=?^_`{|}~-";

fn atom() -> impl Strategy<Value = String> {
    prop::collection::vec(prop::sample::select(ATOM_CHARS.chars().collect::<Vec<_>>()), 1..8)
        .prop_map(|cs| cs.into_iter().collect())
}

fn label() -> impl Strategy<Value = String> {
    // inner hyphens allowed, never at an end
    ("[a-z0-9]", prop::option::of("[a-z0-9-]{0,10}[a-z0-9]")).prop_map(|(a, rest)| format!("{a}{}", rest.unwrap_or_default()))
}

/// A valid, normalised email: dot-separated atoms, labels, and an alphabetic top-level domain.
fn valid_email() -> impl Strategy<Value = String> {
    (
        prop::collection::vec(atom(), 1..4),
        prop::collection::vec(label(), 1..3),
        "[a-z]{2,6}",
    )
        .prop_map(|(local, labels, tld)| format!("{}@{}.{}", local.join("."), labels.join("."), tld))
}

proptest! {
    #![proptest_config(proptest_config(256))]

    #[test]
    fn prop_a_generated_valid_email_is_accepted(email in valid_email()) {
        prop_assert!(is_valid_email(&email), "{email}");
    }

    /// Control for the property above: the same emails with one forbidden character put in anywhere are all refused, so
    /// the validator is not a function that says yes.
    #[test]
    fn prop_one_forbidden_character_anywhere_makes_an_email_invalid(
        email in valid_email(),
        at in 0usize..200,
        bad in prop::sample::select(vec![' ', '\t', '@', ',', '(', ')', '"', ';', '<', '>', '\\', '[', ']', '\u{e9}']),
    ) {
        let mut chars: Vec<char> = email.chars().collect();
        chars.insert(at % (chars.len() + 1), bad);
        let broken: String = chars.into_iter().collect();
        prop_assert!(!is_valid_email(&broken), "{broken:?}");
    }
}

/// Rule 8 for the email properties: sampled at the property's own case count (256) with a fixed seed, the generator must
/// reach every class that matters. A generator that never made a dotted local part, a multi-label domain or an atom
/// character beyond letters and digits would pass the properties on a validator that gets those wrong.
#[test]
fn prop_email_strategy_reaches_every_input_class() {
    use proptest::strategy::ValueTree;
    use proptest::test_runner::TestRunner;
    let mut runner = TestRunner::deterministic();
    let (mut dotted_local, mut multi_label, mut special_atom, mut inner_hyphen, mut digit_first, mut long_local) =
        (false, false, false, false, false, false);
    for _ in 0..256 {
        let email = valid_email().new_tree(&mut runner).unwrap().current();
        let (local, domain) = email.rsplit_once('@').unwrap();
        dotted_local |= local.contains('.');
        multi_label |= domain.matches('.').count() >= 2;
        special_atom |= local.chars().any(|c| "!#$%&'*+/=?^_`{|}~".contains(c));
        inner_hyphen |= domain.split('.').any(|l| l.len() > 2 && l[1..l.len() - 1].contains('-'));
        digit_first |= domain.starts_with(|c: char| c.is_ascii_digit());
        long_local |= local.len() >= 12;
    }
    let reached = [dotted_local, multi_label, special_atom, inner_hyphen, digit_first, long_local];
    assert!(reached.iter().all(|r| *r), "never generated, in order [dotted, multi-label, special atom, inner hyphen, digit-first, long local]: {reached:?}");
}

/// The same for the broken-email property: every forbidden character is put into both halves of an address.
#[test]
fn prop_forbidden_character_insertion_reaches_both_halves_and_every_character() {
    use proptest::strategy::ValueTree;
    use proptest::test_runner::TestRunner;
    let bad_chars = [' ', '\t', '@', ',', '(', ')', '"', ';', '<', '>', '\\', '[', ']', '\u{e9}'];
    let strategy = (valid_email(), 0usize..200, prop::sample::select(bad_chars.to_vec()));
    let mut runner = TestRunner::deterministic();
    let (mut in_local, mut in_domain) = (false, false);
    let mut seen = std::collections::BTreeSet::new();
    for _ in 0..256 {
        let (email, at, bad) = strategy.new_tree(&mut runner).unwrap().current();
        let at_byte = at % (email.chars().count() + 1);
        let local_len = email.rsplit_once('@').unwrap().0.chars().count();
        if at_byte <= local_len {
            in_local = true;
        } else {
            in_domain = true;
        }
        seen.insert(bad);
    }
    assert!(in_local && in_domain, "insertion points: local {in_local}, domain {in_domain}");
    assert_eq!(seen.len(), bad_chars.len(), "characters never inserted: {:?}", bad_chars.iter().filter(|c| !seen.contains(c)).collect::<Vec<_>>());
}

/// Spec examples (Django's `validate_email` and RFC 5321 limits), each one boundary of the rule.
#[rstest]
#[case::empty("")]
#[case::no_at("plainaddress")]
#[case::no_local("@no-local.test")]
#[case::no_domain("a@")]
#[case::one_label("a@b")]
#[case::numeric_tld("a@b.1")]
#[case::short_tld("a@b.c")]
#[case::double_dot_in_local("a..b@x.test")]
#[case::leading_dot_in_local(".a@x.test")]
#[case::trailing_dot_in_local("a.@x.test")]
#[case::leading_hyphen_in_label("a@-x.test")]
#[case::trailing_hyphen_in_label("a@x-.test")]
#[case::empty_label("a@x..test")]
#[case::trailing_dot_in_domain("a@x.test.")]
#[case::space_in_local("a b@x.test")]
fn an_email_of_the_wrong_shape_is_invalid(#[case] email: &str) {
    assert!(!is_valid_email(email), "{email:?}");
}

#[test]
fn the_length_limits_of_an_email_are_exact() {
    let local = |n: usize| "a".repeat(n);
    assert!(is_valid_email(&format!("{}@x.test", local(64))), "64 is the longest local part");
    assert!(!is_valid_email(&format!("{}@x.test", local(65))), "65 is too long");
    let label = |n: usize| "b".repeat(n);
    assert!(is_valid_email(&format!("a@{}.test", label(63))), "63 is the longest label");
    assert!(!is_valid_email(&format!("a@{}.test", label(64))), "64 is too long");
    // 320 in all: the bound the sign-in path already uses. A 64-character local part and a 255-character domain make 320
    // exactly; five full labels make 326, and every label and the local part are within their own limits.
    let at_limit = format!("{}@{}.{}.{}.{}", local(64), label(63), label(63), label(63), label(63));
    assert!(at_limit.len() == 320 && is_valid_email(&at_limit), "{} bytes", at_limit.len());
    let over = format!("a@{}.{}.{}.{}.{}.test", label(63), label(63), label(63), label(63), label(63));
    assert!(over.len() > 320 && !is_valid_email(&over), "{} bytes", over.len());
}

#[test]
fn the_top_level_domain_may_be_punycode() {
    assert!(is_valid_email("a@example.xn--p1ai"));
    assert!(!is_valid_email("a@example.xn--"), "an empty punycode label is not one");
}

proptest! {
    #![proptest_config(proptest_config(256))]

    /// A top-level domain with a digit in it is no domain, however long: only letters, or `xn--`, make one. (Long ones
    /// too, so a rule that lets anything over a few characters through is caught.)
    #[test]
    fn prop_a_top_level_domain_with_a_digit_is_invalid(
        local in atom(),
        tld in "[a-z0-9]{2,12}".prop_filter("a digit, and not punycode", |t| t.chars().any(|c| c.is_ascii_digit()) && !t.starts_with("xn--")),
    ) {
        let email = format!("{local}@example.{tld}");
        prop_assert!(!is_valid_email(&email), "{email}");
    }
}

// --- passwords -------------------------------------------------------------------------------------------------------

// Any string of fewer than `MIN_PASSWORD_LEN` characters is weak, whatever it holds.
proptest! {
    #![proptest_config(proptest_config(256))]

    #[test]
    fn prop_a_short_password_is_weak(p in "\\PC{0,9}") {
        prop_assert!(!password_is_strong(&p), "{p:?}");
    }

    #[test]
    fn prop_a_long_password_of_few_distinct_characters_is_weak(
        p in prop::collection::vec(prop::sample::select(vec!['a', 'b', 'c', 'd']), 10..60)
            .prop_map(|cs| cs.into_iter().collect::<String>()),
    ) {
        prop_assert!(!password_is_strong(&p), "{p:?}");
    }

    /// Exactly five distinct characters is enough, however long: the bound is "at least 5", not "more than 5".
    #[test]
    fn prop_exactly_five_distinct_characters_are_enough(
        p in prop::collection::vec(prop::sample::select(vec!['a', 'b', 'c', 'd', 'e']), 10..40)
            .prop_map(|cs| cs.into_iter().collect::<String>())
            .prop_filter("exactly 5 distinct", |p| p.chars().collect::<std::collections::BTreeSet<_>>().len() == 5),
    ) {
        prop_assert!(password_is_strong(&p), "{p:?}");
    }

    /// Long, varied and not a common password: strong. (A random string equals none of the common ones.)
    #[test]
    fn prop_a_long_varied_password_is_strong(
        p in prop::collection::vec(prop::sample::select("ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnpqrstuvwxyz23456789!#-_".chars().collect::<Vec<_>>()), 12..40)
            .prop_map(|cs| cs.into_iter().collect::<String>())
            .prop_filter("at least 5 distinct characters", |p| p.chars().collect::<std::collections::BTreeSet<_>>().len() >= 5),
    ) {
        prop_assert!(password_is_strong(&p), "{p:?}");
    }
}

/// A common password stays weak under decoration (digits and symbols around the letters), however long the decoration
/// makes it: the spec examples Plane's zxcvbn also refuses.
#[rstest]
fn a_common_password_is_weak_whatever_is_around_it(
    #[values("password", "qwerty", "letmein", "iloveyou", "admin", "welcome", "Passw0rd", "PASSWORD")] word: &str,
    #[values("", "1", "123456", "!!", "2024!")] suffix: &str,
    #[values("", "1", "#")] prefix: &str,
) {
    let p = format!("{prefix}{word}{suffix}");
    assert!(!password_is_strong(&p), "{p:?}");
}

/// Control for the matrix above: the same words inside a longer phrase are not "a common password in disguise".
#[rstest]
#[case("password-horse-battery-staple")]
#[case("my letmein is a long sentence")]
#[case("Qwerty-Zebra-Lantern-92")]
fn a_common_word_inside_a_longer_phrase_is_fine(#[case] p: &str) {
    assert!(password_is_strong(p), "{p:?}");
}

#[test]
fn the_password_length_bounds_are_exact() {
    let varied = |n: usize| "aB3$xY7!kQ".chars().cycle().take(n).collect::<String>();
    assert!(!password_is_strong(&varied(MIN_PASSWORD_LEN - 1)));
    assert!(password_is_strong(&varied(MIN_PASSWORD_LEN)));
    assert!(password_is_strong(&varied(1024)));
    assert!(!password_is_strong(&varied(1025)), "over the sign-in bound, so it could never sign in");
}

// --- the use-cases ---------------------------------------------------------------------------------------------------

fn fresh() -> World {
    let w = world();
    w.hasher.reset();
    w
}

fn bearer(token: &str) -> Credentials<'_> {
    Credentials { bearer: Some(token), ..Credentials::default() }
}

#[test]
fn sign_up_makes_one_user_who_is_signed_in_and_can_sign_in_again() {
    let w = fresh();
    let s = block_on(sign_up(&w.app, "  New.User@Example.TEST ", PASSWORD)).unwrap();
    assert_eq!(w.hasher.count(), 1, "one unit of hash work");
    // the session authenticates as the new user
    let who = block_on(authenticate_request(&w.app, &bearer(&s.token))).unwrap();
    assert_eq!(who.principal.user_id, s.user_id);
    // the same account, however the email is spelled
    for spelling in ["new.user@example.test", "NEW.USER@EXAMPLE.TEST", " new.user@example.test "] {
        let again = block_on(sign_in(&w.app, spelling, PASSWORD)).unwrap();
        assert_eq!(again.user_id, s.user_id, "{spelling}");
        assert!(block_on(email_exists(&w.app, spelling)).unwrap(), "{spelling}");
    }
    // control: the wrong password does not
    assert!(matches!(block_on(sign_in(&w.app, "new.user@example.test", "not the password")), Err(CoreError::Unauthenticated)));
}

#[derive(Debug, Clone, Copy)]
enum Refusal {
    BadEmail,
    WeakPassword,
    TakenEmail,
    TakenEmailOtherSpelling,
}

#[rstest]
fn a_refused_sign_up_changes_nothing_and_does_no_hash_work(
    #[values(Refusal::BadEmail, Refusal::WeakPassword, Refusal::TakenEmail, Refusal::TakenEmailOtherSpelling)] why: Refusal,
) {
    let w = fresh();
    let first = block_on(sign_up(&w.app, "taken@example.test", PASSWORD)).unwrap();
    w.hasher.reset();
    let (email, password, expect_conflict) = match why {
        Refusal::BadEmail => ("not-an-email", PASSWORD, false),
        Refusal::WeakPassword => ("weak@example.test", "password123", false),
        Refusal::TakenEmail => ("taken@example.test", "a different strong password", true),
        Refusal::TakenEmailOtherSpelling => (" TAKEN@example.test", "a different strong password", true),
    };
    let got = block_on(sign_up(&w.app, email, password));
    match (&got, expect_conflict) {
        (Err(CoreError::Conflict(_)), true) | (Err(CoreError::Invalid(_)), false) => {}
        _ => panic!("{why:?}: {got:?}"),
    }
    assert_eq!(w.hasher.count(), 0, "{why:?}: a refusal costs no hash work");
    // nothing was created: a refused new email is still unknown, and the existing account is untouched
    assert!(!block_on(email_exists(&w.app, "weak@example.test")).unwrap());
    let old = block_on(sign_in(&w.app, "taken@example.test", PASSWORD)).unwrap();
    assert_eq!(old.user_id, first.user_id, "the first password still opens the first account");
    assert!(matches!(
        block_on(sign_in(&w.app, "taken@example.test", "a different strong password")),
        Err(CoreError::Unauthenticated)
    ));
}

#[test]
fn two_sign_ups_get_different_users_and_different_sessions() {
    let w = fresh();
    let a = block_on(sign_up(&w.app, "a@example.test", PASSWORD)).unwrap();
    let b = block_on(sign_up(&w.app, "b@example.test", PASSWORD)).unwrap();
    assert_ne!(a.user_id, b.user_id);
    assert_ne!(a.token, b.token);
    assert_ne!(a.csrf_token, b.csrf_token);
    // a's token authenticates a, not b
    let who = block_on(authenticate_request(&w.app, &bearer(&a.token))).unwrap();
    assert_eq!(who.principal.user_id, a.user_id);
}

#[rstest]
fn email_exists_is_false_for_an_unknown_or_malformed_email(
    #[values("nobody@example.test", "", "not-an-email", "a@b")] email: &str,
) {
    let w = fresh();
    block_on(sign_up(&w.app, "somebody@example.test", PASSWORD)).unwrap();
    assert!(!block_on(email_exists(&w.app, email)).unwrap(), "{email:?}");
    // control: it is not constantly false
    assert!(block_on(email_exists(&w.app, "somebody@example.test")).unwrap());
}

#[test]
fn a_sign_up_with_a_failing_random_source_is_an_internal_error_and_stores_nothing() {
    // the salt comes from the Rng port: a source that cannot deliver must not produce a user with a predictable hash
    struct Dead;
    impl thrust_ports::Rng for Dead {
        fn fill(&self, _buf: &mut [u8]) -> Result<(), thrust_ports::RngError> {
            Err(thrust_ports::RngError("down (test)".into()))
        }
    }
    let w = fresh();
    let mut app = w.app.clone();
    app.ports.rng = std::sync::Arc::new(Dead);
    assert!(matches!(block_on(sign_up(&app, "x@example.test", PASSWORD)), Err(CoreError::Internal(_))));
    assert!(!block_on(email_exists(&w.app, "x@example.test")).unwrap());
}
