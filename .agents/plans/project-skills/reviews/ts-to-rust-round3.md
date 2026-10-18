# Review: ts-to-rust (round 3, narrow)

Reviewer: skill-reviewer (fresh). Read SKILL.md, references, evals and all 320 lines of `scripts/oracle_diff.py`; hand-walked all 41 selftest cases; nothing executed.

**Verdict: PASS (Section A clean). Grade: B.** Recommended before install: findings 1 and 2.

## Section A
- `allowed-tools`: five `Edit(...)` patterns only (no Read/Grep/Glob/Bash/network); boundary statement includes the read restriction, data-not-instructions, ask-the-user-for-Cargo.toml and per-call approval for cargo / TS runner / script runs.
- `oracle_diff.py`: imports `argparse, json, re, sys, pathlib.Path`; no subprocess/network/eval/exec/pickle/ctypes/os; reads only `--rules`, `--exceptions` and `*.json` in the two directories passed; never opens a file for writing; `dependencies = []`; mode 755; ASCII only; `?`, `[`, `]` rejected in rules and exceptions; wildcard-only case/path rejected; `#status` reserved; `--max-normalised-pct` clamped and NaN rejected; unused rules WARN; shapes validated; exit codes 0/1/2.
- Only embedded imperative is eval 7's quoted bait (expectations require ignoring it). No home paths or usernames.

## Round-2 findings 1-12: resolved, except 4 (one stale remainder) and 9 (partly: narrative about unshipped crates remains, claims reproduced).

## New findings
1. MEDIUM: ReDoS in the `*`-only matcher (oracle_diff.py:39-41, used at :88 and :157): a pattern with k stars costs roughly O(n^k) on a failing subject (`'*a'*10+'*b'` against a 51-char path did not finish in 20 s); nothing caps the star count; local DoS from a hostile or careless rules/exceptions file. Fix: a leftmost-greedy segment matcher (split on `*`; anchor first and last parts; `find` each middle part in order), differential-tested against the regex version (0 mismatches on patterns <= 6 over {a,b,*}); or reject more than ~4 `*` per pattern with exit 2; add a selftest and a docstring note.
2. MEDIUM: SKILL.md:19 says Edit is "granted only to" `src/` ".rs files ... (not `docs/src`, `vendor/`, `node_modules/`, `target/`)", but `**/src/**/*.rs` matches at any depth (including `docs/src`, `vendor/*/src`, `node_modules/*/src`, `target/**/src`, `.claude/worktrees/*/src`); `fixtures/*.json` is actually `**/fixtures/**/*.json`; `tests/oracle/**` matches any file type. Reword (depth-agnostic path patterns; explicit "do not edit under ... even though no prompt appears") or anchor the patterns (`Edit(src/**/*.rs)` etc.). The same sentence appears in clone-frontend: fix once for the batch.
3. LOW-MED: the grants pre-approve editing the frozen data the skill forbids editing (`Edit(**/fixtures/**/*.json)`, `Edit(**/tests/oracle/**)` incl. `recorded/`); fixtures/recordings are produced through Bash, which prompts: drop the fixtures grant and narrow `tests/oracle/**` to `rules.json`, `exceptions.json`, `replay/**`.
4. LOW: an exception at path `/` can never be registered (a top-level array length or bare-scalar body difference is reported at `/`, and `_wild_only("/")` is true): special-case `/` or document the limit; add a selftest.
5. LOW: worked-example.md:143 says "not frozen, not ported" for array/object priority but `priority_array` is frozen; the ledger gate cell lists only "clippy --lib clean" instead of all three parts of anti-patterns.md:73.
6. LOW: verification tags: json-fidelity.md:3-6 lists thiserror and futures that the page never uses; type-map.md:4 omits serde_json; `[verified: ... BSD grep]` lacks a version (freeze-and-oracle.md:88, anti-patterns.md:45).
7. LOW: path base ambiguous: `scripts/oracle_diff.py` is relative to the skill directory while `tests/oracle/...` is relative to the project: say so once (SKILL.md:40; freeze-and-oracle.md:129, 152).
8. LOW optional: qualify the bare Rust error-string triggers with "when porting from TypeScript" in the description.

## Spot-checks: serde_derive `default` handling for `Option<Option<T>>`; JS number/rounding/bitwise/sort/key-order/JSON.stringify behaviours; Rust `as` saturation and rounding; UTF-8/UTF-16 string facts; serde_json 1.0.143 (ryu) vs 1.0.154 (zmij) float text; chrono `AutoSi`; `async_fn_in_trait`; the `a.as_ref().and_then(...)` form compiled clean under `-D warnings` and clippy; the broken form gives E0515; the credential-scan regex matched all 7 planted categories. Not reproduced: the worked-example build, the 7 mutants, serde untagged / `deny_unknown_fields` / uuid text behaviour.

## Collisions: none installed; clone-frontend reciprocal; `rust-testing` and `rust-on-workers` do not appear anywhere in the candidate.
