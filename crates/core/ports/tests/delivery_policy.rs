//! The delivery policy every queue adapter applies (ADR-0070, FR-21, issue #53): what to do with a message after the consumer answered,
//! and how long to wait before delivering it again.
//!
//! The spec is written out here and not computed from the crate's constants, so that changing `MAX_DELIVERIES` without changing the
//! Worker's `max_retries = 3` and this table together fails a test: a message is delivered four times at most, the first delivery
//! and the three retries.

mod common;

use proptest::prelude::*;
use rstest::rstest;
use thrust_ports::{disposition, retry_delay, ConsumerError, Disposition, RETRY_BASE_DELAY, RETRY_MAX_DELAY};

#[derive(Debug, Clone, Copy)]
enum Outcome {
    Done,
    Permanent,
    Transient,
}

impl Outcome {
    fn answer(self) -> Result<(), ConsumerError> {
        match self {
            Outcome::Done => Ok(()),
            Outcome::Permanent => Err(ConsumerError::Permanent("the message is wrong".into())),
            Outcome::Transient => Err(ConsumerError::Transient("the database is down".into())),
        }
    }
}

/// The three things a queue adapter can do, without the delay.
fn kind(d: Disposition) -> &'static str {
    match d {
        Disposition::Ack => "ack",
        Disposition::Retry(_) => "retry",
        Disposition::DeadLetter => "dead-letter",
    }
}

/// The table: what each answer means for the Nth delivery of a message (`deliveries` counts this one).
fn spec(outcome: Outcome, deliveries: u32) -> &'static str {
    match (outcome, deliveries) {
        (Outcome::Done, _) => "ack",
        (Outcome::Permanent, _) => "dead-letter",
        (Outcome::Transient, 1..=3) => "retry",
        (Outcome::Transient, _) => "dead-letter",
    }
}

#[rstest]
fn every_answer_and_delivery_count_gets_the_disposition_of_the_table(
    #[values(Outcome::Done, Outcome::Permanent, Outcome::Transient)] outcome: Outcome,
    #[values(1, 2, 3, 4, 5, 6)] deliveries: u32,
) {
    let got = disposition(&outcome.answer(), deliveries);
    assert_eq!(kind(got), spec(outcome, deliveries), "{outcome:?} on delivery {deliveries}: {got:?}");
    // A retry carries the delay of this delivery, not some other one.
    if let Disposition::Retry(delay) = got {
        assert_eq!(delay, retry_delay(deliveries), "{outcome:?} on delivery {deliveries}");
    }
}

/// Delivery counts that matter, weighted: the ones a message really reaches, the edges of the range and any other.
fn deliveries() -> impl Strategy<Value = u32> {
    prop_oneof![
        4 => 1..=8u32,
        1 => Just(u32::MAX),
        1 => Just(u32::MAX - 1),
        2 => 1..=u32::MAX,
    ]
}

proptest! {
    #![proptest_config(common::proptest_config(256))]

    /// A message that is done, or that is wrong, is treated the same on the first delivery and on the millionth.
    #[test]
    fn prop_done_and_permanent_ignore_the_delivery_count(n in deliveries()) {
        prop_assert_eq!(disposition(&Ok(()), n), Disposition::Ack);
        prop_assert_eq!(disposition(&Outcome::Permanent.answer(), n), Disposition::DeadLetter);
    }

    /// A transient error is retried for the first three deliveries and dead-lettered from the fourth on, however large the count.
    #[test]
    fn prop_transient_is_retried_below_the_limit_and_dead_lettered_from_it(n in deliveries()) {
        let got = disposition(&Outcome::Transient.answer(), n);
        prop_assert_eq!(kind(got), if n <= 3 { "retry" } else { "dead-letter" }, "delivery {}", n);
    }

    /// The delay never leaves [base, cap], for any count at all, zero and the largest included.
    #[test]
    fn prop_retry_delay_stays_between_the_base_and_the_cap(n in prop_oneof![Just(0u32), deliveries()]) {
        let d = retry_delay(n);
        prop_assert!(d >= RETRY_BASE_DELAY, "{:?} is under the base", d);
        prop_assert!(d <= RETRY_MAX_DELAY, "{:?} is over the cap", d);
    }

    /// Each delivery waits twice as long as the one before, until it reaches the cap. That one relation also means the delay never
    /// goes down and that it reaches the cap, so there is no separate test for either.
    #[test]
    fn prop_retry_delay_doubles_until_it_reaches_the_cap(n in deliveries().prop_filter("n + 1 must fit", |n| *n < u32::MAX)) {
        let (now, next) = (retry_delay(n), retry_delay(n + 1));
        prop_assert_eq!(next, now.saturating_mul(2).min(RETRY_MAX_DELAY), "between delivery {} and {}", n, n + 1);
    }
}
