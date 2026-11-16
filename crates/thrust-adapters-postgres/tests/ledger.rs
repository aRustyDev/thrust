//! The migration ledger's arithmetic (issue #195), without a database: [`plan`] against a reference model over every small world and
//! over generated larger ones, what applying a plan leaves, the JSON `migrate --plan` prints, what a contract file is, and the list of
//! files against the `migrations/` directory. The same code against a real Postgres is `ledger` in
//! `thrust-adapters-selfhosted/tests/pg_adapter.rs`.
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
use thrust_adapters_postgres::ledger::{is_contract, plan, sha256_hex, Entry, Plan};
use thrust_adapters_postgres::{carried, MIGRATIONS};

fn entry(file: &str, sha: &str) -> Entry {
    Entry { file: file.to_string(), sha256: sha.to_string() }
}

/// The reference: the rules of `ledger::plan`'s documentation, written with lists and `contains` instead of maps, so that it is not
/// the code under test written twice.
fn reference(known: &[Entry], ledger: &[Entry]) -> Plan {
    let mut changed: Vec<String> = known
        .iter()
        .filter(|k| ledger.iter().any(|l| l.file == k.file && l.sha256 != k.sha256))
        .map(|k| k.file.clone())
        .collect();
    changed.sort();
    if !changed.is_empty() {
        return Plan::Changed { files: changed };
    }
    let mut pending: Vec<String> =
        known.iter().filter(|k| !ledger.iter().any(|l| l.file == k.file)).map(|k| k.file.clone()).collect();
    let mut unknown: Vec<String> =
        ledger.iter().filter(|l| !known.iter().any(|k| k.file == l.file)).map(|l| l.file.clone()).collect();
    pending.sort();
    unknown.sort();
    if unknown.is_empty() {
        let newest = ledger.iter().map(|l| l.file.clone()).max();
        if let (Some(first), Some(newest)) = (pending.first(), newest) {
            if *first < newest {
                return Plan::OutOfOrder { pending, newest_applied: newest };
            }
        }
        let contract = pending.iter().filter(|f| is_contract(f)).cloned().collect();
        return Plan::Apply { pending, contract };
    }
    if pending.is_empty() {
        Plan::Older { unknown }
    } else {
        Plan::Diverged { pending, unknown }
    }
}

/// A world: for each file of a small universe, whether the build has it and with which checksum, and the same for the ledger.
/// `None` = absent; `Some(0)` and `Some(1)` = two different contents. The universe has a contract file in the middle, so every
/// ordering case (a contract file pending before, between or after recorded ones) is in it.
const UNIVERSE: [&str; 4] = ["0001_a.sql", "0002_contract_drop.sql", "0003_c.sql", "0004_d.sql"];
const SHAS: [&str; 2] = ["aaaa", "bbbb"];

fn side(choice: &[Option<usize>]) -> Vec<Entry> {
    UNIVERSE.iter().zip(choice).filter_map(|(file, c)| c.map(|i| entry(file, SHAS[i]))).collect()
}

/// Every world of the universe: each of the four files absent or one of two contents, on each side (3^8 = 6561 worlds).
fn every_world() -> impl Iterator<Item = (Vec<Entry>, Vec<Entry>)> {
    let choices = [None, Some(0), Some(1)];
    (0..3usize.pow(8)).map(move |mut n| {
        let mut picks = Vec::with_capacity(8);
        for _ in 0..8 {
            picks.push(choices[n % 3]);
            n /= 3;
        }
        (side(&picks[..4]), side(&picks[4..]))
    })
}

/// Exhaustive over the small universe: `plan` is the reference in every world, and every outcome is reached (the control that the
/// enumeration is not missing a class: a world generator that never made, say, an out-of-order case would fail here).
#[test]
fn model_plan_matches_the_reference_in_every_small_world() {
    let mut seen = std::collections::BTreeSet::new();
    for (known, ledger) in every_world() {
        let got = plan(&known, &ledger);
        assert_eq!(got, reference(&known, &ledger), "known {known:?}, ledger {ledger:?}");
        seen.insert(match &got {
            Plan::Apply { pending, .. } if pending.is_empty() => "current",
            Plan::Apply { contract, .. } if !contract.is_empty() => "apply with a contract file",
            Plan::Apply { .. } => "apply",
            Plan::Older { .. } => "older",
            Plan::Diverged { .. } => "diverged",
            Plan::OutOfOrder { .. } => "out of order",
            Plan::Changed { .. } => "changed",
        });
    }
    assert_eq!(seen.len(), 7, "every outcome is reached: {seen:?}");
}

