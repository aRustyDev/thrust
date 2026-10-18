# Review: rust-on-workers (round 1)

Reviewer: skill-reviewer. Draft read in full (SKILL.md, 5 references, 2 eval files). Spike HEAD at review: 620e2a1.

**Verdict: REJECT. Grade: UNGRADEABLE (Section A failed).**

## Section A (blocking)
- A2.1 overly broad tool permissions: `Bash(scripts/rs:*)` (SKILL.md:16). `scripts/rs` is an exec trampoline (last line `exec "$@"`), so the prefix rule pre-approves any command. Confirmed by the orchestrator against the spike's `scripts/rs`.
  Fix: command-specific rules (`scripts/check-deps.sh`, `scripts/rs cargo check:*`, `scripts/rs cargo tree:*`, `scripts/rs worker-build:*`) or drop Bash.
- Everything else in A passed (credentials, network, RCE, injection text, attribution, frontmatter key).

## Section B/C (non-blocking, to fix in the same pass)
1. Trigger precision: compat-date error string is language-agnostic (fires for JS Workers); Dioxus exclusion suppresses the skill where the toolchain fix applies. Qualify "in a Rust (worker-build) Worker"; add a JS should_not_trigger case; decide the Dioxus-toolchain question explicitly.
2. SKILL.md:65 dangling "four checks": enumerate or say "the checks above".
3. SOURCES.md will not survive install: drafting-process language, WebFetch spend-limit leak, stale "Status at drafting" column, inline "untracked at drafting time" phrases (build-and-features.md:46, dev-loop-and-toolchain.md:100).
4. ADR list in SOURCES.md does not match ids cited in the text; bindings.md:56 restates ADR rationale.
5. Overclaims: "every rule has a measured failure behind it"; Workflow-class statement contradicts itself between SKILL.md:50/bindings.md:76 and bindings.md:85-86.
6. "this setup" -> "on macOS where Homebrew's rustc shadows rustup".
7. `Bash(scripts/check-deps.sh:*)` -> exact form.
8. Load cost: SKILL.md 3,731 B vs 1.1-1.5 KB benchmark; rules 4 and 6 duplicate references.
9. No worked final-answer sample; prerequisites not stated up front.
10. Eval 4 hard-codes the measured compat date for wrangler 4.123.0; mark version-pinned.

## Factual spot-checks: 15 made against the spike, all held (one trivial omission: `hyper-util` in a wasm list).
Not independently re-run: wasm32 panic strategy, Homebrew sysroot lacking wasm32 std, dyld fault, workerd date accept/reject, sqlx failure, externref failure (author-tagged `[re-run]`/`[spike]`).
