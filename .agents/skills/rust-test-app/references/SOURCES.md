# Sources: rust-test-app

Adapted from two maintainer-supplied archives, both created 2026-10-18 and read on 2026-10-18 with
`unzip -p` (never extracted): `rust-testing-plan.zip` (skills, configs, samples) and
`reference-workspace.zip` (the validation workspace). The author and the environment are not stated
beyond the first archive's README, which says a sandbox blocked the Playwright driver download. The text
and templates here adapt the archives' skill text and samples; what differs is listed in the tables
below. No pipe-to-shell installer, wildcard tool grant, `npx` line or run-time network fetch was carried
over. The skill declares no `allowed-tools`, so every `cargo` or shell run prompts.

Licence basis: the maintainer (aRustyDev) supplied both source archives and authorises the adapted text and templates under AGPL-3.0-only, 2026-10-18. The archives themselves carry no licence file.

## Provenance of the `[re-run]` tags

Tags: `[re-run]` reproduced; `[zip]` recorded by the archive's author, not re-run; `[derived]` reasoned
from documentation or code, not executed; `[unverified: <tool> not installed]` depends on a tool missing
on the drafting machine.

Every `[re-run]` was reproduced on 2026-10-18 by the drafting session's lead (an AI session on the
maintainer's machine) on a scratch copy of the archive's workspace, not by this skill's author. The
evidence is committed in `.agents/plans/project-skills/verification/rust-testing-2026-10-18/`: its
`README.md` maps claims to `logs/`, scripts are in `scripts/`. Environment, stated once: rustc 1.98.1,
cargo-nextest 0.9.140, dioxus 0.7.10, dioxus-ssr 0.7.10, axum 0.8.9, tower 0.5.3, rstest 0.26.1,
tower-http 0.7.1, reqwest 0.13.5. Not run there: `dx`, Playwright.

## Archive files used

| Source (path inside the archive) | Used for, and what differs |
|---|---|
| `rust-testing-plan/.claude/skills/testing-e2e/SKILL.md` | adapted into the router: layer table and budget; rules (`data-testid`, never sleep, own data, logic out of `rsx!`); harness description; the `[env]` block. Differs: browser and desktop rows marked not covered; retry rule replaced (below); rules re-worded |
| `.../testing-e2e/references/api.md` | adapted into `references/api-router-tests.md`. Differs: dependency-hygiene section, E0599 hint, axum default-features finding, `cargo tree` check added |
| `.../testing-e2e/references/dioxus-components.md` | adapted into `references/dioxus-component-tests.md`. Differs: `lib` feature resolution, flush-loop cap advice, control paragraph, pointers to `handle_event`, `wait_for_suspense`, `dioxus-native-dom` |
| `.../testing-e2e/assets/api_example.rs` | `references/examples/api_example.rs`: one-line header added, nothing else |
| `.../testing-e2e/assets/harness_lib.rs` | `references/examples/harness_lib.rs`: one-line header added, nothing else |
| `.../testing-e2e/assets/e2e_harness_test.rs` | `references/examples/e2e_harness_test.rs`: header; deep-link path `/todos/42` renamed `/entries/42` |
| `.../testing-e2e/assets/ui_components_example.rs` | `references/examples/ui_components_example.rs`: identifiers renamed (`Todo` to `Entry`, `TodoList` to `EntryList`, `remaining_label` to `open_label`, `todo(..)` to `entry(..)`, test ids, titles); **changed from the archive: assertion strengthened so the class-branch control can fail** (below) |
| `rust-testing-plan/.config/nextest.toml`, `AGENTS.md`, `README.md` | read as context (prefix routing, the `e2e` package and profile, retry policy, validation claims); the facts informed the harness and retry text, no file text was copied |
| `reference-workspace/.cargo/config.toml` | adapted into `references/examples/cargo-config-env.toml`: the stub path kept as a comment, the `dx` output path made the live line |
| `reference-workspace/crates/{api,ui,e2e}/Cargo.toml` | the dependency blocks in the three references |
| `reference-workspace/crates/e2e/test-dist/index.html` | the stub shown inline in `references/harness.md` |

