//! `MemoryRateLimiter`, the limiter that counts exactly (issue #43). It is the native server's and the in-isolate Worker's
//! limiter, and the exact one: the Cloudflare adapter is the Workers Rate Limiting binding, which is approximate (D-007), so the
//! contract suite's `create_is_rate_limited_per_user` no longer asserts an exact limit and this is where the exact rule is held.
//!
//! The rule, written out: time is cut into windows of `window_secs` seconds, aligned to the clock (window `n` is the seconds from
//! `n * window_secs` up to but not including `(n + 1) * window_secs`). A key is allowed its first `limit` hits in a window, and
//! `remaining` says how many of the `limit` are left after this one. Every later hit in that window is refused, and told to wait
//! until the window ends (`retry_after_secs`, at least 1). The next window starts again from nothing. Keys do not share an
//! allowance.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "test code: a panic is how a test fails, and clippy.toml exempts only #[test] functions and #[cfg(test)] modules, not the helpers around them"
)]

mod common;

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;

use common::proptest_config;
use futures_executor::block_on;
use proptest::prelude::*;
use thrust_adapters_memory::MemoryRateLimiter;
use thrust_contracts::chrono::DateTime;
use thrust_contracts::*;
use thrust_ports::*;

/// A clock the test sets, in whole seconds since the epoch.
struct SetClock(AtomicI64);

impl Clock for SetClock {
    fn now(&self) -> Timestamp {
        DateTime::from_timestamp(self.0.load(Ordering::Relaxed), 0).unwrap()
    }
}

proptest! {
    #![proptest_config(proptest_config(256))]

    /// Whatever the limit, the window, where the clock starts and which of three keys is hit when: each answer is the one the rule
    /// above gives for that key's count in that window, computed here from the window's index and not from the limiter's own
    /// arithmetic. The windows are 10 s and 60 s, the two the Cloudflare binding has, and any length up to two minutes. Steps of
    /// 0 to 45 s cross a window boundary often, and several hits at one instant are common. A limit of 0 refuses everything.
    #[test]
    fn prop_every_key_gets_exactly_its_limit_in_every_window(
        limit in 0..=40u32,
        window in prop_oneof![Just(10u64), Just(60u64), 1..=120u64],
        start in 0..2_000_000_000i64,
        steps in prop::collection::vec((0..=45i64, 0..3usize), 0..150),
    ) {
        let clock = Arc::new(SetClock(AtomicI64::new(start)));
        let limiter = MemoryRateLimiter::new(clock.clone());
        let width = window as i64;
        // hits so far, by key and window index
        let mut counted: BTreeMap<(usize, i64), u32> = BTreeMap::new();
        let mut now = start;
        for (advance, key) in steps {
            now += advance;
            clock.0.store(now, Ordering::Relaxed);
            let index = now.div_euclid(width);
            let count = counted.entry((key, index)).or_insert(0);
            *count += 1;

            let want = if *count <= limit {
                RateDecision::Allowed { remaining: limit - *count }
            } else {
                RateDecision::Limited { retry_after_secs: ((index + 1) * width - now) as u64 }
            };
            let got = block_on(limiter.hit(&format!("k{key}"), limit, window)).unwrap();
            prop_assert_eq!(got, want, "key {} at t={} (window {}, hit {} of limit {})", key, now, index, count, limit);
        }
    }

    /// The rule as the contract test used to state it, for the key class it was about: after exactly `limit` hits at one instant
    /// the next is refused, and one hit of a key never touches another's allowance. Over any limit, and any instant in any window.
    #[test]
    fn prop_the_hit_after_the_limit_is_refused_and_another_key_is_not(
        limit in 1..=60u32,
        window in prop_oneof![Just(10u64), Just(60u64), 1..=120u64],
        start in 0..2_000_000_000i64,
    ) {
        let clock = Arc::new(SetClock(AtomicI64::new(start)));
        let limiter = MemoryRateLimiter::new(clock);
        for i in 1..=limit {
            let got = block_on(limiter.hit("create:a", limit, window)).unwrap();
            prop_assert_eq!(got, RateDecision::Allowed { remaining: limit - i });
        }
        let over = block_on(limiter.hit("create:a", limit, window)).unwrap();
        prop_assert!(matches!(over, RateDecision::Limited { retry_after_secs } if (1..=window).contains(&retry_after_secs)), "{:?}", over);
        let other = block_on(limiter.hit("create:b", limit, window)).unwrap();
        prop_assert_eq!(other, RateDecision::Allowed { remaining: limit - 1 });
    }
}
