# The TDD loop

Every behaviour change goes through these steps in order. The loop is the same for every layer; only the runner and
the controls change. Steps 3 and 5 are the ones agents skip, and they are the ones that make the rest worth
anything.

## 1. Find the requirement

Look up the feature (`F-…`) or requirement (`R-…`) in `.agents/plans/contrail/TRACEABILITY.md`. Its **Test home**
column says at which layer the acceptance evidence must live (for example R-S1 → "property tests (user never sees
rows outside scope)", R-F2 → "spike 0002 concurrency test"). If your change has no row, say so in the PR; do not
invent an id.

## 2. Pick the tests

Use [Which tests a change needs](by-change.md). Choose the **lowest layer that can catch the defect**, then add the
row's backstops. Load the owning skill named on each technique page before writing the test.

## 3. Red, for the right reason

Write the test first and run it. It must fail **with an assertion about the behaviour you are about to add**.
These do not count as red:

- a compile error (stub the function with the right signature first, returning a wrong but typed value);
- a `todo!()` / `unimplemented!()` panic;
- nextest exit 4, "no tests to run" (your filter or prefix selected nothing: see [Running](running.md));
- a contract script failing on its target guard or count guard rather than on the new test;
- a test that "fails" because an external tool (Docker, wrangler, Node) is missing.

Keep the red output; step 8 cites it.

## 4. Green, under the right runner

Make the smallest change that passes. Run it with the runner that owns the test (nextest for hermetic tests,
`scripts/contract.sh <target> <pass>` for the contract suite, see [Running](running.md)) and check the reported
count includes your test.

## 5. Control

Show that the test can fail for the defect it claims to catch: pick the matching control in [Evidence and
controls](evidence-and-controls.md), apply it, see red, revert it, see green. For contract and adapter invariants
that guard security or uniqueness, the durable form is a planted `mutant_*` feature added to `scripts/mutants.sh`.
This is [Rule 8](rules.md).

## 6. Refactor while green

Tidy the code and the test. Re-run after each step. Never touch the assertion to keep green ([Rule 3](rules.md)).

## 7. Mutation

Ask whether the tests as a whole would notice a wrong implementation:

- if the crate has tests of its own: `cargo mutants -f <changed file>` and fix every MISSED mutant through the public
  API (`rust-test-mutation`);
- if its only tests are the contract suite (most crates today, see [Status](status.md)): `scripts/mutants.sh`, and add
  a planted mutant for any new invariant that matters.

## 8. Definition of done

- [ ] Every row of [by-change](by-change.md) that matches the change has its tests, or a recorded gap.
- [ ] Each new test was seen red for the right reason, and the commit or PR body shows that output.
- [ ] Each new test has its control, applied and reverted.
- [ ] Tests carry their category prefix ([Rule 4](rules.md)) and live in the crate they test ([Rule 5](rules.md)).
- [ ] The default nextest profile is still hermetic ([Rule 6](rules.md)); anything needing Docker, wrangler or Node
      is behind a script or opt-in profile.
- [ ] Test data is committed: regressions, fixtures, seeds ([Rule 7](rules.md)).
- [ ] Mutation step run, with MISSED mutants fixed or each skip justified in writing.
- [ ] The PR names the `F-`/`R-` id it serves, or says it has none.
- [ ] No test was deleted, weakened, ignored or retried to get green ([Rules 2 and 3](rules.md)).
