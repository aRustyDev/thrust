# Spikes

Each spike answers one gating question with a time-boxed experiment. Results are written back into the linked ADR
(status `proposed` → `accepted`/`rejected`) and into QUESTIONS.md. A spike that cannot observe the thing it claims to
test is not evidence — prefer the real operation (real Worker on workerd/Cloudflare, real Postgres, real auth
flow).

Direction recorded in round 21: **backend in Rust from the start; Plane's React frontend adopted with minimal changes;
a Rust (Dioxus) clone of the UI follows once frontend+backend are stable, for multi-target builds** (see
`research/0002-rust-dioxus-fit.md`). Spikes assume that.

Common rules:
- Work in an isolated worktree on a `feat/spike-<n>-<slug>` branch in the **thrust** repo (`git@github.com:aRustyDev/thrust.git`).
- Every spike reports **projected monthly cost** at the sizing target (≤50 users, ≤100k work items; envelope < $100/mo, Q-G5).
- Timebox is a cap; stop and report when it is hit, with what was and wasn't proven.

| # | Spike | Gates | Priority |
|---|---|---|---|
| 0001 | [Rust on Workers feasibility + dual-target slice](0001-rust-on-workers-feasibility.md) | ADR-0006, 0002, 0007 | **First — gates all others** |
| 0002 | [Database engine: `worker-pg` vs an instance-level `worker-do`](0002-database-engine.md) | ADR-0040, 0041, 0083 (Q-0040.6, Q-0083.1, Q-0006.6); confirms ADR-0043's FR-22 order | P0 (M0) |
| 0003 | [Auth slice in Rust: argon2id, session row + CSRF, API keys, one OIDC login](0003-auth-slice-rust.md) | ADR-0050, 0052 (evidence for Q-0050.4's round-29 answer) | P0 (M0) |
| 0004 | [Topology and `.woven` tunnel path](0004-topology-and-woven-tunnel.md) | ADR-0090, 0091 (Q-0090.3) | P0 |
| 0005 | [Collab: Yjs (yrs) on Durable Objects](0005-collab-yrs-durable-objects.md) | ADR-0060, 0061 (Q-G4.1) | P1 |
| 0006 | [Editor data model: conversions without a JS runtime](0006-editor-conversions-rust.md) | ADR-0012, 0061 (Q-0012.1) | P0 gate (ADR-0012) |
| 0007 | [Dioxus multi-target and Cloudflare fit](0007-dioxus-multitarget-fit.md) | ADR-0013 | After backend stable (early feasibility only) |
| 0008 | [v1 oracle harness](0008-v1-oracle-harness.md) | ADR-0120, 0122 (Q-0122.1) | P0 |
| 0009 | [Compat-plane inventory and sizing](0009-compat-plane-inventory.md) | ADR-0034, F-X13 (Q-0010.1) | P0 — before P0 planning |

Order (round 25): 0001 → (0006 ∥ 0002) → (0003 ∥ 0004 ∥ 0008) → 0009 → 0005 → 0007 (0009 needs 0008's recording harness). Spike 0003 uses 0002's candidate engine (Postgres is enough to start).

**Order revised in round 26 (waves).** Only 0001's verdict gates the Rust-dependent spikes, so work that doesn't need Rust starts now:
- **Wave 1 (parallel):** 0001 · 0008 (with bead `plane-b6a`) · 0004 public half (throwaway JS Worker; `.woven` half is a desk recipe) · 0009 desk half (scripted inventory; re-run on the pinned commit; HAR recording waits for 0008's harness).
- **Wave 2 (after 0001 passes):** 0002 ∥ 0006, then 0003. Also re-run 0004 against the Rust Worker, and the utoipa code-first schema check from 0008.
- **Wave 3:** 0009 recordings and replay harness, then 0005, then 0007's early check.

**Order revised in round 29 (FR-04, FR-16; review §3, milestone M0).** Next: **0002** (both cells, after issue #35, the FR-02 port shape) ∥ **0003** (reframed by FR-16 as the two-day auth slice, after issue #36). Then 0005 before P1 collab (M5) and 0007's go/no-go before the Dioxus work (M4). The 0004 Rust-Worker re-run and the 0009 dynamic half keep their round-26 places.

Beads: 0001 `plane-byd.1` · 0004 `plane-byd.2` · 0009 `plane-byd.3` · 0008 `plane-b6a`. Result files (assigned so parallel agents never collide): `research/0003-compat-plane-inventory.md` (0009), `research/0004-rust-on-workers-binding-matrix.md` (0001), `research/0005-oracle-harness.md` (0008), `research/0006-topology-public-half.md` (0004), `research/0011-database-engine-cells.md` (0002, round 29), `research/0012-auth-slice.md` (0003, round 29).

## Status (round 26, 2026-10-18)

| # | Status | Result |
|---|---|---|
| 0001 | **Stages 1 and 2 done: conditional pass.** Stage 2 ran on 2026-10-18 and was torn down (`research/0004` sections 1 and 10); new questions Q-0006.4 to .6. Bead `plane-byd.1` stays open | `research/0004-rust-on-workers-binding-matrix.md`; code on thrust branch `feat/spike-0001-rust-on-workers`, merged as [aRustyDev/thrust#17](https://github.com/aRustyDev/thrust/pull/17) |
| 0002 | Not started. Round 27: scope narrowed to Postgres only (CNPG through Hyperdrive and the Workers VPC service), D1 a desk review, about 3.5 days; waits for bead `plane-byd.4`. **Round 29 (FR-04, FR-16): rewritten as two cells on the same HTTP contract suite and `bench`: `worker-pg` (CNPG through Hyperdrive; `worker-cf` in `scripts/contract.sh`) and `worker-do` (one instance-level Durable Object with SQLite, native twin on `rusqlite`).** Stage 2b (Q-0006.6) folded in; step 6 dropped (FR-07); timebox 3.5 + 3 days; Postgres stays the default if `worker-do` fails any exit criterion, and Q-0040.6 decides if both pass. Waits for issue #35; `worker-pg` recreates the deleted Stage 2 resources. Tracked as issue #37 | |
| 0003 | **Done (issue #38, 2026-10-23): option C, hand-rolled Rust, delivers the P0 credentials and one OIDC login on all four targets.** The main pass is 37 tests on `native-memory`, `native-pg`, `worker-memory` and `worker-cf`; argon2id at OWASP's minimum costs p50 45 ms and p99 78 ms of CPU on a deployed Worker (K4 passes, margin about 300 times); `openidconnect` builds and runs on wasm32 behind a `Fetch` port, in its own crate (K1 passes); the CSRF gap with the stock React app is recorded, not decided (K3, Q-0050.8). **Not run: the Zitadel login (K2) and the OIDC path from a deployed Worker.** ADR-0052 is corrected: a revoked key is 403. Earlier: **Round 29 (FR-16): rewritten as a two-day auth slice, no longer a framework evaluation:** argon2id with OWASP parameters measured on a deployed Worker (must fit the default 30 s CPU limit with margin), a session row with an HttpOnly cookie and a CSRF double-submit token, hashed API keys as `X-Api-Key`, and one OIDC authorization-code login through `openidconnect` against Zitadel or a mock IdP, all through the FR-03 user-only `Principal`. Options A (`better-auth-rs`) and B (a TypeScript identity Worker) dropped; C (hand-rolled Rust) is the plan. Exit: the slice passes the contract suite on the four `scripts/contract.sh` targets. Known unknown: on wasm32, `openidconnect` needs an HTTP client built on `worker::Fetch`. Waits for issue #36. Tracked as issue #38 | `research/0012-auth-slice.md`; thrust branch `feat/auth-slice` (issue #38) |
| 0004 | **Public half: done** (round 27). Two real runs, the second with the Access gate on (35 assertions in 3 passes, all with must-fail controls); one security finding, the case-insensitive bypass (Q-0050.7); a service-binding hop costs +2.3 to +2.8 ms. Two certificate packs remain to delete by hand. **`.woven` half:** desk recipe only, needs four owner decisions. Bead `plane-byd.2` stays open until the Rust Worker re-run | `research/0006-topology-public-half.md` |
| 0005 | Not started | |
| 0006 | **Done** (round 27): Option A (Rust) matches the JS oracle byte for byte (1,337 of 1,337 pairs), recommended for P1 with P0 = B; sanitiser at parity, hardened variant closes all 125 payloads. ADR-0012 was accepted on that basis (Q-0012.3, round 27). Bead `plane-byd.6` | `research/0008-editor-conversions.md`; thrust branch `feat/spike-0006-editor-conversions` (two signed commits), PR [aRustyDev/thrust#18](https://github.com/aRustyDev/thrust/pull/18), rebased onto `main` after #17 merged |
| 0007 | Not started | |
| 0008 | **Done**, with `plane-b6a`: the fork is merged with upstream and pinned by a local tag; harness, register, corpus and diff job built. Open: publishing the pin (Q-0122.4), the draft CI job has never run, the utoipa check and the differential run are wave 2 | `research/0005-oracle-harness.md`; branch `feat/plane-b6a-upstream-sync-oracle` (local) |
| 0009 | **Desk half done** on the pre-pin and the pinned trees. The dynamic half (HAR, call frequency, replay harness) is next now that 0008 exists. Bead `plane-byd.3` stays open | `research/0003-compat-plane-inventory.md`; `scripts/compat-inventory/` |

Next wave: **0002** and **0003** need a TLS-reachable Postgres (0001's Stage 2 needed one too and has run), which is now decided (ADR-0094: CNPG in woven-o11y behind a new Cloudflare Tunnel and a Workers VPC service), **live**: the Secrets Manager, tunnel, VPC-service and Kubernetes roots are applied and verified (`research/0010` section 10); the Hyperdrive configs exist and a Worker smoke test passed (TLS verification is disabled, Q-0094.9); the restore drill is done (Ready after 60 s on a near-empty database; infra PR #810 merged; docs PR #811 open); authenticated TLS (`plane-byd.9`) and a repeat drill with real data (`plane-byd.10`) remain. Spike 0001 Stage 2 RAN on `thrust-pg-dev` on 2026-10-18 (it left nothing behind); spikes 0002 and 0003 can use it, and 0002 should take the Hyperdrive statement-flow constraint (Q-0006.4) and the caching measurement (Q-0006.6). 0006 is done. 0005 follows 0002 (0006 is in); 0007 stays a desk review. The Plane quirks catalogue (`research/0007`, bead `plane-byd.5`) is done. The `space` SSR feasibility build (`research/0009`, bead `plane-byd.7`) is done: feasible, but SSR only produces link-preview metadata, so the owner chose an SPA plus Rust head injection instead (Q-0011.3). **Round 29:** Q-0006.4 is closed by FR-07 (the statement flow is a documented Hyperdrive constraint; no support case), and Q-0006.6 is folded into spike 0002 (FR-16), which also gains the `worker-do` cell (FR-04).

**Scripts these documents cite.** `scripts/compat-inventory/` and `scripts/spike-0004/` are **not** in this copy. They analyse the Plane fork, and the plane fork's required Copy Right Check wants the Plane Software AGPL header on every tracked `.py` file (these 26 were written for the plan, not for Plane), so they sit on branch `chore/contrail-plan-scripts` of `aRustyDev/plane` (one signed commit on that repository's `main`, no PR). Open decision: add the header there, exclude `.agents/**` from that check, or bring the scripts here (this repository has no such check).

**Redactions.** This package is public, so the Cloudflare account id, the thrust tunnel id, the AWS account id, one 1Password item path and an operator email are replaced by placeholders (`<cloudflare-account-id>`, `<thrust-tunnel-id>`, `<aws-account-id>`, `op://<vault>/<item>/token`, `<operator-email>`). Other resource ids (Hyperdrive configs, queues, Access policies, cert packs) and the `woven-o11y` and `.woven` names are not redacted.

**Canonical copy.** This copy, in `aRustyDev/thrust` (merged as #19), is the canonical plan. A mirror at the same redacted state is in `aRustyDev/plane` (PR #61); it is not kept in sync, so change the plan here.
