//! Sign-in with an emailed code (F-A04, D-042): `request_login_code` and `sign_in_with_code`, against the memory adapters, a mail service the
//! test can take down, and a clock the test moves.
//!
//! What must hold, as a generated sequence of asks, right and wrong tries, time, a mail service that goes down and comes back, and accounts that
//! are deactivated and reactivated, checked against a model of the pin's rules after every step (`model_login_codes_follow_the_pins_rules`):
//! * a code is made, and mailed, only for an address an account has, and an address nobody has is answered as a mailed one and mailed nothing;
//! * an address is made at most four codes in a window of ten minutes that the first of them opens and the later ones do not lengthen, the fifth
//!   is refused, and a code that is used up gives the count back with it;
//! * a code works for ten minutes, to the second, and a new one replaces the old; it allows five tries, the fifth wrong one ends it, and the
//!   right one at the fifth try still counts; one that is used up, exhausted or out of time is `Expired`;
//! * a code that is right for an account that is deactivated is used up and refused;
//! * a mail service that refuses the message leaves no code that works and no count against the person.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "test code: a panic is how a test fails, and clippy.toml exempts only #[test] functions and #[cfg(test)] modules, not the helpers around them"
)]

mod common;

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use common::*;
use futures_executor::block_on;
use proptest::prelude::*;
use proptest::test_runner::{Config, TestRunner};
use rstest::rstest;
use thrust_adapters_memory::{ConsumerSlot, MemoryEmail, MemoryQueue};
use thrust_contracts::chrono::DateTime;
use thrust_contracts::*;
use thrust_core::*;
use thrust_ports::*;

const BASE: &str = "https://projects.example.test";
const PASSWORD: &str = "correct horse battery staple";
const TTL: i64 = 600;
const WINDOW: i64 = 600;
const MAX_ATTEMPTS: u32 = 5;
const MAX_ISSUED: u32 = 4;

/// A clock the test moves, in seconds from a fixed start.
struct Clockwork(AtomicI64);
impl Clock for Clockwork {
    fn now(&self) -> Timestamp {
        DateTime::from_timestamp(1_760_000_000 + self.0.load(Ordering::SeqCst), 0).unwrap()
    }
}

/// A mail service that keeps what it accepts, and that the test can take down: a down service refuses every message.
struct Flaky {
    up: AtomicBool,
    inner: MemoryEmail,
}
#[async_trait]
impl EmailSender for Flaky {
    async fn send(&self, message: &EmailMessage) -> Result<String, EmailError> {
        if self.up.load(Ordering::SeqCst) {
            self.inner.send(message).await
        } else {
            Err(EmailError::Unavailable("down".into()))
        }
    }
}

struct Setup {
    w: World,
    mail: Arc<Flaky>,
    clock: Arc<Clockwork>,
}

impl Setup {
    fn advance(&self, secs: i64) {
        self.clock.0.fetch_add(secs, Ordering::SeqCst);
    }
    fn now(&self) -> i64 {
        self.clock.0.load(Ordering::SeqCst)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Who {
    Target,
    Other,
    Unknown,
}

impl Who {
    fn email(self) -> &'static str {
        match self {
            Who::Target => "target@example.test",
            Who::Other => "other@example.test",
            Who::Unknown => "nobody@example.test",
        }
    }
    fn has_account(self) -> bool {
        self != Who::Unknown
    }
}

/// A world that can mail (or, by what is left out, cannot), with two accounts that hold `PASSWORD`.
fn setup(sender: bool, base: bool, rate: u32) -> Setup {
    let mail = Arc::new(Flaky { up: AtomicBool::new(true), inner: MemoryEmail::new() });
    let clock = Arc::new(Clockwork(AtomicI64::new(0)));
    let cfg = CoreConfig {
        outbox_fastpath: false,
        sign_in_rate_limit: rate,
        public_base_url: base.then(|| BASE.to_string()),
        login_code_ttl_secs: TTL,
        login_code_window_secs: WINDOW,
        login_code_max_attempts: MAX_ATTEMPTS,
        login_code_max_issued: MAX_ISSUED,
        ..CoreConfig::default()
    };
    let background: Arc<dyn Background> = Arc::new(NoBackground);
    let queue: Arc<dyn Queue> = Arc::new(MemoryQueue::new(background.clone(), ConsumerSlot::default()));
    let sender: Option<Arc<dyn EmailSender>> = sender.then(|| mail.clone() as Arc<dyn EmailSender>);
    let w = world_mailing(cfg, queue, background, clock.clone(), sender);
    for who in [Who::Target, Who::Other] {
        block_on(sign_up(&w.app, who.email(), PASSWORD)).unwrap();
    }
    w.hasher.reset();
    Setup { w, mail, clock }
}

fn mailing() -> Setup {
    setup(true, true, 100_000)
}

fn user_id(s: &Setup, who: Who) -> UserId {
    block_on(s.w.app.ports.db.user_by_email(who.email())).unwrap().expect("the account").id
}

/// The code in the newest message mailed to `who`: the six digits that end its subject.
fn mailed_code(s: &Setup, who: Who) -> Option<String> {
    s.mail
        .inner
        .sent()
        .iter()
        .rev()
        .find(|m| m.to == who.email())
        .map(|m| m.subject.rsplit(' ').next().expect("a word").to_string())
}

fn mails_to(s: &Setup, who: Who) -> usize {
    s.mail.inner.sent().iter().filter(|m| m.to == who.email()).count()
}

/// What `request_login_code` came to, as a thing to compare.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Asked {
    NotConfigured,
    InvalidEmail,
    Capped,
    Sent,
    NothingSent,
    Mail,
}

