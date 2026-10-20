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
Node process. A missing prefix runs the test with the wrong policy, silently: an unprefixed test that needed the oracle's
output passed in the default loop with nothing to compare (resolved drift D-3 and D-4 in [Status](status.md)).
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

**Rule 9 (generative first).** A static case proves only the input someone thought of, and the bugs that matter
here (a cross-tenant leak, a reused sequence id, a lost event) sit in the inputs and orderings nobody thought of.
A generator searches that space on every run, and the shrunk counterexample it finds becomes the static case. So
static tests keep a role, as the memory of what a generator, a fuzzer or a user already found, but they are not
where a behaviour is first specified. The three allowed kinds (regression, a named spec example, oracle output)
are the cases where the input is fixed for a reason outside the test. Labelling each one shows a reviewer why it is
static, and a static case with no label is the cue to write the generator. The ladder of generative techniques and
the seed policy are in [Strategy](strategy.md#generative-first).
