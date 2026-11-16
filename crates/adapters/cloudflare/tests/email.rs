//! The parts of the Cloudflare email adapter that are plain functions (E3 of the password reset by email, D-041): what the deployment's
//! variables mean ([`EmailConfig::from_lookup`]), which address is one address ([`is_plain_address`]), what a message becomes before the
//! binding is asked about it ([`outgoing`]) and what the binding's refusal means to the port ([`classify`]).
//!
//! The binding is a JS object, so `CfEmail::send` itself cannot run off wasm32: it runs in the `mail` pass of the contract suite on a Worker
//! target, where `wrangler dev`'s simulator of the binding is the mail server (`scripts/contract.sh worker-memory mail`). What is here is what
//! decides, before and after the binding, and each rule has a planted bug (`mutant_cf_email_*`, `scripts/mutants.sh`) that this file must fail on.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "test code: a panic is how a test fails, and clippy.toml exempts only #[test] functions and #[cfg(test)] modules, not the helpers around them"
)]

mod common;

use common::proptest_config;
use proptest::prelude::*;
use rstest::rstest;
use thrust_adapters_cloudflare::{classify, is_plain_address, outgoing, EmailConfig};
use thrust_ports::{EmailError, EmailMessage};

// ---------------------------------------------------------------------------------------------
// Strategies
// ---------------------------------------------------------------------------------------------

/// A plain address: dot-separated atoms, `@`, at least two labels.
fn address() -> impl Strategy<Value = String> {
    let atom = "[a-zA-Z0-9!#$%&'*+/=?^_`{|}~-]{1,12}";
    let label = "[a-z0-9]([a-z0-9-]{0,10}[a-z0-9])?";
    (prop::collection::vec(atom, 1..4), prop::collection::vec(label, 2..4))
        .prop_map(|(local, domain)| format!("{}@{}", local.join("."), domain.join(".")))
}

/// What must never be inside an address that is handed to the binding: whitespace, a line break, a NUL, the characters that start a name,
/// a list, a comment, a group, a quote, a second address, and what is not ASCII.
const HOSTILE: [char; 19] =
    [' ', '\t', '\r', '\n', '\0', '<', '>', ',', ';', ':', '"', '(', ')', '[', ']', '\\', '@', '\u{7f}', 'é'];

/// A plain address with one hostile character put somewhere in it (including at its ends and beside the `@`).
fn hostile_address() -> impl Strategy<Value = String> {
    (address(), any::<prop::sample::Index>(), 0..HOSTILE.len()).prop_map(|(a, at, h)| {
        let boundaries: Vec<usize> = a.char_indices().map(|(i, _)| i).chain(std::iter::once(a.len())).collect();
        let pos = boundaries[at.index(boundaries.len())];
        let mut s = a;
        s.insert(pos, HOSTILE[h]);
        s
    })
}

fn config() -> EmailConfig {
    EmailConfig { from: "no-reply@notify.example.com".into(), name: Some("Thrust".into()) }
}

fn message(to: &str, subject: &str) -> EmailMessage {
    EmailMessage { to: to.into(), subject: subject.into(), text: "body".into(), html: None }
}

// ---------------------------------------------------------------------------------------------
// The configuration
// ---------------------------------------------------------------------------------------------

/// The sender, each with whether it is one plain address. Pinned cases of what an operator might write: the right one, a name in the
/// field, a list, a header smuggled in, no domain, a space.
const FROMS: [(Option<&str>, bool); 11] = [
    (None, false),
    (Some(""), false),
    (Some("no-reply@notify.example.com"), true),
    (Some("a+b@c.example.org"), true),
    (Some("Thrust <no-reply@notify.example.com>"), false),
    (Some("a@b.example, c@d.example"), false),
    (Some("a@b.example\r\nBcc: c@d.example"), false),
    (Some("no-reply@localhost"), false),
    (Some("no-reply"), false),
    (Some("no reply@notify.example.com"), false),
    (Some("@notify.example.com"), false),
];

