//! The sign-in codes in the memory adapter (migration 0018, F-A04, D-042): one row a user, holding the digest of the code that works, its
//! tries and the count of codes made in the window.
//!
//! Exhaustive over every sequence of up to six steps from a small alphabet (make a code, try the right one, try a wrong one, take a code
//! back, take back one that is not the current, let a second pass), against a reference written as the pin stores it: three Redis-like keys
//! (`magic_<address>`, `:issued` and `:verify_attempts`) with their own expiries (`magic_code.py`), not as one row. The two agree on every
//! answer of every step. The lifetimes are cut to the size of the sequences (a code lives 3 s, a window is 4 s, a code allows 2 tries and a
//! window 2 codes), so that every edge is reached inside six steps: a code that is out of time exactly at its expiry, a window that is over
//! exactly at its end, the last try the code allows, the second code of a window and the third. The Postgres adapter is held to the same
//! answers by the generated sequences in `crates/adapters/selfhosted/tests/model` (`CodeIssue`, `CodeCheck`, `CodeDiscard` and the four
//! sequences that reach the edges).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "test code: a panic is how a test fails, and clippy.toml exempts only #[test] functions and #[cfg(test)] modules, not the helpers around them"
)]

use futures_executor::block_on;
use thrust_adapters_memory::MemoryDatabase;
use thrust_contracts::chrono::DateTime;
use thrust_contracts::*;
use thrust_ports::*;

const USER: UserId = UserId(Uuid::from_u128(7));
const TTL: i64 = 3;
const WINDOW: i64 = 4;
const MAX_ISSUED: u32 = 2;
const MAX_ATTEMPTS: u32 = 2;
const STEPS: usize = 6;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    Issue,
    Right,
    Wrong,
    DiscardCurrent,
    DiscardOther,
    Tick,
}

const ALL: [Step; 6] = [Step::Issue, Step::Right, Step::Wrong, Step::DiscardCurrent, Step::DiscardOther, Step::Tick];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Answer {
    Issue(LoginCodeIssue),
    Check(LoginCodeCheck),
    Discard(bool),
    Nothing,
}

fn at(t: i64) -> Timestamp {
    DateTime::from_timestamp(1_760_000_000 + t, 0).unwrap()
}

fn digest(n: u8) -> TokenHash {
    TokenHash([n; 32])
}

/// What the pin keeps for an address, as its keys: each value with the second at which it expires.
#[derive(Default)]
struct Keys {
    /// `magic_<address>`: the code's digest and when it expires.
    token: Option<(u8, i64)>,
    /// `magic_<address>:issued`: the count of codes made, expiring when the window the first of them opened is over.
    issued: Option<(u32, i64)>,
    /// `magic_<address>:verify_attempts`: the tries at the current token (it lives and dies with the token).
    attempts: u32,
}

impl Keys {
    /// The count of codes made is dropped when its window is over (Redis drops the key). A token that is out of time is kept and not
    /// looked at, until the next code replaces it: it is the same to every answer that has a clock, and the row that holds it is the same as the
    /// one a take-back finds.
    fn expire(&mut self, now: i64) {
        if self.issued.is_some_and(|(_, e)| e <= now) {
            self.issued = None;
        }
    }

    fn issue(&mut self, n: u8, now: i64) -> LoginCodeIssue {
        self.expire(now);
        // the pin counts first (`INCR`, with the expiry set by the first) and refuses a count past the cap; this stores nothing when it
        // refuses, which no later answer can tell apart, because the window's end is the first code's
        let (count, expires) = self.issued.unwrap_or((0, now + WINDOW));
        if count >= MAX_ISSUED {
            return LoginCodeIssue::Capped;
        }
        self.issued = Some((count + 1, expires));
        self.token = Some((n, now + TTL));
        self.attempts = 0;
        LoginCodeIssue::Issued
    }

