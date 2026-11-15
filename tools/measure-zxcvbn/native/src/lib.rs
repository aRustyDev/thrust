//! What the measurement needs from zxcvbn and from the rule it would replace.
//!
//! `plane_score` is Plane's rule (D-023's "Plane behaviour" column, `apps/api/plane/authentication/utils/password.py`): zxcvbn is run on the
//! first and on the last 72 characters, and the lower score decides. A password is accepted at a score of 3 or more.
//!
//! `old_rule` restates `thrust_core::password_is_strong` as it stood at `990e1ce` (at least 10 characters, at least 5 distinct, and not a
//! common password once the digits and symbols around the letters are stripped), with an ABBREVIATED common-password list: the probe
//! passwords are nowhere near it, and a full copy would drift from core's. Where the two disagree on a probe, core's tests are right
//! and this restatement is the thing to fix.
use zxcvbn::{zxcvbn, Score};

pub fn plane_score(password: &str) -> Score {
    let chars: Vec<char> = password.chars().collect();
    let first: String = chars.iter().take(72).collect();
    let last: String = chars.iter().rev().take(72).rev().collect();
    let a = zxcvbn(&first, &[]).score();
    let b = zxcvbn(&last, &[]).score();
    if a < b {
        a
    } else {
        b
    }
}

pub fn old_rule(password: &str) -> bool {
    let common = ["password", "qwerty", "letmein", "iloveyou", "admin", "welcome", "abcdef", "zxcvbn", "changeme"];
    let letters = password.trim_matches(|c: char| !c.is_alphabetic()).to_lowercase();
    password.chars().count() >= 10
        && password.chars().collect::<std::collections::BTreeSet<_>>().len() >= 5
        && !common.contains(&letters.as_str())
}
