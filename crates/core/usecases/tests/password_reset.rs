//! Email recovery (F-A03, D-041): `request_password_reset` and `reset_password`, against the memory adapters, a mail service that keeps
//! what it is given, a clock the test moves, and the real argon2id at a cheap cost behind a hasher that counts its work.
//!
//! What must hold:
//! * a request is answered in the pin's order (this deployment can mail; the address is one; an account has it), a deactivated account
//!   is mailed nothing and told what a mailed one is told, a mail service that refuses leaves nothing behind (so the next request is not
//!   held back by the cooldown of a mail that was never sent), and what is mailed is one message to the normalised address whose link
//!   starts with the configured base URL and carries a `uidb64` that names the account, a 256-bit token and the address;
//! * a link resets the password of the account it was made for and no other: it is checked first (it decodes, names an account, is
//!   known, has not expired, is that account's), then the password (missing, then too weak); a refusal changes nothing and costs no
//!   hash work and does NOT use the link up (a weak password must not burn it); a success swaps the password, ends every session of the
//!   account and no other account's, signs nobody in, and uses the link up;
//! * the cooldown is inclusive (a link made exactly `cooldown` s ago still blocks a new one), a new link replaces the old, and a link
//!   works until its `ttl` has passed, to the second.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "test code: a panic is how a test fails, and clippy.toml exempts only #[test] functions and #[cfg(test)] modules, not the helpers around them"
)]

mod common;

use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use common::*;
use futures_executor::block_on;
use proptest::prelude::*;
use rstest::rstest;
use thrust_adapters_memory::{ConsumerSlot, EmailMode, MemoryEmail, MemoryQueue};
use thrust_contracts::chrono::DateTime;
use thrust_contracts::*;
use thrust_core::*;
use thrust_ports::*;

const BASE: &str = "https://projects.example.test";
const PASSWORD: &str = "correct horse battery staple";
const NEW_PASSWORD: &str = "a different strong passphrase";
/// Refused by `password_is_strong` (a common password once its decoration is gone).
const WEAK: &str = "password123";
const TARGET: &str = "target@example.test";
const BYSTANDER: &str = "bystander@example.test";
const TTL: i64 = 3600;
const COOLDOWN: i64 = 60;

/// A clock the test moves, in seconds from a fixed start.
struct Clockwork(AtomicI64);
impl Clock for Clockwork {
    fn now(&self) -> Timestamp {
        DateTime::from_timestamp(1_760_000_000 + self.0.load(Ordering::SeqCst), 0).unwrap()
    }
}

struct Setup {
    w: World,
    mail: Arc<MemoryEmail>,
    clock: Arc<Clockwork>,
}

impl Setup {
    fn advance(&self, secs: i64) {
        self.clock.0.fetch_add(secs, Ordering::SeqCst);
    }
}

/// A world that can mail (or, by what is left out, cannot), with two accounts that hold `PASSWORD`.
fn setup(mode: EmailMode, sender: bool, base: bool) -> Setup {
    let mail = Arc::new(MemoryEmail::with_mode(mode));
    let clock = Arc::new(Clockwork(AtomicI64::new(0)));
    let cfg = CoreConfig {
        outbox_fastpath: false,
        sign_in_rate_limit: 100_000,
        public_base_url: base.then(|| BASE.to_string()),
        password_reset_ttl_secs: TTL,
        password_reset_cooldown_secs: COOLDOWN,
        ..CoreConfig::default()
    };
    let background: Arc<dyn Background> = Arc::new(NoBackground);
    let queue: Arc<dyn Queue> = Arc::new(MemoryQueue::new(background.clone(), ConsumerSlot::default()));
    let sender: Option<Arc<dyn EmailSender>> = sender.then(|| mail.clone() as Arc<dyn EmailSender>);
    let w = world_mailing(cfg, queue, background, clock.clone(), sender);
    for email in [TARGET, BYSTANDER] {
        block_on(sign_up(&w.app, email, PASSWORD)).unwrap();
    }
    w.hasher.reset();
    Setup { w, mail, clock }
}

fn mailing() -> Setup {
    setup(EmailMode::Accept, true, true)
}