Repo skills read so as not to contradict them: `dioxus-07` (`SKILL.md` pin and `dx` preflight;
`references/tooling.md` section 7, testing; `references/legacy.md` and `references/cheatsheet.md`, the
`lib` feature; `references/renderer.md`, the `VirtualDom` API; `references/SOURCES.md`, the 0.8 alpha
date), `blitz-native` (`references/embedding.md`, the headless `dioxus-native-dom` note), `ts-to-rust`
(parity routing) and `mdbook` (the shape of this directory).

## Deferred: not written, on purpose

| Item | Why |
|---|---|
| `testing-e2e/references/browser.md` (playwright-rs 0.20.1, pre-1.0) | Written from documentation and never run: the archive author's sandbox blocked the driver download. No UI crate exists in this repo yet. It also contains an instruction to install a skill with `npx skills add`, which would need `skill-reviewer`. |
| `testing-e2e/references/desktop-mobile.md` | Never run; there are no desktop or mobile shells yet. |
| The browser rules in the archive's `SKILL.md` | `e2e_*` browser tests only in the `e2e` package, "only that package retries (twice)", `cargo nextest run --profile e2e e2e_login`, and the browser row of "Done when". Dropped with the reference they describe. |

The router carries "Not covered yet: browser (playwright-rs) and desktop/mobile shells; prove them on
your CI before relying on them", and the layer table keeps those rows marked "not covered here yet".
When a UI crate exists, write the browser reference from a run on that crate's CI, not from the archive.

## Changed from the archive, and added

- **Retries.** The archive allows two retries for `package(e2e)`, which exists for browser tests. This
  skill treats harness tests as hermetic and in-process: no retries, a FLAKY report is a bug. The
  archive's `nextest.toml` applies the retry override to the whole package, harness test included, and
  that was reproduced (status table); the shipped `nextest.toml` in the sibling skill is to comment the
  override out and say browser-only. Wording here: retries, if any, belong to browser tests only; none
  exist yet; never pass `--retries`, `--flaky-result`, `NEXTEST_RETRIES` or `NEXTEST_FLAKY_RESULT`.
- **Assertion strengthened** (`ui_components_example.rs`, `entry_list_renders_items_and_remaining_count`).
  The archive asserted `contains(class="done")`, which an inverted `class: if entry.done { .. }` branch
  still satisfies, so the documented control could not fail. The template now asserts
  `data-testid="entry-1" class="open"` and `data-testid="entry-2" class="done"`. Changed from the archive:
  assertion strengthened so the class-branch control can fail. The unstrengthened original cannot fail
  that control (`verify-11.log`, H5, 4 passed); the strengthened template does (H4, 3 passed, 1 failed).
- **Toy names** replaced as listed above; every "AGENTS rule N" and `testing-*` cross-reference removed or
  re-pointed to a sibling by name (`rust-test-logic`) or to `ts-to-rust`.
- **Added, not in the archive** (tagged where they appear): the dependency-hygiene section and axum's
  default `tokio` feature; the SPA-fallback-hides-API-404 gotcha; the zero-tests profile gotcha and its
  remedy; routing by package rather than by `e2e_` prefix; environment-variable precedence over the
  `[env]` block and "point it only at the build output directory"; the E0599 hint for a missing
  `ServiceExt`; the `features = ["lib"]` resolution; the flush-loop iteration cap; pointers to
  `VirtualDom::handle_event`, `wait_for_suspense` and `dioxus-native-dom`; a control for each pattern.
- **`dx build --platform web`** is kept as the archive wrote it, but as a user-run step; it is never
  reported as run here.
- **Dropped:** "a browser test costs about a thousand router tests" (`[zip]`, unsupported here) and
  "markup tests are slower" (the logs show SSR tests taking about as long as the router tests). `cargo test --doc --workspace` exit 0
  is not cited as evidence: that run executed no doctests.

## Verification status

Logs are under `.agents/plans/project-skills/verification/rust-testing-2026-10-18/logs/`.

