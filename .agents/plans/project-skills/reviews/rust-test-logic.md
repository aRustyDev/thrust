# Review: rust-test-logic (round 1)

Reviewer: skill-reviewer (fresh). Report transcribed by the drafting session's lead. Read every file in the draft, the
four sibling drafts for boundaries, and `ts-to-rust` for duplication. Spot-checks ran against the committed verification
pack (`.agents/plans/project-skills/verification/rust-testing-2026-10-18/`).

**Verdict: PASS (Section A clean). Grade: B.** Nothing blocked adoption; findings 1 to 4 were recommended before install.

## Section A
Frontmatter is exactly name/description/license/author/version; no `allowed-tools`, so nothing is pre-granted; no network use,
credential paths, fetched-content execution or reviewer-addressed text; the only "install" mentions are "user's decision:
propose and wait"; trigger breadth acceptable.

## Findings and disposition
1. MEDIUM: "commit the regressions file" is wrong for a CI failure (the file died with the runner); proptest prints the `cc`
   line to copy. **Fixed** (`property.md`, `model-based.md`, SKILL.md example, eval 1).
2. MEDIUM: `parity-harness.md` vs `ts-to-rust`: two recorders, two fixture shapes, two error policies; "keeps only what
   ts-to-rust lacks" was not true. **Fixed**: "Whose format wins" (ts-to-rust owns freezing for any port it drives); the harness
   and generator are scoped to scalar units; the `fixtures/intended/` advice, the pinned-count control, environment recording
   and the `Math.round` snippet were removed as duplicates.
3. MEDIUM: collisions with `rust-test-app` (insta, rstest with axum, three copied axum snippets). **Fixed**: non-axum
   examples, insta scoped to plain values, JSON and text, negative triggers and evals added.
4. MEDIUM: SOURCES.md said "not copied: the prose is new". **Fixed**: "adapted", verbatim passages and sample-file edits listed,
   origin line in each example, licence basis recorded (maintainer confirmation, 2026-10-18).
5. LOW: `nextest.toml` comment and eval 3 omitted `--flaky-result pass`. **Fixed**.
6. LOW: `oracle_` adapters should wrap pure functions or stub I/O; the no-Node control is untagged. **Fixed** (`[derived]`).
7. LOW: description lacked `--retries`/flaky/cargo-test-to-nextest vocabulary; no `[profile.mutants]` negative; evals missing
   for "regenerate fixtures to go green" and `oracle_` without Node. **Fixed** (evals 15 to 18, triggers).
8. LOW: "larger neighbour" is by magnitude; CI-guard grep scope unspecified. **Fixed**.
9. INFO: SKILL.md headroom is small (now 4082 of 4096 bytes); fixes live in `references/`; the evals were not run.

## Collisions
`ts-to-rust` (parity and replay: finding 2); `rust-test-app` (insta, rstest with axum: finding 3); `rust-test-mutation` (only the
nextest `mutants` profile). No other installed skill collides.

## Spot-checks: 14 `[re-run]` claims, all held
Exit 100 on failure; a `prop_` test fails under `--retries` and `NEXTEST_RETRIES`; `--flaky-result pass` and the env var override
the per-test config; default persistence warns twice and still saves, and `WithSource` writes the same file;
`PROPTEST_DISABLE_FAILURE_PERSISTENCE` is honoured by the default config; an empty filter selection exits 4; `2 divergence(s)`;
fixtures regenerate byte-identically; a broken legacy function fails `oracle_` and shrinks to n=2; `deep` at 4096 passes; the
`Math.round` and `toFixed` tables; 256 oracle cases in about 0.1 s; all six profiles list. The reviewer also checked the rstest
`#[once]`/`#[files]` docs, proptest `Config` and the datatest-stable no-match behaviour against registry source.

## Not verified
`cargo insta review`; behaviour on nextest 0.9.148 (the pack ran 0.9.140); `deep` inheriting the default overrides (docs only);
the evals themselves (never run).

## After the review
The fixes above were applied after this review and were **not re-reviewed by `skill-reviewer`**. The lead re-ran the mechanical
gate (frontmatter, size, links, forbidden strings, eval and trigger counts) and confirmed each fix by reading the changed text.
