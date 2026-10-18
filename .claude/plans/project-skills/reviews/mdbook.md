# Review: mdbook (round 1)

Reviewer: skill-reviewer. Read SKILL.md, 5 references, `references/examples/docs.yml`, both eval files; ran only mdbook 0.5.4, rustc/cargo 1.98.1 and actionlint 1.7.12 on scratch material.

**Verdict: PASS (Section A clean). Grade: B.** Reviewer: F1 and F2 should block adoption (caller's bar).

## Section A
- `allowed-tools`: Read, Grep, Glob, `Edit(docs/**)`, `Write(docs/**)`, `Edit(.github/workflows/docs.yml)`, `Write(.github/workflows/docs.yml)`, exact `Bash(mdbook --version)`. No wildcard Bash; SKILL.md states it authors files. `Write(path)` entries are inert (Claude Code consults only `Edit(path)`/`Read(path)` rules; `Edit` rules cover all file-editing tools).
- `docs.yml` as executable content: all six actions pinned by 40-hex SHA resolving to the commented tags (rust-toolchain `v1` is a moving tag); `permissions: {}` top level, least privilege per job; `pull_request` trigger only; no `${{ }}` inside `run:`; no `curl | sh`; one `cargo install mdbook --locked --version =0.5.4`; actionlint clean and a mutated copy fails with the expected errors.
- No credential access; `persist-credentials: false`; no preprocessor installs pre-approved.

## Findings
1. MED-HIGH: orphan-chapter check false-positives on `./`-prefixed SUMMARY links (the form `mdbook init` writes); the example workflow would exit 1 on a clean book. Fix `sed -E 's/^\]\((\.\/)?//'` in book-structure-and-config.md:37-44 and docs.yml:75; add a `./` case to the verification note; note that a `.md` reached only via `{{#include}}` is reported as an orphan (fails safe).
2. MEDIUM: delete the two inert `Write(...)` lines (SKILL.md:9, 11): startup warning on Claude Code >= 2.1.210 and misleading.
3. LOW-MED: `Edit(.github/workflows/docs.yml)` removes the prompt on a workflow with OIDC/Pages-deploy authority; recommend dropping it.
4. LOW: bare Read/Grep/Glob add nothing inside the working directory (optional).
5. LOW-MED: scope statement ("docs/ and docs.yml only") contradicts steps requiring `// ANCHOR:` comments in `examples/*.rs` and a `#[cfg(doctest)]` item in `src/lib.rs`; add "Rust sources outside docs/ are edited only with the user's approval".
6. LOW: "default 2015 edition: `async`/`dyn` fail" — `dyn` compiles; remove it; step 1 hard-codes `edition = "2021"` (say "match the linked crate's edition").
7. LOW: firm up preprocessor caveats from upstream manifests (admonish 1.20.0 -> mdbook 0.4.40; linkcheck 0.7.7 -> mdbook 0.4; linkcheck2 0.13.0 and mermaid 0.17.1 target 0.5); crates.io lists both linkcheck crates as MIT.
8. LOW: ci-and-pages.md:33-39 "required check is path-filtered" duplicates gha-workflow-design; replace with a pointer.
9. LOW (zizmor not installed): the publishing job restores `~/.cargo/bin/mdbook` from `actions/cache` and executes it; consider dropping the binary cache in the publishing job.
10. INFO: `author`/`version` are not recognised Claude Code keys (ignored); the matrix requires them as provenance. Decide once across all drafts (kept as is).
11. LOW: add "Installing a preprocessor or running `cargo install` is the user's decision; propose it and wait."
12. LOW: eval 4 expectation about `[unverified]` tags should be conditional.

## Spot-checks: 14 confirmed (broken include exits 0; mistyped `[output.html]` key exits 0; orphan chapter omitted silently; native `> [!NOTE]`; `mdbook test -L` fixes E0432/E0464; missing preprocessor/`create-missing`; 0.5 change list; linkcheck vs linkcheck2; mermaid/admonish facts; `#[cfg(doctest)]` include; anchors; docs.yml lint with must-fail control; pins). Partly: default 2015 edition (`dyn` compiles).

## Collisions: none installed. Sibling `gha-workflow-design` is the real neighbour; recommend adding "mdBook docs builds/Pages deploys (see mdbook)" to its NOT-for line. No dangling branch-name reference found (the "spike-0001" mention the engineer reported is absent from shipped files).