fn asked(r: &Result<CodeRequested, CodeRequestError>) -> Asked {
    match r {
        Ok(CodeRequested::Sent) => Asked::Sent,
        Ok(CodeRequested::NothingSent) => Asked::NothingSent,
        Err(CodeRequestError::NotConfigured) => Asked::NotConfigured,
        Err(CodeRequestError::InvalidEmail) => Asked::InvalidEmail,
        Err(CodeRequestError::Capped) => Asked::Capped,
        Err(CodeRequestError::Mail(_)) => Asked::Mail,
        Err(other) => panic!("a fault that is not the request's: {other:?}"),
    }
}

fn ask(s: &Setup, email: &str) -> Asked {
    asked(&block_on(request_login_code(&s.w.app, email)))
}

/// What `sign_in_with_code` came to, as a thing to compare.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Tried {
    SignedIn,
    Invalid,
    Expired,
    Exhausted,
    Deactivated,
    UnknownUser,
    NotConfigured,
    RateLimited,
}

fn tried(r: &Result<SignedIn, CodeSignInError>) -> Tried {
    match r {
        Ok(_) => Tried::SignedIn,
        Err(CodeSignInError::Invalid) => Tried::Invalid,
        Err(CodeSignInError::Expired) => Tried::Expired,
        Err(CodeSignInError::Exhausted) => Tried::Exhausted,
        Err(CodeSignInError::Deactivated) => Tried::Deactivated,
        Err(CodeSignInError::UnknownUser) => Tried::UnknownUser,
        Err(CodeSignInError::NotConfigured) => Tried::NotConfigured,
        Err(CodeSignInError::RateLimited { .. }) => Tried::RateLimited,
        Err(other) => panic!("a fault that is not the sign-in's: {other:?}"),
    }
}

fn try_code(s: &Setup, email: &str, code: &str) -> Tried {
    tried(&block_on(sign_in_with_code(&s.w.app, email, code)))
}

// ---------------------------------------------------------------------------------------------------------------------------------
// The model
// ---------------------------------------------------------------------------------------------------------------------------------

#[derive(Debug, Clone)]
enum Op {
    Ask(Who),
    TryRight(Who),
    /// This many wrong tries in a row (a code allows five, so that its last one is reached by more than luck).
    TryWrong(Who, usize),
    Advance(i64),
    MailDown,
    MailUp,
    SetActive(Who, bool),
}

/// The seconds a step moves the clock by: nothing, the edges of a code's life and of a window, and anything in between and past them, so
/// that a boundary is hit by many sequences and not by one in a thousand (two random instants are almost never exactly a window apart).
fn advance() -> impl Strategy<Value = i64> {
    prop_oneof![
        3 => Just(0i64),
        2 => Just(1i64),
        2 => Just(TTL - 1),
        2 => Just(TTL),
        2 => Just(TTL + 1),
        2 => Just(WINDOW - 1),
        1 => Just(2 * TTL),
        3 => 0..(2 * TTL + 50),
    ]
}

