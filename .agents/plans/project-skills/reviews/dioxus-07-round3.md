# Review: dioxus-07 (round 3, narrow: the testing-boundary edit)

Reviewer: skill-reviewer (fresh). Report transcribed by the drafting session's lead. Scope: the four changed files only
(`SKILL.md`, `references/tooling.md`, `evals/evals.json`, `evals/triggers.json`); read against the installed `rust-test-app` and
`rust-test-logic` descriptions. No model-firing test was run.

**Verdict: PASS (Section A clean on the changed lines). Grade: B, unchanged from round 2.**

## What changed and why
`rust-test-app` owns writing tests for Dioxus components (dioxus-ssr, flush loop) and axum routers (tower `oneshot`). dioxus-07 had no
reciprocal NOT clause, so "test my dioxus component" could fire both. The description now ends `NOT for writing Dioxus component tests
or axum router tests (use rust-test-app)`; `references/tooling.md` section 7 points to `rust-test-app` for writing the tests; two
should-not-trigger and two should-trigger prompts and one routing eval (12) were added.

## Section A (changed lines)
`allowed-tools` untouched; no non-ASCII characters, comments or escapes in the diff; the only new instruction is routing to an
installed sibling; trigger breadth narrows.

## Findings and disposition
1. Regression risk LOW: the first wording ("tests of Dioxus components or axum routers") could be read with "axum routers" alone. The
   reviewer recommended `NOT for writing Dioxus component tests or axum router tests`. **Applied**; two should-trigger prompts added to
   pin it (a fullstack 404 on a server-function route with no test vocabulary; a mixed component-and-test request).
2. LOW: eval 12 expectation 4 (SSR cannot cover clicks) was a non sequitur for a prompt about classes. **Replaced** with the three
   approaches the official page lists (section 7).
3. LOW: tooling.md "only records what the official page covers" was inaccurate (section 7 also records the `dioxus-test` and
   `dioxus-ai` facts); "name which of the three supported approaches you used" sat in tension with the deferral. **Reworded**
   ("records what exists"; "name the approach the answer relies on").
4. INFO: "comparing the output" differed from rust-test-app's assertions. **Reworded** ("asserting on the rendered output").
5. INFO: no should-trigger prompt pinned the axum case. **Added** (see 1).

## Boundary after the edit
Mutual: `rust-test-app` says "NOT for Dioxus app code (dioxus-07)" and `rust-test-logic` points Dioxus tests to `rust-test-app`. A mixed
request ("write a component and a test") correctly fires both: dioxus-07 owns the component, `rust-test-app` the test.

Residual, not fixed: dioxus-07 still carries the bare "Fire on 'dioxus'" keyword, so double-fire is reduced, not eliminated; tests of a
`#[server]` function are claimed by neither skill.

## Size
`SKILL.md` 4008 of 4096 bytes (3856 before); description 769 characters.

## Not verified
Actual description-routing behaviour; whether 4096 is a harness limit or only this plan's target.

## After the review
The reviewer's recommendations were applied after the review and were **not re-reviewed**; the lead re-ran the mechanical checks
(frontmatter keys, size, `allowed-tools` unchanged, JSON validity, eval and trigger counts, the named skill is installed).
