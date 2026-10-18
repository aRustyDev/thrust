# Review: rust-on-workers (round 2)

Reviewer: skill-reviewer (fresh). Draft read in full; spike read-only at HEAD 620e2a1.

**Verdict: PASS. Grade: B.** Section A clean.

## Section A
- Round-1 blocker gone. `allowed-tools`: Read, Grep, Glob, exact `Bash(scripts/check-deps.sh)`, exact `Bash(scripts/rs cargo check -p thrust-worker --no-default-features --target wasm32-unknown-unknown)`. No wildcard remains. (The two cargo/rs wildcard forms the first fix round produced were tightened to the single exact literal by the orchestrator before this review.)
- Residual (informational): `cargo check` runs dependency build scripts; path-based allow-lists approve a repo-mutable script path.

## Round-1 findings 1-10: all resolved except 8 (load cost partly: body 3,418 B vs 1.1-1.5 KB benchmark; rule 6 still repeats bindings.md; within the plan's 4 KB target) and the trivial `hyper-util` omission.

## Follow-ups (do at PR-2 install time, after the spike merges)
MANDATORY
- Re-stamp provenance: SOURCES.md (lines ~5-6, 17, 27) and build-and-features.md:45 say "verified at 620e2a1", but `probes/sqlx-wasm/*` and a `scripts/contract.sh` change exist only as staged index state, and a squash merge drops 620e2a1. Re-verify every cited path against merged `main`, replace "spike tree"/branch name/SHA with "the repo at <merged sha>".
- ADR location: no ADR files exist on `main` or in the spike's committed tree; a public reader cannot resolve ADR-0002 etc. Add the ADR location to SOURCES.md or confirm the docs land with/before adoption.
RECOMMENDED
- dev-loop-and-toolchain.md:17-18: "Neither occurs when rustc on PATH is rustup's own" is ambiguous and contradicts line 32; rustup's proxies set DYLD_FALLBACK_LIBRARY_PATH themselves, a bare `<toolchain>/bin` on PATH fixes Fault 1 only. Tag [derived].
- build-and-features.md:45: soften "sqlx does not build for wasm32" to the measured configuration (sqlx 0.8.6, postgres, no runtime feature; needed a getrandom `js` workaround to reach the `home` failure).
- build-and-features.md:10, 15-16: add `hyper-util` to the wasm32 list; "all targets" applies to bullet 1 only.
- SKILL.md:24: macOS is needed only for the two toolchain faults.
- Optional: `--locked` on the cargo literal and the Verify command.
- Trivial: sample report order vs Verify order; unused `[unverified: <tool> not installed]` legend entry.

## Spot-checks: 17 made, all held (scripts/rs, check-deps.sh lists, release profile, strip=true mechanism confirmed in wasm-bindgen-cli-support 0.2.129 source, panic-strategy abort, Homebrew sysroot lacks wasm32, send-shape matrix 6/6, worker 0.8.7 SendFuture, wrangler compat-date comment, entry.mjs, feature claims, worker-root behaviours).
Not independently re-run: workerd accept/reject boundary, rust-lld dyld fault, worker-build exit codes, ~1-in-4 failure rate, workerd panic blast radius.