fn user_id(s: &Setup, email: &str) -> UserId {
    block_on(s.w.app.ports.db.user_by_email(email)).unwrap().expect("the account").id
}

fn bearer(token: &str) -> Credentials<'_> {
    Credentials { bearer: Some(token), ..Credentials::default() }
}

fn alive(s: &Setup, token: &str) -> bool {
    block_on(authenticate_request(&s.w.app, &bearer(token))).is_ok()
}

/// A fresh sign-in's session token for `email` and `password`, if the password is the account's.
fn signs_in(s: &Setup, email: &str, password: &str) -> Option<String> {
    block_on(sign_in(&s.w.app, email, password)).ok().map(|t| t.token)
}

/// What `request_password_reset` came to, as a thing to compare.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Asked {
    NotConfigured,
    InvalidEmail,
    UnknownUser,
    Sent,
    NothingSent,
    Mail,
}

fn asked(r: &Result<ResetRequested, ResetRequestError>) -> Asked {
    match r {
        Ok(ResetRequested::Sent) => Asked::Sent,
        Ok(ResetRequested::NothingSent) => Asked::NothingSent,
        Err(ResetRequestError::NotConfigured) => Asked::NotConfigured,
        Err(ResetRequestError::InvalidEmail) => Asked::InvalidEmail,
        Err(ResetRequestError::UnknownUser) => Asked::UnknownUser,
        Err(ResetRequestError::Mail(_)) => Asked::Mail,
        Err(other) => panic!("a fault that is not the request's: {other:?}"),
    }
}

fn request(s: &Setup, email: &str) -> Asked {
    asked(&block_on(request_password_reset(&s.w.app, email)))
}

/// The link of the last mail, taken apart: `(uidb64, token, email)` as the app's reset page reads them from its query.
fn last_link(s: &Setup) -> (String, String, String) {
    let sent = s.mail.sent();
    let text = &sent.last().expect("a mail was sent").text;
    let link = text.lines().find(|l| l.starts_with(BASE)).expect("the mail carries the link").to_string();
    let query =
        link.strip_prefix(&format!("{BASE}/accounts/reset-password/?")).expect("the link is the app's reset page");
    let get = |name: &str| {
        query
            .split('&')
            .find_map(|kv| kv.strip_prefix(&format!("{name}=")))
            .unwrap_or_else(|| panic!("no {name} in {query}"))
            .to_string()
    };
    (get("uidb64"), get("token"), get("email"))
}

/// What `reset_password` came to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Reset {
    Done,
    InvalidToken,
    PasswordMissing,
    TooWeak,
}

fn reset(s: &Setup, uidb64: &str, token: &str, password: &str) -> Reset {
    match block_on(reset_password(&s.w.app, uidb64, token, password)) {
        Ok(()) => Reset::Done,
        Err(PasswordResetError::InvalidToken) => Reset::InvalidToken,
        Err(PasswordResetError::PasswordMissing) => Reset::PasswordMissing,
        Err(PasswordResetError::PasswordTooWeak) => Reset::TooWeak,
        Err(other) => panic!("a fault that is not the reset's: {other:?}"),
    }
}

// --- the request -------------------------------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
enum Deployment {
    NoSender,
    NoBaseUrl,
    Mails,
    SenderRefuses,
    SenderUnreachable,
}

#[derive(Debug, Clone, Copy)]
enum Address {
    Known,
    KnownPaddedAndUpperCase,
    KnownButDeactivated,
    Unknown,
    Garbled,
    Empty,
}

