---
status: proposed
date: 2026-10-17
decision-makers: asmith
consulted: []
informed: []
---

# ADR-0122: Public API v1 oracle and parity tests

## Context and Problem Statement

ADR-0031 (accepted) keeps public `/api/v1` wire-compatible with Plane. We need an executable definition of
"compatible". Plane exposes an OpenAPI schema via drf-spectacular at `api/schema/` when enabled **[S]**; `api/v1`
covers assets, cycles, intake, invites, labels, members, modules, projects, states, stickies, users and
work items, authenticated by `X-Api-Key` with a rate limit **[S]**. Schemas catch shape drift but not behavior
(sequence ids, pagination, expansion params, error bodies, rate-limit headers, soft-delete visibility).

## Decision Drivers

* Existing integrations (scripts, the woven `plane` skill, Beads sync) must keep working.
* Detect both structural and behavioral drift.
* Keep oracle artifacts reviewable and versioned.

## Considered Options

* Option 1: Schema-first — freeze the drf-spectacular OpenAPI as the contract; run schema conformance/fuzz tests (e.g. Schemathesis-style) against the new API
* Option 2: Golden recordings — record request/response pairs from a live reference Plane; replay and diff (with normalisation for ids/timestamps)
* Option 3: Differential — run scenarios against reference Plane and new API in lockstep, diff observable state

## Decision Outcome

Decision pending — see Q-0122. Leading hypothesis: **Option 1 as the PR gate, Option 2 for a curated scenario
corpus, Option 3 nightly** (confidence: medium).

### Consequences

* Good, because layered strictness balances speed and fidelity.
* Bad, because three artifacts to maintain; normalisation rules need care.
* Neutral: the Community-Edition endpoint surface differs from published Cloud docs (the `plane` skill notes docs describe Cloud/EE endpoints that 404 on this instance) — the oracle must be derived from the **running CE instance**, not the public docs.

### Confirmation

CI gate: schema conformance 100% on all v1 operations that are P0/P1; corpus replay diff-clean after normalisation;
any intentional deviation requires a recorded exception in **`DEVIATIONS.md`** (id, reason; package root).

## Pros and Cons of the Options

### Option 1: Schema-first

* Good, because cheap, deterministic, no live Plane needed after the schema is frozen.
* Bad, because schema may be incomplete or enable-flag gated; doesn't prove semantics.

### Option 2: Golden recordings

* Good, because captures real behavior incl. error shapes.
* Bad, because recordings go stale; id/time normalisation.

### Option 3: Differential

* Good, because strongest evidence for stateful behavior.
* Bad, because needs a reference deployment and deterministic seed; slow.

## Update — interview round 8 (2026-10-17): reference

The owner set the oracle reference (Q-G1, Q-0120.1): **update this fork to the latest upstream Plane changes, then pin it; use the pinned running instance *plus* its OpenAPI (drf-spectacular) schema together** — i.e. the "both: schema for shape, pinned instance for behavioral golden recordings" option. This depends on bead `plane-b6a` (update + pin) and is separate from the fork-and-diverge decision for the *frontend* (ADR-0001): the Django fork is frozen at the pin as a reference, not a codebase we keep evolving. Where the pinned instance runs for CI is open (Q-0120.3). The schema endpoint's coverage of every v1 operation is still to be verified (Q-0122.1).

## Update — review fixes, round 25 (2026-10-18)

