# Review: mdbook (round 2, focused)

Reviewer: skill-reviewer (fresh). Read every file incl. `references/examples/docs.yml`; verification ran on scratch material only (mdbook 0.5.4, cargo, actionlint 1.7.12 with shellcheck, `git ls-remote`, `gh api`, crates.io).

**Verdict: PASS (Section A clean). Grade: B.** No finding blocks adoption.

## Section A
- `allowed-tools`: `Edit(docs/**)` and exact `Bash(mdbook --version)` only; the inert `Write(...)` lines, the workflow-file Edit and the bare Read/Grep/Glob were removed. Claude Code consults only `Edit(path)` rules, which cover all file-editing tools.
- `docs.yml` as executable content: `pull_request` trigger (no `pull_request_target`); top-level `permissions: {}`; `build` has `contents: read`; `deploy` only `pages: write` + `id-token: write` and checks out nothing; no `${{ }}` inside any `run:` body; `persist-credentials: false`; publishing gated on `push` to `main`; mdBook installed with `cargo install --locked --version =0.5.4` each run; all six actions pinned by 40-hex SHA that resolve to their tags (checked via `git ls-remote`); actionlint clean, and a mutated copy fails with 4 errors.
- No credential access, remote fetch-and-execute, or reviewer-addressed text; no sibling skill names; no thrust-only paths.

## Round-1 findings 1-12: all resolved (orphan check `./` fix reproduced on a fresh `mdbook init` book; nested, include-only and injected-orphan cases behave as documented).

## Follow-ups (all applied after this review)
1. LOW: caching claim stronger than evidence (mdBook is uncached only because `cargo install` runs before rust-cache): add `cache-bin: "false"`, reword the comment, change "exact and reproducible" to "mdBook version is exact; the Rust toolchain floats".
2. LOW: template assumptions unstated: adjust `paths:` to every directory the book includes (for example `examples/**`); `--locked` needs a committed `Cargo.lock`.
3. LOW: scope statement incomplete: only `mdbook --version` runs without a prompt; the book root is assumed to be `docs/`.
4. INFO: fixed `/tmp/listed.txt` paths in the orphan check: use cwd files and remove them.
5. LOW: no eval for the user's-decision rule: added eval 7 ("set up mermaid").
6. INFO: `author`/`version` are not recognised Claude Code keys (kept as provenance; `license` added at install).
7. INFO: `workflow_dispatch` builds and tests but never publishes: documented in ci-and-pages.md.

## Spot-checks (all reproduced): SHA pins 6/6; actionlint with a discriminating control; orphan check cases; silent-failure claims (broken include and mistyped `[output.html]` key exit 0); default 2015 edition (`async fn` fails with E0670, `dyn` compiles); crates.io facts for admonish, mermaid, linkcheck, linkcheck2; the workspace `-L` flow; the `#[cfg(doctest)]` route.

## Collisions: none (diagram-design/show-me mention Mermaid, no mdBook overlap).