fn who() -> impl Strategy<Value = Who> {
    prop_oneof![4 => Just(Who::Target), 2 => Just(Who::Other), 1 => Just(Who::Unknown)]
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        6 => who().prop_map(Op::Ask),
        5 => who().prop_map(Op::TryRight),
        5 => (who(), prop_oneof![3 => Just(1usize), 2 => 2usize..=6]).prop_map(|(w, n)| Op::TryWrong(w, n)),
        5 => advance().prop_map(Op::Advance),
        1 => Just(Op::MailDown),
        2 => Just(Op::MailUp),
        1 => (prop_oneof![Just(Who::Target), Just(Who::Other)], any::<bool>()).prop_map(|(w, a)| Op::SetActive(w, a)),
    ]
}

/// What the pin keeps for an address (its Redis keys `magic_<address>`, `:issued` and `:verify_attempts`), and what it does with it.
#[derive(Debug, Clone, Default)]
struct Row {
    /// The code that works now, and when its time is over.
    code: Option<(String, i64)>,
    attempts: u32,
    window_started: Option<i64>,
    issued: u32,
}

struct Model {
    rows: HashMap<Who, Row>,
    active: HashMap<Who, bool>,
    mail_up: bool,
}

impl Model {
    fn new() -> Self {
        Self {
            rows: HashMap::new(),
            active: [(Who::Target, true), (Who::Other, true), (Who::Unknown, true)].into_iter().collect(),
            mail_up: true,
        }
    }

    /// What asking for a code at `now` comes to, and the state it leaves. A code that is made is `Some(..)` and its digits are read from the
    /// mail by the caller, who puts them in the row.
    fn ask(&mut self, who: Who, now: i64) -> Asked {
        if !who.has_account() {
            return Asked::NothingSent;
        }
        let row = self.rows.entry(who).or_default();
        let live = row.window_started.is_some_and(|w| w + WINDOW > now);
        if live && row.issued >= MAX_ISSUED {
            return Asked::Capped;
        }
        if live {
            row.issued += 1;
        } else {
            row.window_started = Some(now);
            row.issued = 1;
        }
        row.attempts = 0;
        if self.mail_up {
            // the digits are not known here: a placeholder the caller replaces by what the mail says
            row.code = Some((String::new(), now + TTL));
            Asked::Sent
        } else {
            // the message was refused: the code is taken back, with its place in the count (the row goes when it was the only one)
            row.code = None;
            row.issued -= 1;
            if row.issued == 0 {
                self.rows.remove(&who);
            }
            Asked::Mail
        }
    }

    fn try_code(&mut self, who: Who, right: bool, now: i64) -> Tried {
        if !who.has_account() {
            return Tried::UnknownUser;
        }
        let row = self.rows.entry(who).or_default();
        if !row.code.as_ref().is_some_and(|(_, expires)| *expires > now) {
            return Tried::Expired;
        }
        row.attempts += 1;
        if right && row.attempts <= MAX_ATTEMPTS {
            self.rows.remove(&who);
            return if self.active[&who] { Tried::SignedIn } else { Tried::Deactivated };
        }
        if row.attempts >= MAX_ATTEMPTS {
            row.code = None;
            row.attempts = 0;
            return Tried::Exhausted;
        }
        Tried::Invalid
    }
}

/// A code that is not `code`: six digits that cannot be one that was made (a made code is never below 100000).
fn other_than(code: &str) -> String {
    if code == "000000" {
        "000001".into()
    } else {
        "000000".into()
    }
}

