---
status: accepted
date: 2026-10-17
decision-makers: asmith
consulted: []
informed: []
---

# ADR-0110: Deployment pipeline, progressive delivery and rollback

## Context and Problem Statement

We must define how merged code reaches Cloudflare (and the self-hosted twin) safely. Verified CF facts: Wrangler can
`versions upload` without deploying, then `versions deploy` with a traffic split (gradual deployment); you can only
create a deployment from the **last 100** uploaded versions; rollback creates a new deployment from a prior version
but **does not change connected resources** (so data-structure changes can break old code); gradual deployments cause
**version skew** between versions and between Workers linked by service bindings (mitigated by version affinity via the
`Cloudflare-Workers-Version-Key` header and version overrides); **Durable Object lifecycle changes** are atomic, deployed
only via `wrangler deploy`, can't be gradual, and rollbacks cannot cross them. Plane today deploys via images/compose and
a `feature-deployment.yml` preview workflow **[S]**.

## Decision Drivers

* Zero-downtime deploys; fast rollback.
* DB schema changes must be compatible with N and N−1 code (rollbacks and skew).
* Keep DO class lifecycle changes isolated (ADR-0090).
* Static assets use content-hashed names: affinity avoids 404s mid-rollout.
* Deploy identity via OIDC, no long-lived tokens (ADR-0101).

## Considered Options

* Option 1: `wrangler deploy` (100% immediately) on merge to main, per Worker
* Option 2: `versions upload` on merge → gradual deployment (e.g. 10% → 50% → 100%) with version affinity and automated health gates; DO lifecycle changes via a separate manual-gated `wrangler deploy`; expand/contract DB migrations run first
* Option 3: OpenTofu-managed `cloudflare_workers_deployment` as the release mechanism

## Decision Outcome

**Decision (accepted, round 24): Option 2 with measured gates** — expand migration (CI) → upload at 0% traffic → smoke suite and oracle subset via the version-override header → 10% for 10 minutes → 100%; absolute-count gates (≤3 5xx, probe p95 < 500 ms, no CPU-limit exceptions) with automatic `wrangler rollback`; DO lifecycle changes via a manual-gated `wrangler deploy` (see the round-24 update).

(Earlier hypothesis, retained as rationale — the accepted outcome is above: Leading hypothesis: **Option 2** (confidence: medium).
Order of operations: (1) expand-phase DB migration; (2) deploy callee Workers (`collab`, `consumer`) compatibly;
(3) `api` gradual with version affinity keyed on user/session; (4) contract-phase migration in a later release.
Self-hosted: images tagged by release-please version, rolled via Compose/K8s rolling update.)

### Consequences

* Good, because errors are caught on a slice of traffic; rollback is one command (within the 100-version window).
* Bad, because every migration must be expand/contract; pipeline needs health gates (error rate from Workers metrics/OTel, ADR-0130).
* Neutral: preview environments per PR are a separate question (Q-0092.2).

### Confirmation

Pipeline integration test: simulate deploying N+1 `api` next to N `consumer`; schema N−1 compatibility test (ADR-0121
fixtures); runbook drill for rollback and for a DO-lifecycle deploy.

## Pros and Cons of the Options

### Option 1: Direct deploy

* Good, because simple.
* Bad, because no blast-radius control; skew-free but all-or-nothing.

### Option 2: Gradual + affinity + gates

* Good, because supported platform feature; observable in Workers Metrics release annotations.
* Bad, because skew handling and migration discipline required; DO changes can't use it.

### Option 3: TF-driven deployment

* Good, because auditable in IaC.
* Bad, because slows deploys, awkward gating, conflicts with ADR-0093 leading option.

## Update — interview round 16 (2026-10-17)

Direction: expand-then-contract migrations run by CI before deploy (Q-0110.2); gradual deployments with a health gate. Gate signal and thresholds (Q-0110.1) remain open, so this stays `proposed`.

## Update — interview round 24 (2026-10-18)

Accepted: **Option 2 with measured gates** (Q-0110.1). Order: expand migration (CI) → upload the new version at **0% traffic** → run the **smoke suite and an oracle subset against it via the version-override header** → promote to **10% for 10 minutes** → promote to 100%. Gates use **absolute counts, not rates**, because ≤50 users make percentage slices statistically meaningless: smoke 100% pass, **≤3 5xx**, synthetic-probe **p95 < 500 ms**, **no CPU-limit exceptions**. Any gate failure triggers an automatic `wrangler rollback`. Durable Object lifecycle changes use a separate manual-gated `wrangler deploy`. Self-hosted: rolling update gated on the health endpoint. Version-override header behaviour to be confirmed against current Cloudflare docs during implementation.

## Update — review fixes, round 25 (2026-10-18)

Outcome rewritten to the accepted decision; the earlier hypothesis and order of operations are kept as rationale.

## Open Questions

* Q-0110 — pipeline shape.
* Q-0110.1 — Health-gate signal and thresholds for auto-promotion/rollback.
* Q-0110.2 — Migration runner: who runs expand migrations on Hyperdrive/D1/DO-SQLite (ties to Q-0042)?
* Q-0092.2 — preview environments.

## More Information

CF docs: gradual deployments, version affinity/overrides, rollbacks, DO migrations. Related: [[ADR-0090]], [[ADR-0093]], [[ADR-0100]], [[ADR-0101]], [[ADR-0130]].