/// The name beside it, each with whether it is one a mail client can show.
const NAMES: [(Option<&str>, bool); 8] = [
    (None, true),
    (Some(""), true),
    (Some("Thrust"), true),
    (Some("Thrust Notifications (staging)"), true),
    (Some("Thr\"ust"), false),
    (Some("Thr\nust"), false),
    (Some("<Thrust>"), false),
    (Some("   "), false),
];

/// Every sender with every name: no sender and no name is no mail service; a name with no sender, an unusable sender, an unusable name
/// are errors that name the variable; and a usable pair is the configuration, an empty name being none.
#[rstest]
fn the_deployments_variables_are_read_as_they_are_written(
    #[values(0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10)] f: usize,
    #[values(0, 1, 2, 3, 4, 5, 6, 7)] n: usize,
) {
    let (from, from_ok) = FROMS[f];
    let (name, name_ok) = NAMES[n];
    let lookup = |k: &str| match k {
        "THRUST_EMAIL_FROM" => from.map(str::to_string),
        "THRUST_EMAIL_FROM_NAME" => name.map(str::to_string),
        other => panic!("the configuration read a variable it does not document: {other}"),
    };
    let got = EmailConfig::from_lookup(lookup);
    let sender_given = from.is_some_and(|v| !v.is_empty());
    let name_given = name.is_some_and(|v| !v.is_empty());
    let ctx = format!("from {from:?}, name {name:?}: {got:?}");
    match (sender_given, from_ok, name_given, name_ok) {
        (false, _, false, _) => assert_eq!(got, Ok(None), "{ctx}"),
        (false, _, true, _) => assert!(got.unwrap_err().0.contains("THRUST_EMAIL_FROM"), "{ctx}"),
        (true, false, _, _) => assert!(got.unwrap_err().0.contains("THRUST_EMAIL_FROM must"), "{ctx}"),
        (true, true, true, false) => assert!(got.unwrap_err().0.contains("THRUST_EMAIL_FROM_NAME"), "{ctx}"),
        (true, true, _, _) => {
            let want = EmailConfig {
                from: from.unwrap().to_string(),
                name: name.filter(|v| !v.is_empty()).map(str::to_string),
            };
            assert_eq!(got, Ok(Some(want)), "{ctx}");
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Which address is one address
// ---------------------------------------------------------------------------------------------

proptest! {
    #![proptest_config(proptest_config(512))]

    /// A plain address is accepted.
    #[test]
    fn prop_a_plain_address_is_one(a in address()) {
        prop_assert!(is_plain_address(&a), "{a}");
    }

    /// Anything with a hostile character in it is not: the character may be anywhere, the ends and the `@`'s sides included.
    #[test]
    fn prop_an_address_with_a_hostile_character_is_not_one(a in hostile_address()) {
        prop_assert!(!is_plain_address(&a), "{a:?}");
    }

    /// The independent reading: whatever the guard lets through, a parser that is not ours reads as exactly one mailbox, with no name and
    /// no group, whose address is the very string. (An address the guard accepted and another parser read as two would be the hole.)
    /// The strings are the plain addresses, the ones with one hostile character in them, and whatever text the strategy makes of its own.
    #[test]
    fn prop_what_the_guard_accepts_another_parser_reads_as_that_one_address(
        s in prop_oneof![address(), hostile_address(), any::<String>(), "[ -~]{0,40}"],
    ) {
        if is_plain_address(&s) {
            let parsed = mailparse::addrparse(&s).map_err(|e| TestCaseError::fail(format!("{s:?} did not parse: {e}")))?;
            prop_assert_eq!(parsed.len(), 1, "{:?}", s);
            match &parsed[0] {
                mailparse::MailAddr::Single(one) => {
                    prop_assert_eq!(one.display_name.as_deref(), None, "{:?}", s);
                    prop_assert_eq!(&one.addr, &s);
                }
                other => prop_assert!(false, "{s:?} parsed as {other:?}"),
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// What a message becomes
// ---------------------------------------------------------------------------------------------

proptest! {
    #![proptest_config(proptest_config(256))]

    /// A message to one plain address is handed over whole: the sender and its name are the configuration's, the recipient, the
    /// subject, the text and the html are the message's, none of them changed.
    #[test]
    fn prop_a_good_message_is_handed_over_as_it_is(
        to in address(),
        subject in "[ -~]{0,100}[!-~]",
        text in any::<String>(),
        html in prop::option::of(any::<String>()),
        named in any::<bool>(),
    ) {
        let cfg = EmailConfig { name: named.then(|| "Thrust".to_string()), ..config() };
        let m = EmailMessage { to: to.clone(), subject: subject.clone(), text: text.clone(), html: html.clone() };
        let out = outgoing(&cfg, &m).map_err(|e| TestCaseError::fail(format!("{e:?}")))?;
        prop_assert_eq!(out.from, cfg.from.clone(), "the sender of the mail is not the configured one");
        prop_assert_eq!(out.from_name, cfg.name.clone());
        prop_assert_eq!(out.to, to);
        prop_assert_eq!(out.subject, subject);
        prop_assert_eq!(out.text, text);
        prop_assert_eq!(out.html, html);
    }

    /// A recipient with a hostile character in it is refused, and nothing is handed over.
    #[test]
    fn prop_a_message_to_something_that_is_not_one_address_is_refused(to in hostile_address()) {
        match outgoing(&config(), &message(&to, "Reset your password")) {
            Err(EmailError::Rejected(_)) => {}
            other => prop_assert!(false, "a recipient that is not one address was not refused: {to:?}: {other:?}"),
        }
    }

    /// A subject with a control character in it (a line break above all, which would end the header and start another) is refused.
    #[test]
    fn prop_a_subject_with_a_line_break_or_another_control_character_is_refused(
        before in "[ -~]{0,20}",
        c in prop::sample::select(vec!['\r', '\n', '\0', '\t', '\u{1b}', '\u{7f}', '\u{85}']),
        after in "[ -~]{0,20}",
    ) {
        let subject = format!("a{before}{c}{after}");
        match outgoing(&config(), &message("someone@example.test", &subject)) {
            Err(EmailError::Rejected(_)) => {}
            other => prop_assert!(false, "{subject:?} was not refused: {other:?}"),
        }
    }
}

/// An empty subject, a subject of spaces and one longer than a header line may be are refused; one of exactly the longest length is not.
#[test]
fn a_subject_that_is_empty_or_longer_than_a_header_line_is_refused() {
    for subject in ["", "   ", &"x".repeat(999)] {
        assert!(
            matches!(outgoing(&config(), &message("someone@example.test", subject)), Err(EmailError::Rejected(_))),
            "{:?}",
            subject.len()
        );
    }
    let longest = "x".repeat(998);
    assert!(
        outgoing(&config(), &message("someone@example.test", &longest)).is_ok(),
        "998 characters is a header line that fits"
    );
}

// ---------------------------------------------------------------------------------------------
// What the binding's refusal means
// ---------------------------------------------------------------------------------------------

/// Cloudflare's table of the codes a rejected `send()` carries (the Workers API page, "Error codes", read 2026-11-15), each with whether it is a
/// refusal that would come again for the same message. The sender or the recipient is one the service will not use, or the message is
/// malformed or too large or has headers that are not allowed: refused. A limit, a paused account, an internal error and a delivery that
/// failed: an outage, which a later attempt may get past. This is the page's own list, not the adapter's constant.
const DOCUMENTED: [(&str, bool); 21] = [
    ("E_VALIDATION_ERROR", true),
    ("E_FIELD_MISSING", true),
    ("E_TOO_MANY_RECIPIENTS", true),
    ("E_TOO_MANY_ATTACHMENTS", true),
    ("E_SENDER_NOT_VERIFIED", true),
    ("E_RECIPIENT_NOT_ALLOWED", true),
    ("E_RECIPIENT_SUPPRESSED", true),
    ("E_ACCOUNT_SUSPENDED", false),
    ("E_SENDER_DOMAIN_NOT_AVAILABLE", true),
    ("E_CONTENT_TOO_LARGE", true),
    ("E_DELIVERY_FAILED", false),
    ("E_RATE_LIMIT_EXCEEDED", false),
    ("E_DAILY_LIMIT_EXCEEDED", false),
    ("E_INTERNAL_SERVER_ERROR", false),
    ("E_HEADER_NOT_ALLOWED", true),
    ("E_HEADER_USE_API_FIELD", true),
    ("E_HEADER_VALUE_INVALID", true),
    ("E_HEADER_VALUE_TOO_LONG", true),
    ("E_HEADER_NAME_INVALID", true),
    ("E_HEADERS_TOO_LARGE", true),
    ("E_HEADERS_TOO_MANY", true),
];

/// The words of the messages, with and without the two that mean "refused" when there is no code.
const MESSAGES: [&str; 5] = [
    "",
    "the service is busy",
    "email from example.com not allowed",
    "EMAIL TO x NOT ALLOWED",
    "not allowed because domain is not owned by the same account",
];

fn refused(e: &EmailError) -> bool {
    matches!(e, EmailError::Rejected(_))
}

fn text(e: &EmailError) -> &str {
    match e {
        EmailError::Rejected(t) | EmailError::Unavailable(t) => t,
    }
}

/// Every documented code, with every kind of message (the message does not decide when there is a code, even one that says "not allowed"):
/// the code decides, and the text keeps both.
#[test]
fn every_documented_code_is_a_refusal_or_an_outage_as_the_page_says() {
    assert_eq!(DOCUMENTED.len(), 21, "the page lists 21 codes");
    for (code, want_refusal) in DOCUMENTED {
        for message in MESSAGES {
            let got = classify(Some(code), message);
            assert_eq!(
                refused(&got),
                want_refusal,
                "documented codes were read the wrong way: {code} / {message:?}: {got:?}"
            );
            assert!(text(&got).contains(code), "the code is in the text: {got:?}");
            assert!(text(&got).contains(message), "the message is in the text: {got:?}");
        }
    }
}

/// With no code, as the local simulator and the deployed binding's refusal of an unowned sender say it (spike 0002, `research/0004` F9),
/// the words decide: "not allowed", in any case, is a refusal; anything else is an outage.
#[test]
fn with_no_code_the_words_not_allowed_are_a_refusal_and_nothing_else_is() {
    for message in MESSAGES {
        let got = classify(None, message);
        let want = message.to_ascii_lowercase().contains("not allowed");
        assert_eq!(refused(&got), want, "{message:?}: {got:?}");
        if !message.is_empty() {
            assert_eq!(text(&got), message, "no code: the message is the text");
        }
    }
    assert!(!refused(&classify(None, "")), "an error that says nothing is an outage, not a refusal");
}

proptest! {
    #![proptest_config(proptest_config(512))]

    /// A code the page does not list is an outage, whatever the message says, "not allowed" included: a message that is sent again later is
    /// the lesser harm than one dropped for a refusal that was only a code nobody has met.
    #[test]
    fn prop_a_code_the_page_does_not_list_is_an_outage(
        code in "E_[A-Z_]{1,24}".prop_filter("not a documented code", |c| !DOCUMENTED.iter().any(|(d, _)| d == c)),
        message in prop_oneof![Just(String::new()), "[ -~]{0,40}", Just("email from x not allowed".to_string())],
    ) {
        let got = classify(Some(&code), &message);
        prop_assert!(!refused(&got), "an unknown code was refused: {code} / {message:?}: {got:?}");
        prop_assert!(text(&got).contains(&code));
    }

    /// Whatever the code is, the message is never lost.
    #[test]
    fn prop_the_message_is_always_in_the_text(
        code in prop::option::of("[A-Z_]{0,20}"),
        message in "[ -~]{1,60}",
    ) {
        let got = classify(code.as_deref().filter(|c| !c.is_empty()), &message);
        prop_assert!(text(&got).contains(&message), "{got:?}");
    }
}