Scope corrections from the review of the Plane source: the drf-spectacular schema covers **`/api/v1/` only** (`SCHEMA_PATH_PREFIX`, `preprocess_filter_api_v1_paths` in `settings/openapi.py`) and the endpoint is **disabled unless `ENABLE_DRF_SPECTACULAR=1`**; the internal `/api/` (≈233 `path()` entries in `plane.app`), `/api/public/` and `/auth/` have **no schema**, so the compat-plane adapter (ADR-0034) is validated by golden recordings and the nightly differential run, not schema conformance. "Match Plane's published OpenAPI" (SPEC R-F5) is therefore defined as **"match the pinned instance's schema and recorded behaviour"**. **Pin delta:** this fork carries non-upstream code (woven OIDC: `apps/api/plane/authentication/provider/oauth/oidc.py`, `views/app/oidc.py`); after the update-to-latest-upstream-and-pin step (bead `plane-b6a`) record whether the pinned oracle keeps it and list the fork delta (spike 0008). Intentional deviations (e.g. 30-day vs Plane's 60-day retention) go in `DEVIATIONS.md`; the parity/traceability matrix lives in `TRACEABILITY.md`.

## Update — spike results, round 26 (2026-10-18)

**Spike 0008 result** (`research/0005-oracle-harness.md`; branch `feat/plane-b6a-upstream-sync-oracle`, tag `oracle/v1-pin-20261018` pointing at `ba85d4d3d3`; both **local, not pushed**). The pin exists and the harness works, but the oracle is **schema + exception register + golden corpus**, not the schema alone. Figures are the spike agent's report; the coordinator verified the branch, the four commits and the tag.

* **Q-0122.1 answered: no. 130 of 135 routed v1 operations are in the schema (96.3 %), and not for the reason this ADR assumed.** Routes were enumerated from Django's live URL resolver, not from view classes (a naive count gave 176 and was wrong). The 5 gaps are 2 PUTs and 3 `server` paths that the project's own hook filters out. **v1 estimates are not routed** (404 on Plane too), so "estimates" is removed from the v1 resource list (also ADR-0031). Six operations have no documented 2xx body: user-asset upload, generic asset upload, asset get, work-item attachment upload (both path families) and project summary. `State.required` is only `[color, name]`, and 29 operationIds end in `_2` because both `issues` and `work-items` (and `members` and `project-members`) are routed and documented.
* **Plane fails its own schema.** The schema-driven conformance gate (Schemathesis 4.31.1, seed 1; 102 operations tested, 78 reaching a 2xx, 590 cases) finds **67 failures in 12 classes** on the unmodified pinned Plane: unhandled 500s, undocumented 400/404s, undeclared nullables, `fields` projections that omit required keys, bare arrays where an envelope is documented, search results in an `{"issues": [...]}` wrapper that the schema omits (EX-011, corrected in round 27), and others. So "conformance 100 %" below can only mean **"the failure set equals the committed register"** (`oracle/exceptions.yaml`, `format_version: 1`, 12 entries). The gate fails on an unmatched failure, a stale entry, a runtime error or a vacuous run. Each entry carries `rust_expectation`: `reproduce` for 10 and `fix` for EX-001 and EX-002, which are one failure — an unhandled exception returning 500 on eight v1 operations — seen through two checks. Causes: `per_page=0` (ZeroDivisionError) and `per_page<0` in `utils/paginator.py`, invalid `fields`/`expand`/payload shapes, and the asset-upload `is_server` bug. A faithful port must not reproduce a crash, so these are deviation candidates (Q-0122.3, `DEVIATIONS.md`).
* **Q-0122.2 answered:** normalisation rules and the register format are in `oracle/NORMALISATION.md` and `oracle/exceptions.yaml`. The first rule set missed two things that the CHECK-mode diff exposed and that are now covered: dashless `uuid4().hex` storage keys and a random default project-logo icon.
* **Controls.** The gate passes on Plane. Through a mutating proxy, changing a field type, changing an enum value and dropping a required field each **fail** on the right operation. **Blind spot:** dropping `id` is not detected, because the schema does not mark it required. The first controls run failed because the gate's own fuzz corrupted the database for later runs; every gate and control run now starts from a restored snapshot.
* **Corpus:** 167 exchanges in 10 files (statuses 200, 201, 204, 400, 401, 403, 404, 405, 409, 415, 429, 500); a second recording on a fresh stack is byte-identical. **Not covered:** cycles, modules, intake, stickies, and DELETEs of seeded entities. The fuzz phase is deterministic for one seed; a new seed finds new instances of known classes (accepted), while a new class fails the gate.
* **Schema diff:** `oasdiff` 1.33.0 reports no change on identity and detects 6 of 6 injected break kinds; it does not flag an additive change. Never pass `--open` (it uploads the specs to oasdiff.com).
* **Pipeline:** `oracle/scripts/run-all.sh` ran twice from a fresh stack (`--refreeze`, then CHECK mode, which diffs manifest, schema, corpus, coverage and conventions against the committed copies). Cold image build 58 s (464 MB), cold start 87–90 s. The nightly CI definition `oracle/ci/oracle.yml` is a draft that **has never run in CI**.
* **Pin delta (spike step 7):** upstream preview `879ce8b5c6` (1.4.2) merged into the fork, 20 conflicts all resolved; fork-only delta 78 files (+3838/−209); **the woven OIDC code is kept**, and the fork's admin OIDC UI had to be ported to upstream's new components (it merged textually but no longer compiled; type-check and lint are green, rendering not checked in a browser). The fork adds no v1 routes; its only v1 wire change is `upload_data.method` in upload responses, which the schema does not describe.
* **Not done (wave 2):** the utoipa code-first generation check and the Rust-versus-oracle differential run.

**Recommended Confirmation wording** (replaces the text above when the owner accepts this ADR): *CI gate: the schema-conformance failure set equals the committed exception register; corpus replay is diff-clean after normalisation; every intentional deviation has a `DEVIATIONS.md` entry.* The layering (schema as the PR gate, the corpus for curated scenarios, the differential run nightly) is supported by the evidence. The ADR stays `proposed` until the owner accepts it.

## Update — owner decisions and spike results, round 27 (2026-10-18)

Owner decisions, round 27: **Q-0122.3** — every register entry stays `reproduce`, including EX-001 and EX-002 (the 500s); no deviation is approved, and the `id` blind spot of the control proxy stays documented. **Q-0122.4** — tags are not pushed to the oracle; the pin remains the local annotated tag `oracle/v1-pin-20261018` and `oracle/PIN` on the local branch, so the branch must be kept (it is the only copy). The quirks catalogue (`research/0007`) supersedes `DEVIATIONS.md` candidates C-1 to C-4 as the reference.

## Update — quirks catalogue findings, round 27 (2026-10-18)

The quirks catalogue (`research/0007-plane-quirks-catalog.md`, 157 entries, bead `plane-byd.5`) corrects and extends the register: **EX-011** is a wrapper the schema omits, not a missing `id`; **EX-010** is a drf-spectacular component-name collision; **EX-003 and EX-004 partly measure the fixture** (the seeded projects have cycles, modules and intake switched off, so those families' success shapes were never exercised); and the 500 set is **larger than EX-001/002** (a `deleted_at` body on work-item create, a non-object `issue` on intake create, array bodies, two more `is_server` sites). "Reproduce all 500s" covers all of them. The corpus still does not cover cycles, modules, intake success paths, stickies, invitations, DELETEs of seeded entities, any non-UTC user, or any of `/auth`, `/api`, `/api/public`, `/api/instances`; extending the corpus to those is a follow-up for the dynamic half of spike 0009.

## Open Questions

* Q-0122 — oracle strategy.
* Q-0122.1 — Is the schema endpoint enabled on the reference instance, and does it cover all v1 operations (including those added by the fork)? **Answered (round 26): enabled by `ENABLE_DRF_SPECTACULAR=1`; 130 of 135 routed operations; the fork adds none.**
* Q-0122.2 — Normalisation rules (uuid, `sequence_id`, timestamps, ordering) and the exception register format. **Answered (round 26): `oracle/NORMALISATION.md`, `oracle/exceptions.yaml`.**
* Q-0122.3 — Which register entries become numbered deviations, and the `id` blind spot. **Answered (round 27): none; all `reproduce`.**
* Q-0122.4 — Publish the oracle pin. **Answered (round 27): do not push tags to the oracle.**

## More Information

`apps/api/plane/api/` (v1), `api/schema/`. Related: [[ADR-0031]], [[ADR-0033]], [[ADR-0100]], [[ADR-0120]].
