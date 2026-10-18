# Review: rust-testing (round 2)

Reviewer: skill-reviewer (fresh). Draft read in full; spike read-only at HEAD 620e2a1 plus staged edits.

**Verdict: PASS. Grade: B.** Section A clean.

## Section A
- Round-1 blocker gone; `scripts/rs` is not in `allowed-tools` (the one `scripts/rs cargo tree` command in must-fail-controls.md prompts; do not re-add `scripts/rs:*`).
- `allowed-tools` entry by entry: `contract.sh <cell>:*` x4 cannot smuggle execution (`$1` pinned and re-validated by `case`, quoted, no eval); `flake-hunt.sh:*` is the widest (TARGET/PASS flow unvalidated into `mkdir -p`; no execution); exact `contract-all.sh`, `mutants.sh`, `check-deps.sh` clean. Env-var-prefixed invocations do not match the rules and prompt.
- Residual (shared by all prefix rules): running any of these executes whatever repo code is checked out.

## Follow-ups (PR-2 prep, after the spike merges)
MEDIUM
- F1: the Postgres mutant block in `mutants.sh:41-44` has no baseline/infra guard: Docker down -> test panics -> exit 101 -> false KILLED. Say so (tag `[derived]`), require a passing baseline run before trusting that block's KILLED (must-fail-controls.md:46-51,68-70; nextest-and-mutants.md:96).
- F2: "failed tests= empty" holds only on a fresh log directory (stale per-mutant logs persist; summary.txt alone is truncated). Reword: trust the exit status; 3/4/5 are never a verdict. Soften eval 3 (SKILL.md/must-fail-controls.md:42-44; evals.json:29-35).
- F3: description trigger ("writing, running or extending tests in this Rust workspace") contradicts triggers.json:21 (pure-helper unit test should not fire). Scope to the contract suite/adapters/rejection assertions; drop "(not yet used here)" from the description.
ADVISORY
- F4: narrow wildcards to exact commands (`Bash(scripts/contract.sh native-memory main)`, exact flake-hunt cell with fixed N); upstream: validate TARGET/PASS in contract.sh and flake-hunt.sh before use in paths; confine `THRUST_WRANGLER_STATE` under `$ROOT/target/` before `rm -rf` (worker-dev.sh:19-20).
LOW
- F5 drop `scripts/rs` from the host-only `cargo tree` control (plain `cargo tree --locked --offline` printed the tree) or note it prompts.
- F6 pin merge SHA, re-run the path check against main, rephrase session-log narrative (SOURCES.md:8, 29-31, 46-51; nextest-and-mutants.md:6); provenance not reproducible from 620e2a1 (staged `contract.sh` retry loop).
- F7 "always exits 0" -> "exits 0 once the loop finishes" (SKILL.md:39).
- F8 tags outside the legend (`[derived, not tried]`, doc-tests.md:5 redefines `[re-run]`, `[unverified]` has two meanings).
- F9 guard quote is not verbatim (`VACUITY GUARD: target kind is not '<kind>'`, `adapter set is not '<set>'`, contract.sh:176,180).
- F10 network/cost note: mutants.sh includes the Postgres block (Docker + network); first build resolves crates; worker cells run `wrangler dev --local`.
- F11 ADR-0002/0007/0043 exist only as code comments; confirm they exist on main at adoption.
- F12 nextest exit 101 is BUILD_FAILED upstream; add a line so nobody "fixes" mutants.sh by accepting 101.

## Spike-side observations (for the maintainers, not skill defects)
- `crates/thrust-contract-tests/tests/contract.rs:5` names a nonexistent `scripts/contract.sh mutants` pass (the entry point is `scripts/mutants.sh`).
- `scripts/flake-hunt.sh` exits 0 whatever the iterations did.
- `scripts/mutants.sh` Postgres block can report a false KILLED when Docker is down (F1).
- `scripts/contract.sh:168` passes the admin token on curl's argv, contradicting its own header.
- Memory adapter's `allocate` leaves a gap on rollback; the Postgres adapter's does not; no HTTP-level assertion pins either.

## Spot-checks: 12 groups, all held (kill rule, recorded summary, mutant wiring, fake-vs-real allocate, check-deps.sh asymmetry, nextest exit codes, cargo-mutants exit codes, crates.io versions, expected counts 15/6, flake rate `[spike]`, token on argv, tenancy fixture).
Not re-run: author's nextest scratch-crate measurements, doc-test claims, cargo-mutants commands (not installed).

## Adoption gate
Every path the skill names exists only on the spike branch. Do not install until the spike merges; then re-verify paths against `main`.