/// Run `ops` against the core and the model, step by step, and note each outcome class reached in `seen`.
fn run(ops: &[Op], seen: &Mutex<HashSet<String>>) -> Result<(), TestCaseError> {
    let s = mailing();
    let mut m = Model::new();
    let note = |what: String| {
        seen.lock().unwrap().insert(what);
    };
    for (i, op) in ops.iter().enumerate() {
        let at = format!("step {i} {op:?} at t={}", s.now());
        match op {
            Op::Advance(secs) => s.advance(*secs),
            Op::MailDown => {
                s.mail.up.store(false, Ordering::SeqCst);
                m.mail_up = false;
            }
            Op::MailUp => {
                s.mail.up.store(true, Ordering::SeqCst);
                m.mail_up = true;
            }
            Op::SetActive(who, active) => {
                prop_assert!(block_on(set_user_active(&s.w.app, user_id(&s, *who), *active)).unwrap(), "{at}");
                m.active.insert(*who, *active);
            }
            Op::Ask(who) => {
                let before = mails_to(&s, *who);
                let want = m.ask(*who, s.now());
                let got = ask(&s, who.email());
                prop_assert_eq!(got, want, "{} asking", at);
                // exactly one message to the address when a code was sent, and none otherwise, to anyone
                let sent = mails_to(&s, *who) - before;
                prop_assert_eq!(sent, usize::from(want == Asked::Sent), "{} messages sent", at);
                if want == Asked::Sent {
                    let code = mailed_code(&s, *who).expect("a code in the mail");
                    prop_assert!(
                        code.len() == 6 && code.chars().all(|c| c.is_ascii_digit()) && !code.starts_with('0'),
                        "{at}: {code}"
                    );
                    if let Some(row) = m.rows.get_mut(who) {
                        if let Some((c, _)) = row.code.as_mut() {
                            *c = code;
                        }
                    }
                }
                note(format!("ask {want:?}"));
            }
            Op::TryRight(who) | Op::TryWrong(who, _) => {
                let right = matches!(op, Op::TryRight(_));
                let times = if let Op::TryWrong(_, n) = op { *n } else { 1 };
                for _ in 0..times {
                    let current = m.rows.get(who).and_then(|r| r.code.as_ref()).map(|(c, _)| c.clone());
                    let given = match (&current, right) {
                        (Some(c), true) => c.clone(),
                        (Some(c), false) => other_than(c),
                        (None, _) => "000000".to_string(),
                    };
                    // a model that has no code to be right with is asked for a code that no one has: it answers as for a wrong one
                    let want = m.try_code(*who, right && current.is_some(), s.now());
                    let got = try_code(&s, who.email(), &given);
                    prop_assert_eq!(got, want, "{} trying {:?}", at, given);
                    note(format!("try {want:?}"));
                }
            }
        }
    }
    // a code is never a password: no hash work was done for any of it
    prop_assert_eq!(s.w.hasher.count(), 0);
    Ok(())
}

#[test]
fn model_login_codes_follow_the_pins_rules() {
    let seen = Mutex::new(HashSet::new());
    let mut runner = TestRunner::new(Config::default());
    runner.run(&proptest::collection::vec(op(), 1..60), |ops| run(&ops, &seen)).unwrap();
    // a run that never reached an outcome has not tested it: every answer a request and a try can come to was seen
    let seen = seen.into_inner().unwrap();
    for class in [
        "ask Sent",
        "ask NothingSent",
        "ask Capped",
        "ask Mail",
        "try SignedIn",
        "try Invalid",
        "try Expired",
        "try Exhausted",
        "try Deactivated",
        "try UnknownUser",
    ] {
        assert!(seen.contains(class), "no generated sequence reached `{class}`: {seen:?}");
    }
}

// ---------------------------------------------------------------------------------------------------------------------------------
// The digits
// ---------------------------------------------------------------------------------------------------------------------------------

/// The range the pin draws from (`secrets.randbelow(900000) + 100000`, `magic_code.py`): the first and last code, and the bits at which the
/// mapping stops (it throws away the tail of the 32-bit range, so that no code is likelier than another).
#[test]
fn the_codes_are_the_pins_range_and_the_tail_is_thrown_away() {
    assert_eq!(code_from_bits(0), Some(100_000));
    assert_eq!(code_from_bits(899_999), Some(999_999));
    assert_eq!(code_from_bits(900_000), Some(100_000));
    let limit = (u32::MAX / 900_000) * 900_000;
    assert_eq!(code_from_bits(limit - 1), Some(999_999));
    assert_eq!(code_from_bits(limit), None);
    assert_eq!(code_from_bits(u32::MAX), None);
}

