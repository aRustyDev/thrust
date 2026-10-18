# Review: ts-to-rust (round 4, narrow)

Reviewer: skill-reviewer (fresh). Read every file incl. all 374 lines of `scripts/oracle_diff.py`; nothing executed or imported. The matcher test ran on the reviewer's own transcription of the pure function.

**Verdict: PASS (Section A clean). Grade: B.** Nothing blocks install; findings 1 and 2 recommended before it.

## Section A
- `allowed-tools`: exactly six `Edit(...)` entries (`**/src/**/*.rs`, `**/tests/**/*.rs`, `**/tests/oracle/rules.json`, `**/tests/oracle/exceptions.json`, `**/tests/oracle/replay/**`, `**/PORT-LEDGER.md`); no Read/Bash/Write/WebFetch/wildcard-all; no fixtures grant. The relative `**` patterns are anchored at the working directory and match at any depth within the project (they do not reach `~/.cargo/registry/src`); the boundary sentence says so and names `docs/src`, `vendor/`, `node_modules/`, `target/`.
- `oracle_diff.py`: imports `argparse, json, sys, pathlib.Path` only (no `re`, `os`, `subprocess`, sockets, `open()`, `exec`/`eval`); reads only `--rules`, `--exceptions` and `*.json` in the two directories given; never writes; `dependencies = []`; errors print case names and decode positions, not file content.
- The only embedded imperative is eval 7's quoted bait (expectation requires ignoring it).
- Provenance: no credential, `~` or home-path access; `clone-frontend` is the only sibling named; `rust-testing` absent.

## Matcher (oracle_diff.py:41-60)
Leftmost-greedy segment matcher: split on `*`, first part anchors as prefix, last as suffix, overlap rejected, middle parts located in order with `s.find(mid, pos, end)`. Correct and bounded by pattern length x subject length. Independent test: 0 mismatches against `re.fullmatch` over all 1,676,080 pairs (patterns <= 7 over {a,b,*} vs subjects <= 8 over {a,b}) and against an independent DP matcher over 300,000 random pairs. Measured: 4000-star pattern vs 10,000 subject chars in 35 microseconds. All 61 selftest cases hand-walked.

## Round-3 findings 1-8: all resolved (finding 3 partly: the `replay/` grant remains, see below).

## Findings (none Section A)
1. LOW-MED: the read restriction contradicts the skill's own install (`references/*.md` live under `.agents/skills/ts-to-rust/`): "Read only files inside the project and this skill's own directory; never `~`, other dotfiles, `.env`, keys or `.git`."
2. LOW-MED: `Edit(**/tests/oracle/replay/**)` is the only evidence-bearing directory that is hand-editable without a prompt and no control can detect a forged replay; SKILL.md omits `replay/` from the never-hand-edit list, and eval 7 expectation 4 lists "replay files" among things the agent hand-edits. Drop the grant and update SKILL.md, freeze-and-oracle.md and the eval, or keep it with "replay files hold the port's verbatim responses; never adjust them to match".
3. LOW: "scrub recordings first" gives no remedy on a scan hit while `recorded/` is not granted: "on a hit, fix the recorder and re-record; the user may scrub by hand; do not edit in place".
4. LOW: "touch nothing else" does not mention the TS freeze script (writing it prompts, or the user writes it).
5. NIT: anti-patterns.md:70-72 "the gate counts `todo!`" gives no command: add `grep -rn 'todo!' src`.
6. NIT: oracle_diff.py:33 "in the same run" is wrong (`selftest` is a separate invocation): "in the same session or CI job".
7. NIT: type-map.md:3-5 tag attaches `futures` to snippets that do not use it.
8. NIT: oracle_diff.py:307-308 the many-star selftest returns at the suffix test and never exercises the middle-search loop: use `"*a"*40 + "*c*b"` against `"a"*199+"b"`.
9. NIT: oracle_diff.py:100-111 `normalise` treats the root path as `""` but diffs are reported at `"/"`: a rule cannot target a bare-scalar body: document or use `/` in both.
10. LOW optional: add the upstream copyright line to SOURCES.md if any decision-table phrasing is verbatim.

## Spot-checks (all confirmed): serde `default`/`deserialize_with`/plain `Option` behaviour from registry source; chrono `AutoSi`; serde_json 1.0.143 (ryu) vs 1.0.154 (zmij) float printer; JS number/rounding/bitwise/sort/key-order/JSON.stringify/Unicode/date behaviours (node 26.9.0); Python `json.dumps` NaN; Rust `as` saturation, rounding, UTF-8/UTF-16, `trim`; `a.as_ref().and_then(...)` compiles and the by-value form gives E0515; BSD grep 2.6.0-FreeBSD scan. Not reproduced: the worked-example build, the 7 mutants, the clippy count of 13, running both serde_json versions, `deny_unknown_fields` text, the uuid lenient forms.

## Collisions: none; `clone-frontend` not yet installed (reciprocal NOT clause).