/// A larger universe, generated: file numbers up to 40, each side a random subset with one of two contents.
fn arb_side() -> impl Strategy<Value = Vec<Entry>> {
    prop::collection::btree_map(1u16..40, (any::<bool>(), 0usize..2), 0..20).prop_map(|m| {
        m.into_iter()
            .map(|(n, (contract, sha))| {
                let file = if contract { format!("{n:04}_contract_x.sql") } else { format!("{n:04}_x.sql") };
                entry(&file, SHAS[sha])
            })
            .collect()
    })
}

/// A build and a database that recorded a prefix of what the build has: the common case, an up-to-date or a lagging database.
fn arb_build_and_its_prefix() -> impl Strategy<Value = (Vec<Entry>, Vec<Entry>)> {
    arb_side().prop_flat_map(|known| {
        let n = known.len();
        (Just(known), 0..=n).prop_map(|(known, cut)| {
            let ledger = known[..cut].to_vec();
            (known, ledger)
        })
    })
}

proptest! {
    #![proptest_config(proptest_config(256))]

    /// The same rule as the exhaustive test, on worlds too big to enumerate.
    #[test]
    fn prop_plan_matches_the_reference(known in arb_side(), ledger in arb_side()) {
        prop_assert_eq!(plan(&known, &ledger), reference(&known, &ledger));
    }

    /// Applying a plan and recording what it applied leaves nothing to apply; an older build applies nothing, and stays older.
    #[test]
    fn prop_after_applying_the_plan_nothing_is_pending(known in arb_side(), ledger in arb_side()) {
        match plan(&known, &ledger) {
            Plan::Apply { pending, .. } => {
                let mut after = ledger.clone();
                after.extend(known.iter().filter(|k| pending.contains(&k.file)).cloned());
                prop_assert_eq!(plan(&known, &after), Plan::Apply { pending: vec![], contract: vec![] });
            }
            older @ Plan::Older { .. } => prop_assert_eq!(plan(&known, &ledger), older),
            refused => prop_assert!(
                matches!(refused, Plan::Diverged { .. } | Plan::OutOfOrder { .. } | Plan::Changed { .. }),
                "{:?}",
                refused
            ),
        }
    }

    /// A database that recorded a prefix of the build's files is never refused, and has exactly the rest to apply.
    #[test]
    fn prop_a_lagging_database_gets_exactly_the_rest((known, ledger) in arb_build_and_its_prefix()) {
        let rest: Vec<String> = known[ledger.len()..].iter().map(|e| e.file.clone()).collect();
        let contract: Vec<String> = rest.iter().filter(|f| f.contains("_contract_")).cloned().collect();
        prop_assert_eq!(plan(&known, &ledger), Plan::Apply { pending: rest, contract });
    }

    /// `is_contract` reads any text without panicking (a multibyte character where the number should be included).
    #[test]
    fn prop_is_contract_reads_any_name(name in "\\PC{0,24}") {
        let _ = is_contract(&name);
    }

    /// The checksum is 64 lowercase hex digits, and different SQL gives a different one.
    #[test]
    fn prop_the_checksum_is_hex_and_tells_contents_apart(a in "\\PC{0,64}", b in "\\PC{0,64}") {
        let (ha, hb) = (sha256_hex(&a), sha256_hex(&b));
        prop_assert_eq!(ha.len(), 64);
        prop_assert!(ha.bytes().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c)), "{}", ha);
        prop_assert_eq!(a == b, ha == hb);
    }
}

/// What a contract file is, over every combination of the four parts of its name, against the rule (four digits, `_contract_`, a
/// name that is not empty, `.sql`).
#[rstest]
fn a_contract_file_has_four_digits_the_word_a_name_and_sql(
    #[values("0018", "018", "00a8", "00018", "")] number: &str,
    #[values("_contract_", "_contracts_", "_expand_", "_Contract_", "contract_")] word: &str,
    #[values("", "drop_old_column")] what: &str,
    #[values(".sql", ".SQL", "")] ext: &str,
) {
    let name = format!("{number}{word}{what}{ext}");
    let want = number == "0018" && word == "_contract_" && !what.is_empty() && ext == ".sql";
    assert_eq!(is_contract(&name), want, "{name}");
}

