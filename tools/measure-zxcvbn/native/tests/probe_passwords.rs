//! The probe passwords of the local-workerd and Cloudflare runs are only useful if they split the two rules: the old rule accepts them
//! and zxcvbn refuses them, so a 400 for them can only have come from zxcvbn. These tests are that control, and they can fail.
//!
//! Which passwords: `short` is refused by the old rule's length bound before zxcvbn runs (the request that does not touch zxcvbn, "S");
//! `abcdefghijkl` and `qazwsxedcrfv` are accepted by the old rule and refused by zxcvbn ("Z"); the passphrase passes both.
use zxcvbn::{zxcvbn, Score};
use zxcvbn_first_call::{old_rule, plane_score};

const WEAK_FOR_ZXCVBN: [&str; 2] = ["abcdefghijkl", "qazwsxedcrfv"];
const STRONG: &str = "tr0ub4dor-&-3-horses-lantern";

#[test]
fn the_weak_probes_split_the_two_rules() {
    for p in WEAK_FOR_ZXCVBN {
        assert!(old_rule(p), "{p:?} must pass the old rule, or a 400 for it proves nothing about zxcvbn");
        assert!(plane_score(p) < Score::Three, "{p:?} must be refused by zxcvbn (score {:?})", plane_score(p));
    }
}

#[test]
fn the_strong_probe_passes_both_rules() {
    assert!(old_rule(STRONG));
    assert!(plane_score(STRONG) >= Score::Three, "score {:?}", plane_score(STRONG));
}

#[test]
fn the_short_probe_is_refused_by_the_old_rule_before_zxcvbn_runs() {
    assert!(!old_rule("short"));
}

/// Plane scores the first and the last 72 characters and the lower decides. A password whose first 31 characters are strong and whose last
/// 80 are one letter repeated scores well as a whole, and badly on its last 72: so a rule that scored only the whole, or only the first 72,
/// would accept it. (If this fails because the whole scores low too, the construction no longer separates the cases: change it. The first
/// version of this test used the alphabet three times over as the suffix, and zxcvbn scored its last 72 characters Three: a few sequence
/// pieces still cost an attacker 1e8 guesses or more, so that password did not separate the cases and the test said so.)
#[test]
fn plane_scores_the_last_72_characters_too() {
    let strong_prefix = "k7#Qm2$vXz9&Lp4!Rt8@Wn3^Bd6*Yc1";
    let weak_suffix = "a".repeat(80);
    let p = format!("{strong_prefix}{weak_suffix}");
    assert!(p.chars().count() > 72, "the truncation only matters past 72 characters");
    assert!(
        zxcvbn(&p, &[]).score() >= Score::Three,
        "the whole password must score well for this to separate the cases"
    );
    assert!(
        plane_score(&p) < Score::Three,
        "the last 72 characters are one letter repeated: Plane refuses it (score {:?})",
        plane_score(&p)
    );
}
