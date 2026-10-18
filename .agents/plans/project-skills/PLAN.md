# Plan: proactive project skills for `thrust`

## Status (2026-10-18)

The two-PR split below was replaced by an instruction to open **one PR per skill**. Each reviewed skill is
installed on its own branch (`feat/skill-<name>`) at `.agents/skills/<name>/`, with the shared `.claude/skills`
symlink. Review verdicts are in `reviews/`. Plans live under `.agents/plans/` (they were first written under
`.claude/plans/`; the tree was moved by instruction, so older commit messages and PR text may still use the old path).

| Skill | State | Open items |
|---|---|---|
| `mdbook`, `dioxus-07`, `dioxus-design-system`, `blitz-native` | merged (PRs #1 to #4; review records moved to `.agents/plans/` by #5) | post-review edits were not re-reviewed by `skill-reviewer` |
| `planning` | PR #6 | added after the original plan; adapted from the maintainer's convention and sanitized; the GitNexus `gitnexus-plan` skill is not bundled (PolyForm Noncommercial 1.0.0 is incompatible with AGPL-3.0-only); post-review fixes not re-reviewed |
| `clone-frontend` | PR #7 (PASS, B, four rounds) | round-4 fixes not re-reviewed; in-page JS unverified |
| `ts-to-rust` | PR #8 (PASS, B, four rounds) | round-4 fixes not re-reviewed |
| `rust-on-workers`, `rust-testing` | held until the spike merges (PASS, B, round 2) | re-verify every cited path on merged `main`, re-stamp provenance, locate the ADR text; narrow `rust-testing`'s `scripts/contract.sh <cell>:*` and `flake-hunt.sh:*` wildcards to exact commands |
| `dockerfile`, `helm-chart`, `gha-workflow-design` | held until the spike merges (PASS, B, round 1 only) | fixes from round 1 were applied but **not re-reviewed**; re-review after the paths are on `main`; apply the later batch decisions (extension-qualified `Edit` globs, depth-agnostic boundary sentence, `license`) |

Never done: the evals were not run against the final skill text; `rust-analyzer-lsp`, Context7 authentication and the
other user-level items in the checklist below were not executed; the PR-2 skills reference files that exist only on
`feat/spike-0001-rust-on-workers`.

## Context
The pasted report recommends skills/MCPs/plugins for a Rust/Dioxus + infra + CI stack. `thrust` is a
Plane-compatible, Cloudflare-native, self-hostable project-management platform. At the time, `main` held only
LICENSE/README/.gitignore; all real code was on `feat/spike-0001-rust-on-workers` (in a separate local
worktree, with uncommitted changes).
ADR-0006 (Rust vs TS core) is still gated on that spike. The user chose **everything in the report** and
**`.agents/skills/` + `.claude/skills` symlink**.

Deliverable: 11 reviewed, committed skills for the repo. Third-party plugins/MCPs/skills stay **out** of the repo
(user-level, user-run checklist below).

## Where the report is wrong or incomplete for this machine/repo (verified)
| Report says | Reality | Plan response |
|---|---|---|
| "Context7 already covers Dioxus 0.7 / Blitz" | Context7 is **unauthenticated in this session** (only `authenticate` tools exist) | Skills name Context7 library ids as preferred source, with a `WebFetch`-of-official-pages fallback. User must authenticate. |
| Vendor `references/llms-full.txt` (~467K tokens) | Quality matrix benchmarks references at 14-37 KB | **Do not vendor.** Cite the URL/library id; curate a small cheat sheet. |
| Install `hashicorp/agent-skills` | Its terraform skills are already installed at user level (style-guide, test, stacks, refactor-module, search-import, provider-*) | Skip. |
| Slack plugin "likely already installed" | Installed set is messaging/search only; the dev skills (`create-slack-app`, `block-kit`) are absent. But nothing in the repo touches Slack. | Out of scope; noted. |
| `rust-testing` = proptest-state-machine / mockall / wiremock | Repo method is: shared HTTP contract suite run against every adapter, in-memory adapters instead of mocks, testcontainers Postgres, hand-rolled mutation controls (`mutants.sh`: "prove the suite can fail"), `flake-hunt.sh`, dep-graph guard (`check-deps.sh`) | Spine = repo method. proptest/mockall/wiremock/doc-tests become `references/`, tagged "not yet used in repo". |
| (silent) workers-rs / wasm32 | This is the repo's actual core, and the report has nothing on it | **Add `rust-on-workers`** (not in the report). Defers to installed `cloudflare:workers-best-practices`, `wrangler`, `durable-objects` for JS-side. |
| `rust-analyzer-lsp` plugin | Exists in the local official marketplace, but `rust-analyzer` is not installed and Homebrew `rustc` shadows rustup (see `scripts/rs` header) | User-level checklist item with the PATH caveat. |

## The 11 skills
Router `SKILL.md` small (target <=4 KB), depth in `references/`, `evals/evals.json` ships with each.

| # | Skill | Job / trigger | Ground truth | PR |
|---|---|---|---|---|
| 1 | `dioxus-07` | rsx!/signals/Stores/server fns/router/assets/`dx`; banned legacy (`cx: Scope`, `use_state`, `render!`; `to_owned!` was dropped from the list after review: the macro still exists in 0.7.10); `references/renderer.md` from custom_renderer page + AGENTS.md | Official 0.7 docs (Context7 / fetched pages); local `dx` | 1 |
| 2 | `dioxus-design-system` | `dx components add/list`, dioxus-primitives, theme CSS vars, Tailwind auto-detect, shadcn mapping, a11y checklist | DioxusLabs/dioxus-components | 1 |
| 3 | `blitz-native` | dioxus-native/Blitz crate map, Stylo/Taffy/Parley/Vello roles, CSS subset, embedding | DioxusLabs/blitz | 1 |
| 4 | `clone-frontend` | capture -> tokens -> `dx translate` -> components -> visual diff | `dx translate`; Playwright/Chrome DevTools MCP (versions unverified in report) | 1 |
| 5 | `ts-to-rust` | scope -> freeze tests -> redesign -> port -> verify -> anti-pattern gate; TS->Rust type map; module-by-module | Report's `rust-porter` / cpp-to-rust method as *source material only*; drop the conflicting cost figures | 1 |
| 6 | `mdbook` | SUMMARY, book.toml, preprocessors, `mdbook test -L`, `{{#include}}` anchors shared with doc-tests, linkcheck, Pages | local `mdbook` (verify in scratch book) | 1 |
| 7 | `dockerfile` | cargo-chef multi-stage, BuildKit cache mounts, non-root, HEALTHCHECK (+ distroless probe), .dockerignore | local `docker` (`build --check`); hadolint absent | 1 |
| 8 | `helm-chart` | values.schema.json, `helm lint/template`, helm-unittest, `ct` (self-host packaging) | local `helm` | 1 |
| 9 | `rust-on-workers` | workers-rs, wasm32, `worker-build`, `!Send` futures vs axum, `strip=true` kills externref, compat-date limits, rustup-vs-Homebrew toolchain fault | spike: `scripts/rs`, `wrangler.toml`, `probes/send-shape`, `Cargo.toml` profile comments | 2 |
| 10 | `rust-testing` | contract-first testing + must-fail controls (spine); refs for doc-tests, proptest-state-machine, mockall/wiremock, nextest | spike: `crates/thrust-contract-tests`, `scripts/contract*.sh`, `mutants.sh`, `flake-hunt.sh`, `check-deps.sh` | 2 |
| 11 | `gha-workflow-design` | seams (reusable vs composite), triggers/concurrency/`merge_group`, caching, matrix/shards, required-check stubs, permissions + SHA pin; worked example = repo's contract matrix | spike `scripts/contract-all.sh`; local `actionlint` | 2 |

**Two PRs, because skills 9-11 cite files that exist only on the unmerged spike branch** (and some, e.g. `mutants.sh`,
`startup-check.sh`, `probes.rs`, are still untracked there). PR-1 = scaffold + skills 1-8 from `main`. PR-2 = skills 9-11,
drafted now (read-only against the spike worktree) but installed only after spike-0001 merges, with every cited path
re-checked against merged `main`. This also covers the ADR-0006 risk: if the verdict flips to TS core, 9-11 are not shipped.

## Process (existing agents; the engineer never installs)
1. `EnterWorktree` -> `chore/project-skills` (standing authorization; never work in the canonical checkout; do **not** touch
   the spike worktree). Copy this plan to `.agents/plans/project-skills/PLAN.md` (plans record work; tracked).
2. Draft with `skill-engineer`, **in parallel, 4 agents**, output to `.agents/plans/project-skills/artifacts/skills/<name>/`:
   E1 = rust-on-workers + rust-testing - E2 = gha-workflow-design + dockerfile + helm-chart + mdbook -
   E3 = dioxus-07 + dioxus-design-system + blitz-native - E4 = clone-frontend + ts-to-rust (references dioxus skills by name only).
   Each brief must carry: the quality-matrix path; the table row above; "no vendored llms-full.txt, pin Dioxus 0.7.x and note
   0.8-alpha drift"; "tag any command whose tool is not installed `[unverified: <tool> not installed]`" (missing here: zizmor,
   hadolint, cargo-nextest, cargo-mutants, worker-build); "cite ADR ids and enforcement scripts, never restate ADRs"; the
   `find -L` collision check over all three roots; zsh gotcha (`path` is tied to `PATH`); and "read the spike worktree, never write".