/// Every deployment by every address: the answer is the table's, and what was mailed is what the answer says.
#[test]
fn every_request_comes_to_the_outcome_the_table_says() {
    use Address::*;
    use Deployment::*;
    let deployments = [NoSender, NoBaseUrl, Mails, SenderRefuses, SenderUnreachable];
    let addresses = [Known, KnownPaddedAndUpperCase, KnownButDeactivated, Unknown, Garbled, Empty];
    let mut cases = 0;
    let mut seen = std::collections::BTreeSet::new();
    for deployment in deployments {
        for address in addresses {
            let case = format!("{deployment:?} x {address:?}");
            let s = match deployment {
                NoSender => setup(EmailMode::Accept, false, true),
                NoBaseUrl => setup(EmailMode::Accept, true, false),
                Mails => setup(EmailMode::Accept, true, true),
                SenderRefuses => setup(EmailMode::Reject, true, true),
                SenderUnreachable => setup(EmailMode::Unavailable, true, true),
            };
            if matches!(address, KnownButDeactivated) {
                assert!(block_on(set_user_active(&s.w.app, user_id(&s, TARGET), false)).unwrap());
            }
            let email = match address {
                Known | KnownButDeactivated => TARGET.to_string(),
                KnownPaddedAndUpperCase => format!("  {}  ", TARGET.to_uppercase()),
                Unknown => "nobody@example.test".to_string(),
                Garbled => "not an address".to_string(),
                Empty => String::new(),
            };
            let want = match (deployment, address) {
                (NoSender | NoBaseUrl, _) => Asked::NotConfigured,
                (_, Garbled | Empty) => Asked::InvalidEmail,
                (_, Unknown) => Asked::UnknownUser,
                (_, KnownButDeactivated) => Asked::NothingSent,
                (Mails, Known | KnownPaddedAndUpperCase) => Asked::Sent,
                (SenderRefuses | SenderUnreachable, Known | KnownPaddedAndUpperCase) => Asked::Mail,
            };
            let got = request(&s, &email);
            assert_eq!(got, want, "{case}");
            // what was mailed: one message to the normalised address when the answer is `Sent`, none otherwise
            let sent = s.mail.sent();
            if want == Asked::Sent {
                assert_eq!(sent.len(), 1, "{case}");
                assert_eq!(sent[0].to, TARGET, "{case}: the normalised address");
                assert!(!sent[0].subject.is_empty() && sent[0].html.is_none(), "{case}");
                let (uidb64_in_link, token, email_in_link) = last_link(&s);
                assert_eq!(uidb64_in_link, uidb64(user_id(&s, TARGET)), "{case}: the link names the account");
                assert_eq!(token.len(), 64, "{case}: 256 bits");
                assert!(token.chars().all(|c| c.is_ascii_hexdigit()), "{case}");
                assert_eq!(email_in_link, "target%40example.test", "{case}: the address, percent-encoded");
            } else {
                assert!(sent.is_empty(), "{case}: nothing is mailed");
            }
            // a refusal by the mail service leaves nothing behind: asking again is not held back by the cooldown of a mail that never went
            if want == Asked::Mail {
                assert_eq!(
                    request(&s, &email),
                    Asked::Mail,
                    "{case}: the next request is not blocked by a link that was never sent"
                );
            }
            seen.insert(format!("{want:?}"));
            cases += 1;
        }
    }
    // control: a table that reached nothing would pass; this one reached every kind of answer
    assert_eq!(cases, 30);
    assert_eq!(seen.len(), 6, "{seen:?}");
}

// --- the reset ---------------------------------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
enum Link {
    Good,
    GarbageUid,
    UidOfNobody,
    NonUtf8Uid,
    WrongToken,
    OtherAccountsUid,
    Expired,
}

#[derive(Debug, Clone, Copy)]
enum Pw {
    Strong,
    Empty,
    Weak,
}

