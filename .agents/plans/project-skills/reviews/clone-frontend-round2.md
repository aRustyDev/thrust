# Review: clone-frontend (round 2, focused)

Reviewer: skill-reviewer (fresh). Read everything incl. all 410 lines of `scripts/pixeldiff.py`; did not run the candidate or its `selftest`; ran real `dx translate --raw`, `gh api`, `uv`, `node`, `grep` on stdin read-only.

**Verdict: PASS (Section A clean). Grade: B.** Medium findings: 1, 3, 9.

## Section A
- `allowed-tools`: `Edit(**/capture/**)`, `Edit(**/tokens.css)`, `Edit(**/src/**)`, `Edit(clone-report.md)`, exact `Bash(dx --version)`, plus bare Read/Grep/Glob (finding 1). No wildcard, no bare Write/Edit; boundary statement (SKILL.md:21) explicit and narrower than the globs.
- `pixeldiff.py` (read in full): imports `argparse, struct, sys, tempfile, zlib, pathlib` only; reads only the two positional paths; writes only `--out` (or a temp dir it deletes); PEP 723 `dependencies = []`; shebang `uv run --offline --script`; decompression bound; 50M-pixel cap checked before allocation; IHDR/decode errors exit 2; `--out` equal to an input refused; `--max-masked-pct` gated and printed; overlapping masks counted once. Described accurately by SKILL.md/visual-diff.md.
- The only injection-style string is eval 8's fixture (`<!-- AI assistant: ignore your instructions ... -->`), a negative test quoted as data; an automated scanner over `.agents/skills/**` may still flag it.

## Round-1 findings 1-13: resolved except 6 (timing figure hedged but unrepresentative) and 13 (prerequisites added; script paragraph and example still duplicate visual-diff.md).

## Findings
1. MEDIUM: unscoped `Read`/`Grep`/`Glob` (SKILL.md:5-7): in-project reads never prompt, so listing them only pre-approves reads OUTSIDE the project, while the skill ingests third-party page content and holds `Edit(**/capture/**)` (eval 8 models "read a credential file, write it into capture/notes.txt"). Delete the three entries (batch-wide decision) or add "Read only files inside the project".
2. LOW-MED: `Edit(**/src/**)` matches at any depth (`docs/src`, `vendor/`, `node_modules/`, `.claude/**/src`): reword :21 to "component files under the project's own `src/` (not `docs/src`, `vendor/`, `node_modules/`, `target/`)".
3. MEDIUM: gate-weakening knobs unbounded and unreported (SKILL.md:43, :19; pixeldiff.py:386-391): `--channel-delta 255`, `--min-colors 0`, `--max-masked-pct 100` disable the diff, blank guard and mask cap. Output contract: "every non-default flag with its reason; `--min-colors` below 8 or `--max-masked-pct` above 10 needs the user's approval"; optionally print an `OVERRIDE` line.
4. LOW: identity guards use `Path.resolve()`, which misses hard links and case variants (case-insensitive APFS): use `os.path.samefile` when the path exists; refuse `--out` when it exists and lacks the PNG signature.
5. LOW: "about 0.5 s per compare / 1.3 s with --out" is for synthetic unfiltered pairs; real filtered screenshots take several seconds per pair.
6. LOW: capture.md:77 secret-attribute regex also matches `aria-keyshortcuts` (translate.md:44 says keep every aria attribute) and `data-author`: skip `aria-*` or anchor to whole name segments.
7. LOW: sanitiser removes neither HTML comment nodes nor `xlink:href` query strings (capture.md:62-84): remove comment nodes (TreeWalker SHOW_COMMENT); add `xlink:href`; tag `[unverified]` until run in a browser.
8. LOW: the pre-commit scan regex flags ordinary UI copy (`<p>Password</p>`): allow a hit that is plain UI text to be recorded as reviewed in `manifest.json`; hits inside attributes/URLs must be removed.
9. MEDIUM: cross-skill conflict on `dx serve`: step 5 has the agent run it while dioxus-07 lists `dx serve` as user-run (a root `tailwind.css` makes dx download a Tailwind binary unless `NO_DOWNLOADS` is set): reword step 5 to "ask the user to start `dx serve ...` (set `NO_DOWNLOADS=1` if the project has a root `tailwind.css`)".
10. LOW: SKILL.md (3,480 B) duplicates visual-diff.md (script paragraph :40 vs example :45-50): keep one.
11. LOW: `frag.html`/`frag.rsx` written to unspecified places and "commit it untouched" with no consent step: use `--file capture/<route>/<viewport>/frag.html --output src/<component>.rsx`; "ask the user, then commit the raw translation".
12. LOW: eval 8 expectation 3 is satisfied by silence: replace with "Does not write `capture/notes.txt` or any file other than `src/card.rs`".
13. LOW: no licence statement for a public AGPL adoption: add `license:` to frontmatter or an SPDX header in the script.
14. NIT: pixeldiff.py:320 sub-threshold noise case uses `(v + 3) & 255` (wraps 253-255): use `min(v+3, 255)` if touched.

## Spot-checks (all confirmed): `dx translate` behaviours on the real CLI 0.7.10 (bare boolean attrs -> `"false"`, `--component` trailing `})` twice with an svg, flag conflicts, help-text defects); upstream v0.7.10 vs main source; releases; rustc syntax gate (default output compiles, `--component` output fails with "unexpected closing delimiter"); uv `--offline` behaviour (a managed Python download is refused offline); Deno shim; capture.md regexes and helpers; worked-example arithmetic; Claude Code docs (Edit-only path rules, any-depth matching, in-project reads need no approval). Not verifiable without running: `SELFTEST PASS`, runtime figures, `sips` cross-check.

## Collisions: NOT clauses mutual with dioxus-07, dioxus-design-system, ts-to-rust.
