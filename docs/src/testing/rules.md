# Rules

The binding testing rules live in `AGENTS.md`, which every agent session loads. They are included here verbatim, so
this page cannot drift from them; the notes below explain why each exists. The notes add no rules.

{{#include ../../../AGENTS.md:testing-rules}}

## Why

**Rule 1 (nextest).** nextest runs each test in its own process, applies per-category timeouts and makes the
retry policy part of the config instead of the command line. Doctests are the one thing it cannot run. The contract
suite and `pg_adapter` still run on `cargo test` through scripts that parse its `test result:` line; that is
deliberate and documented in `AGENTS.md`, not an exception to copy.

**Rule 2 (no retry or flaky overrides).** A randomized test that failed once has found an input that breaks the
code. A retry that passes hides it. `flaky-result = "fail"` in `.config/nextest.toml` is only a default:
`--flaky-result pass` turns a retried failure green.

**Rule 3 (a failing test is a finding).** Editing a test until it passes converts a known defect into an unknown
one. If the test really was wrong, the commit message is where that argument is recorded and reviewed.

**Rule 4 (prefixes).** The prefix is how `.config/nextest.toml` routes a test to its timeout, its retry policy and
its profile: `oracle_` tests are excluded from the default loop and grouped to two at a time because each starts a
Node process. A missing prefix runs the test with the wrong policy, silently (drift item D-3 in [Status](status.md)).
The contract suite has a second, separate set of prefixes (`sweeper_`, `panic_`, `worker_bindings_`, …) that select a
*pass* in `scripts/contract.sh` rather than a nextest policy; see [Contract suite](techniques/contract-suite.md).

**Rule 5 (tests in their crate).** cargo-mutants runs only the mutated package's own tests. A crate whose only tests
live elsewhere reports every mutant as unkillable, or stops at the baseline with exit 4.

**Rule 6 (hermetic default).** The inner loop must give the same answer on every machine and every run, fast.
Anything needing Docker, a database, wrangler or Node sits behind a script or an opt-in profile so that its absence
is a visible skip, never a hidden pass.

**Rule 7 (commit test data).** Shrunk proptest counterexamples, fixtures and fuzz seeds are the memory of every bug
already found; without them the next run starts from zero. `mutants.out*` is a run report, not test data.

**Rule 8 (a check that cannot fail is not evidence).** Every live suite in this repo has had a vacuous pass at some
point: a filter selecting nothing, a target that was the wrong adapter, a replay with no inputs. The controls that
caught them are catalogued in [Evidence and controls](evidence-and-controls.md).
