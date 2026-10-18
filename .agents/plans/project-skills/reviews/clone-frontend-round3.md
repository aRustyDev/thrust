# Review: clone-frontend (round 3, narrow)

Reviewer: skill-reviewer (fresh). Read every line of SKILL.md, 4 references, both eval files and all 489 lines of `scripts/pixeldiff.py`; nothing executed.

**Verdict: PASS (Section A clean). Grade: B.** Nothing blocks installation under Section A.

## Section A (on what changed)
- `allowed-tools`: `Edit(**/capture/**)`, `Edit(**/tokens.css)`, `Edit(**/src/**/*.rs)`, `Edit(clone-report.md)`, exact `Bash(dx --version)`; `license: AGPL-3.0-only`; no wildcard, no bare Edit/Write/Bash, no Read/Grep/Glob. Boundary statement (SKILL.md:19) matches every grant and includes the read restriction.
- `pixeldiff.py`: imports `argparse, os, struct, sys, tempfile, zlib, pathlib`; no subprocess/socket/urllib/eval/exec/pickle/ctypes/environ; PEP 723 `dependencies = []`; reads the two positional paths and the first 8 bytes of an existing `--out`; writes `--out` and a temp dir it deletes; `os.path.samefile` guards (hard links, case variants); refuses non-PNG `--out`; decompression bound; pixel cap before allocation; OVERRIDE lines on five triggers; `--max-masked-pct` fails the verdict; SKILL.md/references describe it accurately.
- No network, credential paths or reviewer-addressed text (eval 8's fixture is a quoted negative test; an automated scanner may flag it).

## Round-2 findings 1-14: resolved except 11 (partly: ordering contradiction + missing frag files in the bundle layout) and 12 (partly: eval 8 contradicts the skill's own flow).

## Findings
1. MEDIUM: check-in sequence contradicts between SKILL.md:27 ("ask the user to check in the raw translation untouched, then wrap it into `src/<component>.rs`") and translate.md:81-82 (wrap, get it compiling, then check in that state alone); capture/ may be git-ignored. Make SKILL.md:27 match translate.md.
2. LOW-MED: scratch files written outside the grant against "touch nothing else": capture.md:162-164 writes `used.txt`/`defined.txt` into the project root; translate.md:64-70 creates `check.rs` and leaves a `.rmeta` in cwd. Use process substitution for the comm pipeline; feed the stub to `rustc` on stdin with `--out-dir <temp dir>` (`-o /dev/null` does not work with `--emit=metadata`).
3. LOW-MED: approval rule asymmetric: `--channel-delta 255` and a very large `--tolerance-pct` disable the comparison as completely as `--min-colors 0`/`--max-masked-pct 100` but need only a reason: add "`--channel-delta` above 32 or `--tolerance-pct` above 5 also needs the user's approval"; update `overrides()` strings, selftest, SKILL.md:34, visual-diff.md:64.
4. LOW: sanitiser secret-attribute list narrower than the scan list (capture.md:72 vs :177): `data-password`, `data-authorization`, `data-apikey`, `data-sessionid`, `data-signature`, `data-xsrf`, `data-cookie`, `data-credentials`, `data-bearer`, `data-jwt` pass the sanitiser; extend the whole-segment list and the concatenated forms; add the same names to the scan; keep the `aria-` exemption.
5. LOW: read boundary conflicts with reading the reference implementation (capture.md:129-130, 96-97): "if the original's source is outside the project, ask the user to name the path first".
6. LOW: evals 8 and 9 muddy: eval 8 should allow `capture/**/frag.*`; eval 9 should say what the agent does (does not present the forced pass as clean; runs only with the flags recorded as OVERRIDE).
7. LOW: `dx serve --web --port N --open false` spelling unsourced (SKILL.md:29): verify against `dx serve --help` or drop `--open false`.
8. LOW: `frag.html` and `frag.rsx` missing from the bundle layout (capture.md:45-55; translate.md:78-80).
9. NITs: exclusions in prose only for the `.rs` clause; SKILL.md:36-39 example omits two `hot_cell` lines; SKILL.md:19 is one 790-char line; driver prerequisites omit device scale factor and wait-for-fonts/network-idle; `MAX_PIXELS` 50M is a sanity cap; step 4 delegates to dioxus-design-system: add "that skill's approval flow governs"; the flags-needing-a-reason parenthetical omits `--min-colors`/`--max-masked-pct`.

## Spot-checks: `dx translate` behaviours confirmed again on the real CLI 0.7.10 (bare boolean attributes, `--component` trailing `})`, sorting, key mapping); tool versions match; capture.md helpers, comm pipeline and scan reproduced. Not verifiable without running: `SELFTEST PASS`, timings, `sips`, `--open false`.

## Collisions: none installed; mutual NOT clauses with dioxus-07, dioxus-design-system, ts-to-rust (ts-to-rust must ship in the same batch or the NOT clause dangles).