3. Gate with `skill-reviewer`, one per skill, in parallel. Section A is absolute. Verdicts saved to
   `.agents/plans/project-skills/reviews/`. Send fixes back to `skill-engineer`; resubmit.
4. Install (PR-1): copy PASSed 1-8 to `.agents/skills/<name>/`; create relative symlink `.claude/skills -> ../.agents/skills`.
5. Commit with Conventional Commits, e.g. `chore(skills): add dioxus skill family`; `Refs:` footer **only if a tracker issue ID exists**
   (open item - do not invent one); end with the `Co-Authored-By: Claude Sonnet 5.5` trailer. Open PR-1; PR body ends with the
   Claude Code attribution line. After it merges, prune the merged worktrees and branches.
6. PR-2 (new worktree, `chore/project-skills-rust-ci`) once spike-0001 is on `main`: re-verify cited paths, re-review changed
   lines, install 9-11.

## User-level checklist (NOT executed by me; not in the repo)
- `! rustup component add rust-analyzer`, then `/plugin install rust-analyzer-lsp@claude-plugins-official` (confirmed present
  locally). Mind the Homebrew-vs-rustup PATH split; Serena's `project.yml` here sets no `languages`, so it is no substitute yet.
- Authenticate the Context7 connector (Dioxus/Blitz skills' preferred doc source).
- Optional: `github-mcp-server` with `--toolsets repos,pull_requests,actions`; pinned `terraform-mcp-server` tag; OpenTofu MCP.
- Third-party skills (`actionbook/rust-skills`, `zizmorify`): run `skill-reviewer` first; never vendored here (AGPL repo,
  supply-chain, licence). Skip: a Kubernetes MCP (one is already installed), lprior/"knowledge patch" Dioxus skills (mixed-version),
  `sirius` dioxus-ui-skill (executes `search.py`; 3 stars).
- Optional tools to make skill commands verifiable: `zizmor`, `hadolint`, `cargo-nextest`, `cargo-mutants`.

## Critical files
- New: `.agents/skills/<name>/{SKILL.md,references/,evals/evals.json}` x8 (PR-1), x3 (PR-2); symlink `.claude/skills`.
- New: `.agents/plans/project-skills/{PLAN.md,reviews/,artifacts/skills/}`.
- Reuse (do not duplicate): the user-level skill quality matrix and the `skill-engineer` / `skill-reviewer` agent specs (not in
  this repo), and a Qdrant plugin's `evals/evals.json` + `scripts/run_eval.py` as the eval-harness reference (run via `uv run`,
  never bare `pip`).
