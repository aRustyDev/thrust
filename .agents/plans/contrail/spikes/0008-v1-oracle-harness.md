# Spike 0008 — v1 oracle harness (pinned Plane + OpenAPI)

**Gates:** ADR-0120, ADR-0122, ADR-0031, ADR-0032.
**Questions:** Q-0122.1, Q-0122.2, Q-0120.3, Q-0032, bead `plane-b6a` (update fork to latest upstream, then pin).
**Timebox:** 3 working days. **Depends on:** `plane-b6a`. **Owner:** TBD.

## Question
Can the pinned fork run in CI via docker compose with its drf-spectacular schema enabled, and can we diff a Rust v1
implementation against it (schema conformance + golden responses + nightly differential)?

## Method
1. Update the fork to latest upstream, pin the commit, run it under compose (Postgres, Valkey, RabbitMQ, MinIO) — round 20
   decision; synthetic seed data only.
2. Confirm the schema endpoint (`api/schema/`) is enabled and covers every v1 operation, including fork additions (Q-0122.1).
3. Normalisation rules for uuid, `sequence_id`, timestamps, ordering; exception-register format (Q-0122.2).
4. Run a schema-conformance tool (Schemathesis-style) against the pinned instance itself to validate the harness (must
   pass against Plane; a deliberately mutated response must fail — control).
5. Record a corpus for the P0 resources (projects, work items, states, labels, members, assets) including error bodies,
   pagination and rate-limit headers.
6. Decide OpenAPI as source of truth for the Rust server: `utoipa`-style code-first vs spec-first generation (Q-0032) and
   how a TS client for scripts/MCP consumers is generated from it (React does **not** get a regenerated client, ADR-0034).

7. **Pin delta:** this fork carries non-upstream code (woven OIDC in `apps/api/plane/authentication/provider/oauth/oidc.py` and `views/app/oidc.py`); record whether the pinned oracle keeps it and list the fork delta against upstream. The schema endpoint is off by default (`ENABLE_DRF_SPECTACULAR`) and covers `/api/v1` only — internal `/api/` (≈233 routes) and `/auth/` have no schema.

## Exit criteria
Harness runs in CI against the pinned instance, passes on Plane, fails on the mutated control; corpus committed; OpenAPI
approach chosen.

## Deliverables
ADR-0122 and ADR-0032 decisions; CI job definition; Q-0122.1/.2 closed.

## Result (round 26)

**Done** (with `plane-b6a`). Findings: `research/0005-oracle-harness.md`; ADR-0122 *Update* has the verdict. Q-0122.1 and Q-0122.2 are answered; the utoipa check and the differential run move to wave 2.