    fn check(&mut self, given: u8, now: i64) -> LoginCodeCheck {
        self.expire(now);
        let Some((token, _)) = self.token.filter(|(_, e)| *e > now) else {
            return LoginCodeCheck::Expired;
        };
        self.attempts += 1;
        if given == token && self.attempts <= MAX_ATTEMPTS {
            // the token and both counters go
            self.token = None;
            self.issued = None;
            self.attempts = 0;
            return LoginCodeCheck::Matched;
        }
        if self.attempts >= MAX_ATTEMPTS {
            // the token and its counter go; the count of codes made stays
            self.token = None;
            self.attempts = 0;
            return LoginCodeCheck::Exhausted;
        }
        LoginCodeCheck::Wrong
    }

    /// Thrust's own step, which the pin has not (its mail is queued): the code that was never delivered goes, and its place in the count.
    /// It looks at no clock: the port's call has none, and a code that is stored is the current one, out of time or not (a take-back follows
    /// the mail that failed at once, so the difference is never seen, but the answer must not depend on a time nobody passes).
    fn discard(&mut self, given: u8) -> bool {
        let Some((token, _)) = self.token else {
            return false;
        };
        if token != given {
            return false;
        }
        self.token = None;
        self.attempts = 0;
        self.issued = match self.issued {
            Some((count, e)) if count > 1 => Some((count - 1, e)),
            _ => None,
        };
        true
    }
}

/// The digest of the code made by the `made`-th issue, which is what the right tries and the take-backs of the current code name.
fn code_of(made: u8) -> u8 {
    made + 1
}

#[test]
fn model_login_codes_match_the_pins_keys_over_every_short_sequence() {
    let mut total = 0u64;
    let mut reached = std::collections::BTreeSet::new();
    // every sequence of up to STEPS steps: for each length, the numbers below ALL.len()^len read in base ALL.len()
    for len in 1..=STEPS {
        let count = ALL.len().pow(u32::try_from(len).unwrap());
        for number in 0..count {
            total += 1;
            let steps: Vec<Step> = (0..len)
                .map(|place| ALL[(number / ALL.len().pow(u32::try_from(place).unwrap())) % ALL.len()])
                .collect();
            let db = MemoryDatabase::new();
            let mut keys = Keys::default();
            let (mut t, mut made) = (0i64, 0u8);
            for (i, step) in steps.iter().enumerate() {
                t += 1;
                let (got, want) = match step {
                    Step::Tick => (Answer::Nothing, Answer::Nothing),
                    Step::Issue => {
                        made += 1;
                        let c = NewLoginCode {
                            user_id: USER,
                            code_hash: digest(code_of(made)),
                            created_at: at(t),
                            expires_at: at(t + TTL),
                        };
                        let got = block_on(db.issue_login_code(&c, WINDOW, MAX_ISSUED)).unwrap();
                        (Answer::Issue(got), Answer::Issue(keys.issue(code_of(made), t)))
                    }
                    Step::Right | Step::Wrong => {
                        let given = if *step == Step::Right { code_of(made) } else { 0 };
                        let got = block_on(db.check_login_code(USER, &digest(given), at(t), MAX_ATTEMPTS)).unwrap();
                        (Answer::Check(got), Answer::Check(keys.check(given, t)))
                    }
                    Step::DiscardCurrent | Step::DiscardOther => {
                        let given = if *step == Step::DiscardCurrent { code_of(made) } else { 0 };
                        let got = block_on(db.discard_login_code(USER, &digest(given))).unwrap();
                        (Answer::Discard(got), Answer::Discard(keys.discard(given)))
                    }
                };
                assert_eq!(got, want, "step {i} of {steps:?} at t={t}");
                reached.insert(format!("{got:?}"));
            }
        }
    }
    // every answer the port can give was given, so no answer went unchecked
    for answer in [
        "Issue(Issued)",
        "Issue(Capped)",
        "Check(Matched)",
        "Check(Wrong)",
        "Check(Exhausted)",
        "Check(Expired)",
        "Discard(true)",
        "Discard(false)",
    ] {
        assert!(reached.contains(answer), "no sequence reached {answer}: {reached:?}");
    }
    assert!(total > 50_000, "{total} sequences");
}