- Read-only sources: spike worktree files listed in the table.

## Verification
1. `readlink .claude/skills` == `../.agents/skills`; `find -L .claude/skills -name SKILL.md | wc -l` == 8 (then 11). **Zero means the
   symlink is broken**, not that nothing is installed. Re-run the three-root collision check against installed skills.
2. Per skill: frontmatter parses (`uv run` one-liner); `SKILL.md` byte size recorded; every `references/` link resolves; no `~`
   paths, no network fetch at run time, `allowed-tools` minimal.
3. `skill-reviewer` PASS for every skill; evals graded per assertion with cited evidence.
4. Run the snippets the local tools allow, in the scratchpad: `mdbook build && mdbook test` on a scratch book; `helm lint` +
   `helm template` on a scratch chart; `actionlint` on the example workflows; `docker build --check` on the example Dockerfile;
   `dx --version` and a `dx translate` smoke test.
5. Trigger smoke test in a fresh session inside the worktree: one positive prompt per skill, plus negatives (a JS-Worker question
   must **not** fire `rust-on-workers`; a Terraform question must not fire `dockerfile`).
6. PR-2 only: script asserting every path cited by skills 9-11 exists on merged `main`.

## Open items / risks
- **Tracker issue ID** for the `Refs:` footer - none known. **SPEC/ADR documents** - not available to the authoring agents; skills cite ADR ids only.
- Report figures (stars, installs, tsc-rs cost) are unverified; nothing here depends on them. Its own unverified list (llms.txt URL,
  `dx new` prompt filenames, whether a Dioxus MCP shipped, Playwright/Chrome DevTools versions) must be verified by the authoring
  agent or tagged.
- Dioxus 0.8 alphas are in flight: skills pin 0.7.x and carry a re-check note. Symlinks need `core.symlinks` for Windows contributors.
- Cost: 4 engineer + 11 reviewer agents. Sizeable but bounded; no Workflow fan-out.