proptest! {
    /// Any draw below the limit is six digits, and the mapping is periodic over the whole range it keeps, so each of the 900000 codes has
    /// exactly as many draws as any other (a bias would show as a code with one more).
    #[test]
    fn prop_a_code_is_six_digits_and_every_code_has_the_same_number_of_draws(bits in any::<u32>()) {
        let limit = (u32::MAX / 900_000) * 900_000;
        match code_from_bits(bits) {
            Some(c) => {
                prop_assert!(bits < limit);
                prop_assert!((100_000..=999_999).contains(&c));
                prop_assert_eq!(c.to_string().len(), 6);
                if bits + 900_000 < limit {
                    prop_assert_eq!(code_from_bits(bits + 900_000), Some(c));
                }
            }
            None => prop_assert!(bits >= limit),
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------------------------
// The order of answers, the mail, the limiter
// ---------------------------------------------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
enum Mailing {
    NoService,
    NoBaseUrl,
    Both,
}

/// The pin's order for a request, which the core keeps: an address that is not one, then a deployment that cannot mail, then an address
/// no account has (answered as a sent one, with nothing sent), then a code. Every combination of the three conditions.
#[rstest]
fn a_request_is_answered_in_the_pins_order(
    #[values(Mailing::NoService, Mailing::NoBaseUrl, Mailing::Both)] mailing: Mailing,
    #[values("nope", "", "target@example.test", "nobody@example.test")] email: &str,
) {
    let (sender, base) = match mailing {
        Mailing::NoService => (false, true),
        Mailing::NoBaseUrl => (true, false),
        Mailing::Both => (true, true),
    };
    let s = setup(sender, base, 100_000);
    let want = if !is_valid_email(&email.to_lowercase()) {
        Asked::InvalidEmail
    } else if !matches!(mailing, Mailing::Both) {
        Asked::NotConfigured
    } else if email.starts_with("nobody") {
        Asked::NothingSent
    } else {
        Asked::Sent
    };
    assert_eq!(ask(&s, email), want);
    // whatever it came to, only a sent code mailed anything
    assert_eq!(s.mail.inner.sent().len(), usize::from(want == Asked::Sent));
}

/// The pin's order for a try, which the core keeps (the form's emptiness is the route's): an account with the address (answered even by a
/// deployment that cannot mail, as the pin does), then a deployment that can mail, then the code.
#[rstest]
fn a_try_is_answered_in_the_pins_order(
    #[values(Mailing::NoService, Mailing::NoBaseUrl, Mailing::Both)] mailing: Mailing,
    #[values(Who::Target, Who::Unknown)] who: Who,
) {
    let (sender, base) = match mailing {
        Mailing::NoService => (false, true),
        Mailing::NoBaseUrl => (true, false),
        Mailing::Both => (true, true),
    };
    let s = setup(sender, base, 100_000);
    let want = match (who, mailing) {
        (Who::Unknown, _) => Tried::UnknownUser,
        (_, Mailing::Both) => Tried::Expired,
        _ => Tried::NotConfigured,
    };
    assert_eq!(try_code(&s, who.email(), "123456"), want);
}

/// What is mailed: one message to the address the person typed, in the spelling the account has, whose subject and text carry the same six
/// digits, that names Thrust, says how long the code works and has no link (a code is typed, never followed).
#[test]
fn the_mail_carries_the_code_and_nothing_to_follow() {
    let s = mailing();
    assert_eq!(ask(&s, "  TARGET@Example.Test "), Asked::Sent);
    let sent = s.mail.inner.sent();
    assert_eq!(sent.len(), 1);
    let m = &sent[0];
    assert_eq!(m.to, "target@example.test");
    let code = mailed_code(&s, Who::Target).unwrap();
    assert_eq!(m.subject, format!("Your Thrust sign-in code is {code}"));
    assert!(m.text.contains(&code) && m.text.contains("Thrust") && m.text.contains("10 minutes"), "{}", m.text);
    assert!(!m.text.contains("http"), "{}", m.text);
    assert!(m.html.is_none());
}

/// The tries are limited per address, like a password's: past the limit a right code is refused and used up by nothing, and the window's end
/// frees the address again. (The limit is `sign_in_rate_limit` in a window of `sign_in_rate_window_secs`.)
#[rstest]
fn the_tries_of_an_address_are_limited(#[values(1u32, 2, 3, 4)] limit: u32) {
    let s = setup(true, true, limit);
    assert_eq!(ask(&s, Who::Target.email()), Asked::Sent);
    let code = mailed_code(&s, Who::Target).unwrap();
    for _ in 0..limit {
        // each of these is a try that is counted by the limiter, whatever it comes to (an unknown address is a different bucket); fewer than the
        // code's five, so that it is still there afterwards
        assert_eq!(try_code(&s, Who::Target.email(), "000000"), Tried::Invalid);
    }
    // the right code, one try too many: refused for the limit, and the code is still good afterwards
    assert_eq!(try_code(&s, Who::Target.email(), &code), Tried::RateLimited);
    assert_eq!(try_code(&s, Who::Unknown.email(), "000000"), Tried::UnknownUser, "another address has its own budget");
    s.advance(i64::try_from(CoreConfig::default().sign_in_rate_window_secs).unwrap() + 1);
    assert_eq!(try_code(&s, Who::Target.email(), &code), Tried::SignedIn);
}

/// The session a right code makes is the account's own and a real one, and the code is not a way to sign in as anyone else.
#[test]
fn a_right_code_signs_in_its_own_account_only() {
    let s = mailing();
    assert_eq!(ask(&s, Who::Target.email()), Asked::Sent);
    assert_eq!(ask(&s, Who::Other.email()), Asked::Sent);
    let (target_code, other_code) = (mailed_code(&s, Who::Target).unwrap(), mailed_code(&s, Who::Other).unwrap());
    // the other account's code is no code of the target's
    assert_eq!(try_code(&s, Who::Target.email(), &other_code), Tried::Invalid);
    let signed = block_on(sign_in_with_code(&s.w.app, Who::Target.email(), &target_code)).expect("signed in");
    assert_eq!(signed.user_id, user_id(&s, Who::Target));
    let c = Credentials { bearer: Some(&signed.token), ..Credentials::default() };
    let who = block_on(authenticate_request(&s.w.app, &c)).expect("a live session");
    assert_eq!(who.principal.user_id, user_id(&s, Who::Target));
}

/// A limiter that holds the keys to the classes the Worker's rate-limiting bindings count (`signin:` and `v1:`, `thrust-adapters-cloudflare`'s
/// `KeyClass`) and refuses any other as that adapter does, which the memory limiter never would: the first version of the sign-in with a code
/// used a key of its own and was answered with a 500 by the `worker-cf` cell alone.
struct TwoClasses(Arc<dyn RateLimiter>);
#[async_trait]
impl RateLimiter for TwoClasses {
    async fn hit(&self, key: &str, limit: u32, window_secs: u64) -> Result<RateDecision, RateError> {
        match key.split_once(':').map(|(class, _)| class) {
            Some("signin" | "v1") => self.0.hit(key, limit, window_secs).await,
            _ => Err(RateError::Backend("a rate-limit key must start with `signin:` or `v1:`".into())),
        }
    }
}

/// Every try, of a known address and of an unknown one, is counted under a key that the Worker's limiter has a binding for, and under one that
/// is not the password's (an address that spends its tries at a code still has its tries at a password, and the reverse).
#[test]
fn the_tries_are_counted_under_a_key_the_workers_limiter_accepts_and_apart_from_the_passwords() {
    let s = setup(true, true, 2);
    let mut app = s.w.app.clone();
    app.ports.limiter = Arc::new(TwoClasses(app.ports.limiter.clone()));
    assert_eq!(ask(&s, Who::Target.email()), Asked::Sent);
    for who in [Who::Target, Who::Unknown] {
        let r = block_on(sign_in_with_code(&app, who.email(), "000000"));
        assert!(matches!(r, Err(CodeSignInError::Invalid | CodeSignInError::UnknownUser)), "{who:?}: {r:?}");
    }
    // two tries at a code, then the limit (2) is spent for the code, and the password's own count is untouched
    assert!(matches!(block_on(sign_in_with_code(&app, Who::Target.email(), "000000")), Err(CodeSignInError::Invalid)));
    assert!(matches!(
        block_on(sign_in_with_code(&app, Who::Target.email(), "000000")),
        Err(CodeSignInError::RateLimited { .. })
    ));
    assert!(block_on(sign_in(&app, Who::Target.email(), PASSWORD)).is_ok(), "the password's tries are its own");
}