| Claim group | Status and evidence |
|---|---|
| Default profile: 31 passed, 1 skipped; the four `ui` tests (flush loop, reference-markup compare, effect clamp, helper) and the router tests (`oneshot`, status-code tables, rstest cases and fixtures) all pass | `[re-run]`: `verify-1-nextest.log`; `verify-7.log` (a1-default) for the shipped templates |
| Harness test passes under `--profile e2e` with the stub dist; fails with exit 100 under `E2E_WEB_DIST=/nonexistent` (`web UI not built at /nonexistent`), so an exported variable beats the `[env]` block, which has no `force` | `[re-run]`: `verify-2.log` E6 (e6a, e6b); `verify-7.log` (a2-e2e) for the shipped copy |
| Default profile excludes the `e2e` package; `cargo nextest run -E 'package(e2e)'` under it selects nothing, prints `no tests to run`, exits 4 | `[re-run]`: `verify-2.log` E7 (e7c) |
| Under the archive's `retries` override for `package(e2e)`, a deterministic failure of the harness test was attempted three times (`TRY 1 FAIL`, `TRY 2 FAIL`, `TRY 3 FAIL`); the deep-link expectation changed to 404 fails the test | `[re-run]`: `verify-7.log` (c2-e2e-mutated); `verify-2.log` E6 (e6b) |
| Mutating only the production `open_label` branch (`1 => "1 items left"`) fails 2 of the 4 `ui` tests and leaves 2 green | `[re-run]`: `verify-7b.log` (g2-ui-prod-mutated). **`verify-7`'s ui control (c1) was badly designed**: it changed the production string and the assertions alike, so it stayed green; `verify-7b` is the valid control, and c1 is cited only as the reason to mutate production code |
| A per-test `flaky-result = "fail"` is only a default: `--flaky-result pass` or `NEXTEST_FLAKY_RESULT=pass` overrides it, so a retried failure goes green | `[re-run]`: `verify-9.log` (T2) |
| The four shipped Rust templates compile and run (identifier renames plus a header; `ui_components_example.rs` before the assertion change) | `[re-run]`: `verify-7.log` (a1-default, a2-e2e, r1-restored) |
| The four shipped templates as they now stand, including the strengthened class assertions in `ui_components_example.rs`: default profile 33 passed (31 plus two fuzz-template controls present in that scratch workspace), 1 skipped; `--profile e2e` 1 passed | `[re-run]`: `verify-11.log` (h1-default, h2-e2e, r1-restored) |
| Inverting `class: if entry.done { "done" } else { "open" }` in production code fails the strengthened test (`entry_list_renders_items_and_remaining_count`; 3 passed, 1 failed, exit 100); the same inversion on the archive's original template stays green (4 passed) | `[re-run]`: `verify-11.log` (h4-inverted, h5-original-inverted), script `scripts/verify-11.sh` |
| `dx build --platform web`; output under `target/dx/<app>/debug/web/public`; nextest passes `[env]` to tests; the 0.7 hook-testing guide | `[zip]`; the `dx` claims also `[unverified: dx not installed]` |
| `insta::assert_snapshot!`, `cargo insta review` | `[zip]` prose only (no validated crate depends on `insta`); `[unverified: cargo-insta not installed]` |
| SPA fallback answers unknown `/api/...` with `index.html` and 200; `-p e2e` under the default profile selects nothing; `--ignore-default-filter`; `oneshot` consumes the router; `wait_for_suspense`; `handle_event`; `dioxus-native-dom` for headless events | `[derived]`, not run |
| axum 0.8.9: default `tokio` feature enables tokio `net`, `rt`, `macros`; tower's `util` is always enabled; dioxus 0.7.10 `lib` is the four features plus `mounted`, `document`, `asset`, `warnings`, `cli-config` | `[derived]`: read from the crates' `Cargo.toml`, not built |
| The `tokio` dev-dependency line (`macros`, `rt`); the `cargo tree -p <router-crate> -e normal` check; building the Worker target with `-p` | `[derived]`, not built |
| Evals in `evals/` | authored against this text; **not run**, with or without the skill: no with/without delta exists |

## Reference lifecycle

Re-check on: a stable Dioxus 0.8 (feature names, the `NoOpMutations` path, the `dioxus-ssr` version, the
flush loop, `wait_for_suspense`); an axum release that changes route syntax, default features or
`axum::serve`; a major bump of `tower-http` or `reqwest`; the first UI crate and the first CI run with a
browser (then write the browser reference from that run); the install of `rust-test-logic` and its
siblings (the names in the router table must still resolve); and at the latest six months after
2026-10-18. A reference that cites a version here is wrong the day that version moves: say so rather
than patch quietly.