/// The JSON `migrate --plan` prints: the five keys are always there, as lists, and `outcome` names the plan. Over every world of the
/// small universe, with and without a ledger.
#[test]
fn the_plan_as_json_always_has_its_five_keys() {
    for (known, ledger) in every_world() {
        for present in [true, false] {
            let p = plan(&known, &ledger);
            let json = p.to_json(present);
            for key in ["pending", "contract", "unknown", "changed"] {
                assert!(json[key].is_array(), "{key} in {json}");
            }
            let want = match &p {
                Plan::Apply { .. } if !present => "baseline",
                Plan::Apply { pending, .. } if pending.is_empty() => "current",
                Plan::Apply { .. } => "apply",
                Plan::Older { .. } => "older",
                Plan::Diverged { .. } => "diverged",
                Plan::OutOfOrder { .. } => "out_of_order",
                Plan::Changed { .. } => "changed",
            };
            assert_eq!(json["outcome"], want, "{json}");
            assert_eq!(json.get("newest_applied").is_some(), matches!(p, Plan::OutOfOrder { .. }), "{json}");
            let listed = |key: &str| -> Vec<String> {
                json[key].as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_string()).collect()
            };
            match &p {
                Plan::Apply { pending, contract } => {
                    assert_eq!(&listed("pending"), pending);
                    assert_eq!(&listed("contract"), contract);
                }
                Plan::Older { unknown } => assert_eq!(&listed("unknown"), unknown),
                Plan::Diverged { pending, unknown } => {
                    assert_eq!(&listed("pending"), pending);
                    assert_eq!(&listed("unknown"), unknown);
                }
                Plan::OutOfOrder { pending, newest_applied } => {
                    assert_eq!(&listed("pending"), pending);
                    assert_eq!(json["newest_applied"], newest_applied.as_str());
                }
                Plan::Changed { files } => assert_eq!(&listed("changed"), files),
            }
        }
    }
}

/// Pinned regression (#195): a database from before the ledger has nothing recorded, so the plan is every file, and `--plan` calls
/// it the baseline. Today's production and staging databases take this path once.
#[test]
fn a_database_from_before_the_ledger_gets_every_file_once() {
    let known = carried(&MIGRATIONS);
    let p = plan(&known, &[]);
    let every: Vec<String> = MIGRATIONS.iter().map(|m| m.file.to_string()).collect();
    assert_eq!(p, Plan::Apply { pending: every, contract: vec![] });
    assert_eq!(p.to_json(false)["outcome"], "baseline");
    // and a database that has all of them is current
    assert_eq!(plan(&known, &known), Plan::Apply { pending: vec![], contract: vec![] });
}

/// The adapter's list of files is the `migrations/` directory: every `.sql` file in it, once, in name order, each with the content
/// of its file. A file added to the directory and not to `MIGRATIONS` would never be applied.
#[test]
fn the_list_of_migrations_is_the_directory() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
    let mut on_disk: Vec<(String, String)> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "sql"))
        .map(|p| (p.file_name().unwrap().to_string_lossy().into_owned(), std::fs::read_to_string(&p).unwrap()))
        .collect();
    on_disk.sort();
    let listed: Vec<(String, String)> = MIGRATIONS.iter().map(|m| (m.file.to_string(), m.sql.to_string())).collect();
    assert!(on_disk.len() >= 17, "the control: the directory was read ({} files)", on_disk.len());
    assert_eq!(listed, on_disk);
    // and the checksums `carried` reports are of those contents
    for (e, m) in carried(&MIGRATIONS).iter().zip(MIGRATIONS.iter()) {
        assert_eq!((e.file.as_str(), e.sha256.clone()), (m.file, sha256_hex(m.sql)));
    }
}

/// The SHA-256 of the empty string and of "abc", the examples FIPS 180-2 gives (spec examples, not generated).
#[rstest]
#[case::empty("", "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855")]
#[case::abc("abc", "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad")]
fn the_checksum_is_sha256(#[case] sql: &str, #[case] want: &str) {
    assert_eq!(sha256_hex(sql), want);
}
