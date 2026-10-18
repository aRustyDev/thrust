# Review: clone-frontend (round 4, narrow)

Reviewer: skill-reviewer (fresh). Read every line of SKILL.md, the 4 references, both eval files and all 510 lines of `scripts/pixeldiff.py`; nothing executed. Ran `dx` read-only (`--version`, `translate`, `serve --help`), `strings` on the dx binary, a node test of the sanitiser regex, and the scan pattern over unrelated PNGs.

**Verdict: PASS (Section A clean). Grade: B.** Nothing blocks installation under Section A.

## Section A
- `allowed-tools`: `Edit(**/capture/**)`, `Edit(**/tokens.css)`, `Edit(**/src/**/*.rs)`, `Edit(clone-report.md)`, exact `Bash(dx --version)`; no wildcard, no bare Edit/Write/Bash; `license`, `author`, `version` present.
- Boundary sentence is honest about the depth-agnostic globs (names `docs/src`, `vendor/`, `node_modules/`, `target/`; "do not edit there even though no prompt appears").
- `pixeldiff.py`: imports `argparse, os, struct, sys, tempfile, zlib, pathlib` only; no subprocess/socket/urllib/eval/exec/pickle/ctypes/environ; reads the two positional paths and the first 8 bytes of an existing `--out`; writes only `--out` (and a temp dir in `selftest`); `samefile` guards; non-PNG `--out` refused; decompression bound; pixel cap checked before allocation; the four approval tiers print "needs the user's approval" exactly as documented; NaN float flags fail the gate.
- The only injection-shaped string is eval 8's quoted fixture; an automated scanner may flag it.
- `rm -rf "$tmp"` in translate.md is safe (quoted, `mktemp`-created only).

## Round-3 findings 1-9: all resolved or mostly resolved (details in the round-3 record).

## New findings (none blocking)
1. LOW-MED: the rustc syntax gate is vacuous on a missing or empty input (translate.md:65-75): guard with `[ -s file ] &&` and `[ -d "$tmp" ] &&`; a `SYNTAX OK` with a `cat:` error is void.
2. LOW: the secret scan lists compressed PNGs (measured: 8 of 150 unrelated PNGs hit on `pwd`/`jwt`; 0 with `-I`): use `grep -rIliE` or exclude `*.png`; `manifest.json` lists itself once `scan_reviewed` quotes a word.
3. LOW: the pixel cap bounds area, not row count (a 1x50,000,000 PNG passes): also reject `max(w, h)` above about 20,000 and add a selftest.
4. LOW: the Reads line contradicts steps 2 and 5 (a user-named source path may be outside the project).
5. LOW: the token exit check scans `src` only and reads a missing `src` as "all defined".
6. LOW/NIT: the sanitiser keeps `data-accesstoken`, `data-secretkey`, `data-clientsecret`, `data-idtoken`, `data-sessiontoken`, `data-privatekey` (the scan catches them); `noQ` keeps `#fragment`.
7. NIT: `scripts/pixeldiff.py` is relative to the skill directory, not the project.
8. NIT: three of the four named sibling skills are not installed in the target repo: degrade gracefully.
9. NIT: "never capture production" leaves no route for a public page that needs no sign-in.
10. NIT: naming (`<route>` vs `<route-slug>`; one `frag.html` per route x viewport conflicts with several fragments; "dioxus-components" reads like a skill name).
11. NIT: `os.path.exists` is false for a dangling symlink (`--out` through one would create its target); the docstring omits the 8-byte read; add an SPDX header.

## Spot-checks (dioxus 0.7.10): `dx serve --help` confirms `--web`, `--port`, `--open [true|false]`; `dx translate --help` and behaviours (`disabled: "false"`, `"onclick": "go()"`, comment dropped, `--component` ends `})` twice) confirmed; `NO_DOWNLOADS` string present in the binary. Not verifiable without executing: `SELFTEST PASS`, timings, `sips`.

## Collisions: none; mutual NOT clauses consistent; 7 should-trigger, 9 should-not-trigger, 12 evals.
