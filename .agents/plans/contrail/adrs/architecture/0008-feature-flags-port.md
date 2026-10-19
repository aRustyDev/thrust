---
status: proposed
date: 2026-10-18
decision-makers: asmith
consulted: []
informed: []
---

# ADR-0008: Feature flags port

## Context and Problem Statement

Feature flags are one of the four P0 foundations (F-E00a, round 4). Open-EE gated features behind flags resolved per
instance/workspace **[O]**. Plane CE has instance-level config but no flag system. Flags gate capabilities in the
`Authorizer` (ADR-0051), UI affordances and rollout of Open-EE-equivalent features (custom roles, custom fields, …).

## Decision Drivers

* Per-instance and per-workspace evaluation, cheap in the request path, no external dependency at P0.
* Same behaviour on both adapter sets; testable.
* Admin-editable without redeploy (workspace settings, ADR-0021).

## Considered Options

* Option 1: A `Flags` port backed by a database table (`key`, `scope` instance|workspace, `enabled`, optional rules), read per request and memoised for the request
* Option 2: Static configuration (env/TOML) only, change = redeploy
* Option 3: OpenFeature-style port with an external provider (Unleash/flagd) adapter

## Decision Outcome

Decision pending — see Q-0008. Leading hypothesis: **Option 1** (confidence: medium): consistent with "no cache at P0", editable at runtime, trivial self-hosted; keep the port OpenFeature-compatible in semantics so Option 3 can be added as an adapter.

### Consequences

* Good: runtime control, audit via ADR-0048, no new infra.
* Bad: one more indexed read per request unless memoised; flag hygiene (removal) is on us.

### Confirmation

Contract tests for flag evaluation (instance default, workspace override, unknown key = off); a lint that new capabilities declare their flag.

## Open Questions

* Q-0008 — flag storage/evaluation model and who may edit flags.

## More Information

Related: [[ADR-0021]], [[ADR-0051]], [[ADR-0048]]. Features: F-E00a.