/// Every link by every password: 21 worlds against the table. A bad link is refused whatever the password is (the link is checked
/// first), a good link meets the password's checks in order, and what a refusal leaves is exactly what it found.
#[test]
fn every_reset_comes_to_the_outcome_the_table_says() {
    let links = [
        Link::Good,
        Link::GarbageUid,
        Link::UidOfNobody,
        Link::NonUtf8Uid,
        Link::WrongToken,
        Link::OtherAccountsUid,
        Link::Expired,
    ];
    let passwords = [Pw::Strong, Pw::Empty, Pw::Weak];
    let (mut cases, mut done, mut refused_links, mut refused_passwords) = (0, 0, 0, 0);
    for link in links {
        for pw in passwords {
            let case = format!("{link:?} x {pw:?}");
            let s = mailing();
            let target_session = block_on(sign_in(&s.w.app, TARGET, PASSWORD)).unwrap().token;
            let bystander_session = block_on(sign_in(&s.w.app, BYSTANDER, PASSWORD)).unwrap().token;
            assert_eq!(request(&s, TARGET), Asked::Sent, "{case}");
            let (uid, token, _) = last_link(&s);
            let (u, t) = match link {
                Link::Good => (uid.clone(), token.clone()),
                Link::GarbageUid => ("!!!".to_string(), token.clone()),
                Link::UidOfNobody => (uidb64(UserId(Uuid::from_u128(0xDEAD))), token.clone()),
                Link::NonUtf8Uid => (URL_SAFE_NO_PAD.encode([0xff, 0xfe]), token.clone()),
                Link::WrongToken => (uid.clone(), "0".repeat(64)),
                Link::OtherAccountsUid => (uidb64(user_id(&s, BYSTANDER)), token.clone()),
                Link::Expired => {
                    s.advance(TTL);
                    (uid.clone(), token.clone())
                }
            };
            let password = match pw {
                Pw::Strong => NEW_PASSWORD,
                Pw::Empty => "",
                Pw::Weak => WEAK,
            };
            let want = match (link, pw) {
                (Link::Good, Pw::Strong) => Reset::Done,
                (Link::Good, Pw::Empty) => Reset::PasswordMissing,
                (Link::Good, Pw::Weak) => Reset::TooWeak,
                _ => Reset::InvalidToken,
            };
            s.w.hasher.reset();
            assert_eq!(reset(&s, &u, &t, password), want, "{case}");
            let hashed = s.w.hasher.count();
            match want {
                Reset::Done => {
                    done += 1;
                    assert_eq!(hashed, 1, "{case}: one hash, of the new password");
                    assert!(signs_in(&s, TARGET, NEW_PASSWORD).is_some(), "{case}: the new password signs in");
                    assert!(signs_in(&s, TARGET, PASSWORD).is_none(), "{case}: the old one does not");
                    assert!(!alive(&s, &target_session), "{case}: every session of the account ended");
                    assert!(alive(&s, &bystander_session), "{case}: no other account's did");
                    assert!(signs_in(&s, BYSTANDER, PASSWORD).is_some(), "{case}: no other account's password changed");
                    assert_eq!(
                        reset(&s, &u, &t, "another strong passphrase 2"),
                        Reset::InvalidToken,
                        "{case}: the link is used up"
                    );
                }
                refusal => {
                    if refusal == Reset::InvalidToken {
                        refused_links += 1;
                    } else {
                        refused_passwords += 1;
                    }
                    assert_eq!(hashed, 0, "{case}: a refusal costs no hash work");
                    assert!(signs_in(&s, TARGET, PASSWORD).is_some(), "{case}: the password is as it was");
                    assert!(signs_in(&s, TARGET, NEW_PASSWORD).is_none(), "{case}");
                    assert!(alive(&s, &target_session) && alive(&s, &bystander_session), "{case}: no session ended");
                    // a refusal does not use the link up: the one that was mailed still works (unless it is the expired one)
                    if !matches!(link, Link::Expired) {
                        assert_eq!(
                            reset(&s, &uid, &token, NEW_PASSWORD),
                            Reset::Done,
                            "{case}: the mailed link still works"
                        );
                    }
                }
            }
            cases += 1;
        }
    }
    // control: each kind of outcome was reached, in the numbers the table gives
    assert_eq!(cases, 21);
    assert_eq!((done, refused_passwords, refused_links), (1, 2, 18));
}

/// A reset signs nobody in: no session exists that was not there before.
#[test]
fn a_reset_signs_nobody_in() {
    let s = mailing();
    assert_eq!(request(&s, TARGET), Asked::Sent);
    let (uid, token, _) = last_link(&s);
    let before =
        block_on(s.w.app.ports.db.revoke_user_sessions(user_id(&s, TARGET), s.w.app.ports.clock.now())).unwrap();
    assert_eq!(before, 1, "the sign-up's session is the only one");
    assert_eq!(reset(&s, &uid, &token, NEW_PASSWORD), Reset::Done);
    let after =
        block_on(s.w.app.ports.db.revoke_user_sessions(user_id(&s, TARGET), s.w.app.ports.clock.now())).unwrap();
    assert_eq!(after, 0, "the reset made no session");
}

