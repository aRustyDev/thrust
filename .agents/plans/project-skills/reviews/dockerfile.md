# Review: dockerfile (round 1)

Reviewer: skill-reviewer. Read every file incl. examples; built and ran nothing from the candidate.

**Verdict: PASS (Section A clean). Grade: B.** Reviewer: treat findings 1-5 as the adoption bar.

## Section A
- `allowed-tools`: Read, Grep, Glob, `Edit(Dockerfile*)`, `Edit(.dockerignore)`, `Edit(docker/**)` (+ three inert `Write(...)` lines). No Bash, no wildcards. SKILL.md:21-22 states it authors `Dockerfile*`, `.dockerignore`, `docker/**` only (policy met).
- Examples: base-image digests verified live against the registries (`gcr.io/distroless/cc-debian13:nonroot` -> sha256:e792ab3d..c1c2 matches; `lukemathwalker/cargo-chef:latest-rust-1.99-slim-trixie` -> sha256:b22df2bf..ba99 matches; both index/created as claimed); non-root USER 65532 matches the image config; no ARG/secret ENV; exec-form HEALTHCHECK; `.dockerignore` excludes `.env*`/`.dev.vars*`; `healthcheck.rs` std-only, connects only to the fixed argv address, has timeouts.
- No network instruction, credential access, remote execution or injection text.

## Findings
1. MEDIUM: delete the three inert `Write(...)` lines (SKILL.md:9, 11, 13); optionally state that Edit rules cover Write.
2. HIGH (sequencing): the worked example and evals 1, 2, 4 cite `crates/thrust-server/src/main.rs`, `crates/thrust-api/src/lib.rs`, the postgres adapter and the workspace `Cargo.toml`, which exist only on the unpushed spike branch (public `main` holds only .gitignore, LICENSE, README.md). Land the skill with or after the crates (or add a README line saying the example targets a crate not yet merged). A commit SHA would dangle too.
3. MEDIUM: eval 4 has a false premise: `thrust-server` -> `thrust-adapters-selfhosted` -> `thrust-adapters-postgres` with `features = ["native"]`, so feature unification already enables `native` and `cargo build -p thrust-server -p thrust-adapters-postgres` already builds `migrate`; the "cannot stat migrate" scenario only reproduces with `-p thrust-adapters-postgres` alone. `--features thrust-adapters-postgres/native` is redundant but defensible. Rewrite eval 4; add the discriminating row to the verified table.
4. MEDIUM: a skill with no Bash cannot resolve digests: add "never copy digests from the example and never invent one; ask the user to run `docker buildx imagetools inspect <ref>` and paste the index digest"; date the Dockerfile header ("resolved 2026-10-18, refresh before use").
5. MEDIUM: output-contract tag `[unverified: <tool> not installed]` asserts an environment fact the agent cannot check; use `[unverified: not run]` (also pinning-and-lint.md:4,104-106; runtime-and-health.md:24,30).
6. LOW: `# syntax=docker/dockerfile:1` is an unpinned frontend, contradicting the pinning thesis: pin by digest or state the exception.
7. LOW: runtime-and-health.md:43: `THRUST_TEST_ROUTES` gates `/internal/*` behind the admin token but `/_info` and `/_test/panic` are ungated; fix the parenthetical.
8. LOW: Kubernetes stop-signal wording (the kubelet asks the runtime; containerd/CRI-O default to SIGTERM); SKILL.md:36 hardcodes "handles only SIGINT": add "re-read main.rs; this will change".
9. LOW: process narrative and author-environment state in reference prose (pinning-and-lint.md:3-4, 60-64; multistage-and-caching.md:3-5; runtime-and-health.md:99-100); keep verification status only in SOURCES.md.
10. LOW: Dockerfile:35 vs :43 on whether the `probe` stage is optional; SKILL.md step 5 never says to copy `healthcheck.rs` to `docker/healthcheck.rs`.
11. LOW: evals 5, 6 and most of 3 are baseline-saturated; eval 4 invalid; evals.json:76 vacuous; missing: output contract ("never claim it builds"), digest invention, near-miss for a GitHub Actions `docker/build-push-action` workflow, "Dockerfile for a different Rust crate" negative.
12. LOW: `Edit(Dockerfile*)`/`Edit(.dockerignore)` match at any depth; unscoped Read/Grep/Glob wider than needed.
13. LOW: `hadolint/hadolint` unpinned inside a pinning guide (pinning-and-lint.md:106).
14. INFO: SKILL.md 2,699 B; Q/A example duplicates runtime-and-health.md (~350 B); no README stating the assumed workspace layout; `author`/`version` unrecognised keys (would hard-fail a claude.ai upload; `metadata:` is the recognised home).

## Spot-checks (12, confirmed): SIGINT-only server; PID 1 ignores unhandled SIGTERM; default bind `127.0.0.1:8080`; `/healthz` constant `ok`; `required-features` on `migrate` (standalone case only); env-only DATABASE_URL; cargo-chef flags; BuildKit cache-mount syntax; HEALTHCHECK semantics; `docker build --check`; distroless facts; SOURCE_DATE_EPOCH.

## Collisions: none installed; `helm-chart` and `gha-workflow-design` exclude Dockerfiles; add near-miss negatives.
