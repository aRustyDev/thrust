# Must-fail controls

**A verification that cannot fail is not evidence.** A green run only means something once a known-bad
input has been shown to turn it red. Every new "X is rejected" assertion ships with a control; a
control that passes voids both itself and the result it was guarding.

Evidence: `scripts/mutants.sh`, `crates/thrust-adapters-memory/{Cargo.toml,src/lib.rs}`,
`crates/thrust-adapters-selfhosted/{Cargo.toml,src/pg.rs}`, `crates/thrust-server/Cargo.toml`,
`scripts/contract.sh`, `scripts/check-deps.sh`. Tags are defined in `SKILL.md`; provenance in `SOURCES.md`.

## Three layers of control in this repo

| Layer | Mechanism | Catches |
|---|---|---|
| Inline | each "allowed" is paired with the neighbouring "rejected" in the same test (`contract-suite.md`) | a server that accepts or rejects everything |
| Whole suite | `scripts/mutants.sh`: build a deliberately broken target, require the suite to FAIL | an assertion that observes nothing |
| Harness | `VACUITY GUARD` exits in `scripts/contract.sh`; "precondition not yet true" asserts in async tests; the `check-deps.sh` positive control (below) | a run that exercised the wrong thing, or nothing |

## `scripts/mutants.sh`

Its header: "A mutant that survives means some assertion is vacuous." The list of mutants is the loop at
the top of the script plus the Postgres block below it; read them there rather than from documentation.
A mutant is chosen to break one specific rule, so the summary shows *which tests* guard that rule.

### Reading the summary

The format is the script's own (`target/mutants/summary.txt`); the first block is a real KILLED entry
shape, the last two rows are invented to show the other outcomes.

```text
native-memory + ignore-tenant      KILLED   (3 test(s) failed)
    test cross_tenant_access_is_rejected_and_has_no_effect ... FAILED
    test attachments_roundtrip_with_authz ... FAILED
    test outbox_event_reaches_the_queue_consumer_exactly_once ... FAILED
native-memory + new-rule           SURVIVED or did not run (exit=0, failed tests=0)
native-memory + broken-build       SURVIVED or did not run (exit=3, failed tests=)
```

- **KILLED:** read the listed tests. They are the assertions that guard the rule; yours must be among them.
- **`exit=0, failed tests=0`:** the whole suite passed against the broken build. Either an assertion is
  vacuous or the mutant does not break what you think. Fix the test or the mutant.
- **`exit=3` (or 4, 5), `failed tests=` empty:** no verdict. The build, the database or the target did
  not come up, so there is no test log. This is not evidence of a vacuous assertion; read the build log
  under the mutant's log directory and re-run.

### The kill rule (read it before editing the script)

`kill_check` counts a mutant as KILLED only when the run exits **101** (cargo test failed) **and** at
least one line ends `... FAILED`. Anything else, including a build failure (`contract.sh` exit 3), an
unhealthy target (exit 4) or a vacuity guard (exit 5), is reported `SURVIVED or did not run` and the
script exits 1. A mutant that merely fails to compile is therefore not a kill. Keep that property.

The pass-count guard in `contract.sh` only fires on exit 0, so a mutant run can print e.g.
`passed=12 expected=15` with exit 101; that is expected, not a guard failure.

### How an in-memory mutant is wired

1. A `mutant_*` feature on the crate that owns the behaviour (`thrust-adapters-memory/Cargo.toml`), with a
   comment that it is never enabled outside `scripts/mutants.sh`.
2. A pass-through feature on `thrust-server` (`mutant_x = ["thrust-adapters-memory/mutant_x"]`).
3. A `#[cfg(feature = "mutant_x")]` / `#[cfg(not(feature = "mutant_x"))]` pair at **one** site, with a
   `MUTANT (control run only)` comment.
4. A `name:feature` pair added to the `for pair in ...` loop at the top of `scripts/mutants.sh`. The
   script passes the feature to `contract.sh` through the `THRUST_SERVER_FEATURES` variable, which only
   `contract.sh` reads.
5. Run `scripts/mutants.sh` and confirm the summary names the test you meant to protect.

These steps cover in-memory mutants only. A Postgres adapter mutant is a separate block in the script: a
feature on `thrust-adapters-selfhosted` and a direct `cargo test --features <mutant> --test pg_adapter --
<test filter>` run judged by the same kill rule. Adding another one means mirroring that block.

### Check coverage before relying on it

A rule has whole-suite protection only if a mutant in `scripts/mutants.sh` breaks it. List the rules your
assertion touches and compare them with the loop and the Postgres block; a rule with only an inline
control has no whole-suite control until someone adds the mutant. When no existing target can host the
mutant, say so in the PR; do not skip the control silently.

## Guard scripts need a positive control too

`scripts/check-deps.sh` treats `did not match any packages` and `nothing to print` as "absent". So a
misspelled entry in its `FORBIDDEN` list passes forever. Positive control: a crate that does depend on
the thing must produce a hit:

```sh
scripts/rs cargo tree -p thrust-server -e normal --target all -i tokio   # must print a tree
```

A mistyped *subject* (`-p`) is not silent: with `-i`, cargo ignores the unknown package and prints the
whole-workspace tree, which the script reports as a violation. Fail-closed there, fail-open on the
forbidden list: that asymmetry is why the control belongs next to any edit of that list.

## Extras beyond the one procedure in `SKILL.md`

1. State what the assertion observes ("a request with tenant B's token never reads tenant A's row"),
   and make the mutant remove exactly that behaviour.
2. If you cannot run a needed tool, report it `[unverified: <tool> not installed]`; never "PASS" a
   control you did not execute.