// --- time --------------------------------------------------------------------------------------------------------------------------

/// The cooldown is inclusive, belongs to the account, and a link that is let through replaces the old one.
#[rstest]
fn the_cooldown_is_inclusive_and_a_new_link_replaces_the_old(#[values(0, 1, 59, 60, 61, 3600)] later: i64) {
    let s = mailing();
    assert_eq!(request(&s, TARGET), Asked::Sent);
    let (uid1, token1, _) = last_link(&s);
    s.advance(later);
    // another account is never held back by the target's link
    assert_eq!(request(&s, BYSTANDER), Asked::Sent, "{later} s");
    let blocked = later <= COOLDOWN;
    let second = request(&s, TARGET);
    if blocked {
        assert_eq!(second, Asked::NothingSent, "{later} s: still inside the cooldown");
        assert_eq!(s.mail.sent().len(), 2, "{later} s: nothing was mailed to the target again");
        assert_eq!(reset(&s, &uid1, &token1, NEW_PASSWORD), Reset::Done, "{later} s: the first link still works");
    } else {
        assert_eq!(second, Asked::Sent, "{later} s");
        let (uid2, token2, _) = last_link(&s);
        assert_ne!(token1, token2, "{later} s: a new token");
        assert_eq!(
            reset(&s, &uid1, &token1, NEW_PASSWORD),
            Reset::InvalidToken,
            "{later} s: the old link was replaced"
        );
        assert_eq!(reset(&s, &uid2, &token2, NEW_PASSWORD), Reset::Done, "{later} s: the new link works");
    }
}

/// A link works until `ttl` has passed: one second before it does, at it it does not.
#[rstest]
fn a_link_works_until_its_ttl_has_passed(#[values(TTL - 1, TTL, TTL + 1)] elapsed: i64) {
    let s = mailing();
    assert_eq!(request(&s, TARGET), Asked::Sent);
    let (uid, token, _) = last_link(&s);
    s.advance(elapsed);
    let want = if elapsed < TTL { Reset::Done } else { Reset::InvalidToken };
    assert_eq!(reset(&s, &uid, &token, NEW_PASSWORD), want, "{elapsed} s");
}

// --- the whole loop, generated -----------------------------------------------------------------------------------------------------

fn passphrase() -> impl Strategy<Value = String> {
    prop::collection::vec("[a-z]{4,9}", 4..6).prop_map(|w| w.join(" "))
}

proptest! {
    #![proptest_config(proptest_config(24))]

    /// For any address and any strong passphrase: ask, read the mail, follow the link, and the account has the new password and not
    /// the old, whatever the address looked like (case and padding) when it was asked for.
    #[test]
    fn prop_a_mailed_link_sets_any_strong_password(
        local in "[a-z][a-z0-9]{2,10}",
        old in passphrase(),
        new in passphrase(),
        padded in any::<bool>(),
        upper in any::<bool>(),
    ) {
        prop_assume!(old != new && password_is_strong(&old) && password_is_strong(&new));
        let s = mailing();
        let email = format!("{local}@example.test");
        block_on(sign_up(&s.w.app, &email, &old)).unwrap();
        let asked_as = match (padded, upper) {
            (false, false) => email.clone(),
            (true, false) => format!(" {email} "),
            (false, true) => email.to_uppercase(),
            (true, true) => format!(" {} ", email.to_uppercase()),
        };
        prop_assert_eq!(request(&s, &asked_as), Asked::Sent);
        let (uid, token, in_link) = last_link(&s);
        prop_assert_eq!(in_link.replace("%40", "@"), email.clone());
        prop_assert_eq!(reset(&s, &uid, &token, &new), Reset::Done);
        prop_assert!(signs_in(&s, &email, &new).is_some());
        prop_assert!(signs_in(&s, &email, &old).is_none());
    }
}
