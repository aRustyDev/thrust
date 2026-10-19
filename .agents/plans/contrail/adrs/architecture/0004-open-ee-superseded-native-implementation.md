---
status: accepted
date: 2026-10-17
decision-makers: asmith
consulted: []
informed: []
---

# ADR-0004: Open-EE program is superseded — features are implemented natively in the new backend

## Context and Problem Statement

The fork has an "Open-EE" program (branch `woven/plan`, `.claude/plans/plane-fork/PROGRAM.md`) that builds Plane's paid-tier features (RBAC, ReBAC, SCIM, audit, custom fields, workflows, dashboards, templates, nested pages, time tracking, intake email, CRM, Slack, Beads/GitHub sync) into the Django CE fork, with Phase 0 foundations (feature flags, permission seam, event/outbox backbone, EE isolation, fork CI). It has 31 feature specs and 5 research recon files. Building these on Django and then rewriting the backend would pay for the work twice.

## Decision Drivers

* Avoid double implementation on a backend that is being replaced (ADR-0001).
* The specs and research hold valuable requirements and recon.
* Phase 0 seams (permission, events, flags) map directly onto the new architecture.
* Clean-room and AGPL concerns (Q-G6).

## Considered Options

* Option 1: Continue Open-EE on Django, then port the result
* Option 2: Supersede — implement the features natively in the new backend, reusing specs and research as input
* Option 3: Run both in parallel (Django for the live instance, new backend for the future)

## Decision Outcome

Chosen option: **Option 2**, because the backend is being rewritten. Source: interview 2026-10-17, round 2 ("Replace: the Cloudflare app supersedes Open-EE on Django").

Reuse: `woven/plan:.claude/plans/plane-fork/features/*` and `research/*` become requirements input to FEATURES.md section B. The F0 foundations (feature flags, permission seam, event backbone) become first-class parts of the new architecture, with the permission seam called in the use-case layer.

### Consequences

* Good, because no throwaway Django work.
* Bad, because the live instance gets none of these features until the new backend ships, unless Option 3-style stopgaps are made.
* Neutral: in-flight Django Open-EE branches should be frozen or closed deliberately (follow-up bead).

### Confirmation

FEATURES.md section B lists every Open-EE spec with a tier; the permission seam and event backbone exist before any seam-consuming feature (Q-F2).

## Pros and Cons of the Options

### Option 1: Continue on Django

* Good, because features land on the live instance sooner.
* Bad, because the work is discarded or re-ported.

### Option 2: Supersede

* Good, because the work is done once on the target stack.
* Bad, because there is a delivery gap.

### Option 3: Parallel

* Good, because it hedges.
* Bad, because two code paths and double the effort.

## Open Questions

* Q-F2 — which Open-EE features are P0 given the cross-cutting seams.
* Q-G6 — licensing and clean-room posture.

## More Information

Related: [[ADR-0001]], [[ADR-0051]], [[ADR-0046]]. Specs live on branch `woven/plan`, not on `main`.
