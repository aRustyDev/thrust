---
status: accepted
date: 2026-10-17
decision-makers: asmith
consulted: []
informed: []
---

# ADR-0093: Infrastructure-as-code tooling for Cloudflare resources

## Context and Problem Statement

The Cloudflare adapter set needs durable resources (Hyperdrive/D1/Postgres connectivity, Queues, R2 buckets, KV
namespaces, DO namespaces, Workflows, DNS/Custom Domains, Access applications/policies, Logpush/OTel destinations) and
Worker code deploys. The woven infrastructure repo already uses OpenTofu with the Cloudflare provider and has roots with
`backend "local"` **and** R2-backed roots **[user rules]**. Verified CF facts: the Terraform provider now has
`cloudflare_worker`, `cloudflare_worker_version`, `cloudflare_workers_deployment`, and `cloudflare_workflow`
resources; CF explicitly supports a split where platform teams own the Worker resource in Terraform while dev teams
deploy code with Wrangler.

## Decision Drivers

* Reproducible, reviewable infra; drift detection.
* Developers need a fast `wrangler dev`/`deploy` loop; bindings must stay in sync with code.
* Single source of truth for binding names (avoid TF/wrangler skew).
* Local-state caveat: roots with `backend "local"` must be **applied from the canonical checkout only**, never a worktree; a state file that lives beside a working copy is fragile for CI.
* Liveness: tofu with R2 backend never touches AWS, so SSO-liveness probes do not apply to a Cloudflare-only root (but a CF API token does).

## Considered Options

* Option 1: Wrangler-only (`wrangler.jsonc` declares bindings; `wrangler deploy` provisions with auto-provisioning where available)
* Option 2: OpenTofu for stateful resources/edge config + Wrangler for code and bindings (CF-documented split)
* Option 3: OpenTofu for everything including Worker versions/deployments

## Decision Outcome

**Decision (accepted, round 15): Option 2** — OpenTofu (Cloudflare provider) in the infra repo owns long-lived resources, DNS/Access/destinations and the `cloudflare_worker` shell; Wrangler owns bundles, bindings and versioned/gradual deploys (ADR-0110); remote (R2) state backend.

(Earlier hypothesis, retained as rationale — the accepted outcome is above: Leading hypothesis: **Option 2** (confidence: medium): OpenTofu owns long-lived
resources, DNS/Access/destinations and the `cloudflare_worker` shell; Wrangler owns bundles, bindings and
versioned/gradual deploys (ADR-0110). The new root uses a **remote (R2) backend**, not `backend "local"`, so CI can
plan/apply without the canonical-checkout constraint.)

### Consequences

* Good, because it matches CF's guidance and the existing woven toolchain.
* Good, because gradual deployments stay in Wrangler's hands (the TF deployment resource would otherwise fight them).
* Bad, because binding IDs (KV/D1/Hyperdrive IDs) must flow from TF outputs into `wrangler.jsonc` (generated or env-injected).
* Neutral: self-hosted twin uses Compose/K8s manifests; no TF.

### Confirmation

CI: `tofu plan` in the target root (the only discriminating probe) on PR; a check that every binding in each
`wrangler.jsonc` resolves to a TF output or an explicit literal; drift job on schedule.

## Pros and Cons of the Options

### Option 1: Wrangler-only

* Good, because one tool, great DX, bindings colocated with code.
* Bad, because no drift detection or policy review for Access/DNS/destinations; weak for multi-environment governance.

### Option 2: OpenTofu + Wrangler

* Good, because clear ownership boundary; reviewable infra changes.
* Bad, because two tools and a handoff for IDs.

### Option 3: OpenTofu for everything

* Good, because one reviewed pipeline.
* Bad, because TF deployment resource vs gradual deployment/version affinity; slower code loop; DO migration (`exports`) semantics awkward.

## Update — interview round 15 (2026-10-17)

Accepted. **OpenTofu with the Cloudflare provider in the infra repo** creates resources and IDs; Worker code deploys via wrangler in CI. 1Password for local secrets, GitHub environment secrets for CI (Q-0093.1, Q-0092.1). State backend follows infra-repo practice.

## Update — review fixes, round 25 (2026-10-18)

Outcome rewritten to state the accepted decision; secret values stay out of OpenTofu state (ADR-0092).

## Open Questions

* Q-0093 — tooling split.
* Q-0093.1 — Which repo hosts the Cloudflare TF root: the woven infrastructure repo vs this repo? State backend and who may apply.
* Q-0093.2 — Wrangler auto-provisioning of bindings vs TF-created IDs?

## More Information

CF docs: Workers infrastructure-as-code, Terraform provider (worker/version/deployment/workflow resources).
Related: [[ADR-0090]], [[ADR-0092]], [[ADR-0110]].
