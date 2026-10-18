# Review: rust-test-app (round 1)

Reviewer: skill-reviewer (fresh). Report transcribed by the drafting session's lead. Nothing was built or run by the reviewer.
Read every file in the draft; diffed the four templates against the archive originals via `unzip -p`; read `dioxus-07`,
`blitz-native`, `dioxus-design-system` and `clone-frontend` for contradictions; checked tower-http, axum and dioxus sources.

**Verdict: PASS. Grade: B. Section A clean.** No edit was required to keep PASS; fixing findings 1 to 3 was the sensible minimum.

## Section A
No network fetch, credentials, installers or injection text; the draft is ASCII-only; the only URL is a Dioxus docs pointer.
Frontmatter keys are name, description, license, author, version; no `allowed-tools` (nothing pre-approved). The harness template
binds `127.0.0.1:0` only; `ServeDir` is traversal-safe (tower-http 0.7.1 rejects `..` and root after percent-decoding).

## Findings and disposition
1. MEDIUM: `[re-run]` provenance said "relayed" and named no runner; the logs lived in an ephemeral scratch directory. **Fixed**:
   the scripts and logs are committed (`.agents/plans/project-skills/verification/rust-testing-2026-10-18/`) and every row cites its
   log; the runner and date are named.
2. MEDIUM: the documented control "flip the `class: if entry.done` branch" could not fail on the template (inverting swaps the two
   classes, so `contains(class="done")` stays true). **Fixed**: the assertion now checks the test id and class together per item.
   Reproduced afterwards (`verify-11`): the inversion in production code now fails the test, and the same inversion on the archive's
   original template stays green.
3. MEDIUM: the four-feature Dioxus list does not compile for crates that use `asset!` or `document::` (0.7.10's `lib` feature adds
   `mounted`, `document`, `asset`, `warnings`, `cli-config`). **Fixed**: the caveat is resolved and eval 2 corrected.
4. LOW: the E0599 hint was imprecise (axum 0.8.9 always enables tower `util`; a missing `use` gives E0599, a missing feature
   would give E0432). **Fixed**.
5. LOW: the dependency-hygiene warning was understated (axum's default `tokio` feature enables tokio `net`, `rt`, `macros`).
   **Fixed**: stated as fact with the `default-features = false` remedy and a locator for "the dependency guard".
6. LOW: a zero-test run exits 4 (reproduced); the cited `cargo test --doc` evidence ran 0 doctests. **Fixed**.
7. LOW: `e2e_` is not the routing key (the configs route on `package(e2e)`). **Fixed**.
8. LOW: the harness serves the whole `E2E_WEB_DIST` subtree and an exported variable silently wins; the flush loop is unbounded.
   **Fixed** (point only at the build output directory; cap the iterations).
9. LOW: NOT clause named no logic, fuzz or mutation sibling; triggers and eval 4 disagreed on playwright-rs; "markup tests are
   slower" untagged; absolute snapshot-reaccept ban conflicted with eval 9; no evals for the SPA-fallback and zero-test gotchas.
   **Fixed** (evals 12 and 13 added).

Not changed: `dioxus-07` (merged, tooling section 7 covers testing) has no reciprocal NOT clause, so "test my dioxus component" can
fire both skills. Recorded as a known boundary; any change is a separate edit to a merged skill and would need its own review.

## Collisions
`dioxus-07` (above); `rust-test-logic` (insta and API call-sequence model tests; routing correct both ways); `ts-to-rust` (replay).
No competing skill in the other roots.

## Spot-checks: all held
Default profile 31 passed, 1 skipped; `--profile e2e` 1 passed; a bad `E2E_WEB_DIST` fails with exit 100; a `-E` filter that
selects nothing exits 4; `TRY 1/2/3 FAIL` under the archive's retry override; mutating only the production `open_label` branch fails 2
of 4 tests while changing every occurrence of the string stays green; `--flaky-result pass` and the env var override the per-test
config. The diff claim is true: four templates carry a header line; e2e renames `/todos/42` to `/entries/42`; ui renames
identifiers, titles and a CSS class. The flush-loop symptom is confirmed from dioxus-core 0.7.10 source.

## Not verified
The `dx` output path and flags; a ready fake with `use_resource` loaded state; `handle_event` and `dioxus-native-dom`.

## After the review
The fixes were applied after this review and were **not re-reviewed by `skill-reviewer`**. The lead re-ran the mechanical gate,
compiled all four final templates, and ran the strengthened class-branch control (`verify-11`). Browser tests (playwright-rs) and
desktop/mobile shells are deferred by decision, and the router says "not covered yet".
