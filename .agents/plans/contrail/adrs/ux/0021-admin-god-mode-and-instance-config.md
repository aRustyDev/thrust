---
status: accepted
date: 2026-10-17
decision-makers: asmith
consulted: []
informed: []
---

# ADR-0021: Admin (god-mode) app and instance configuration

## Context and Problem Statement

`apps/admin` ("god-mode", served at `/god-mode`) configures the instance: auth-provider toggles, email, and instance settings, backed by `plane.license` and `api/instances/` in Django [S]. In the new system, much instance configuration shifts to deployment configuration (Secrets, bindings, `AppConfig` — ADR-0007, ADR-0092), identity to Zitadel/Access (ADR-0050), and email to the Email port. We must decide what stays editable in-app versus config-as-code.

## Decision Drivers

* Two adapter sets with different config surfaces.
* Secrets must not be stored in the app DB if avoidable (ADR-0092).
* The retained admin UI expects instance-config endpoints.
* Single-org first (ADR-0003).

## Considered Options

* Option 1: Keep god-mode app; implement `api/instances` equivalents backed by an instance-config store (DB), secrets referenced from the Secrets port
* Option 2: Drop god-mode; configuration is deployment-time only (config files/env)
* Option 3: Fold instance settings into a workspace-admin area and retire the separate app

## Decision Outcome

**Decision (accepted, rounds 7, 12 and 23): Option 3 combined with Option 2 — retire `apps/admin`; expose one trimmed public config endpoint and a few workspace settings; all secrets are env/IaC only** (inventory: `research/0001-admin-capability-inventory.md`). (Earlier hypothesis, superseded: Option 1, keep god-mode trimmed to non-secret settings.)

### Consequences

* Good, because the smallest surface, one admin concept, and secrets as config-as-code.
* Bad, because genuinely instance-level settings still need a home that respects tenancy rules (instance vs tenant scope) and operators lose a separate self-service app.
* Neutral: the `compat-plane` adapter must stub or drop the `api/instances` endpoints the React web app calls (ADR-0034).

### Confirmation

The inventory classification in `research/0001-admin-capability-inventory.md` is accepted; each `api/instances` endpoint is implemented, stubbed with a documented default, or removed, and the auth rows are redone after spike 0003.

## Pros and Cons of the Options

### Option 1: Keep, trimmed

* Good, because reuse of UI/contract.
* Bad, because ambiguous config precedence (DB vs deployment config).

### Option 2: Drop

* Good, because the smallest surface and strong config-as-code.
* Bad, because loses self-service; requires UI removal work.

### Option 3: Fold into workspace admin

* Good, because one admin concept.
* Bad, because UI redesign; the instance-level scope doesn't fit workspace scope.

## Update — interview round 7 (2026-10-17)

Direction from the owner: **the admin app's capabilities need to be reviewed; a large portion should be folded into workspace settings, and the remainder should be IaC/env-only settings.** This points away from keeping `apps/admin` (god-mode) as a separate app. It is not a final decision until the capability review is done (new **Q-0021.2**: inventory every capability of `apps/admin` / `api/instances` and classify each as *workspace setting*, *IaC/env only*, or *drop*). Because deployment is single-org first (ADR-0003), "instance" and "workspace" largely coincide, which makes folding natural; the multi-tenant-capable data layer must still keep a place for genuinely instance-level settings. Knock-on: ADR-0011 builds two apps (web, space) instead of three, and Q-0011.1/Q-0021.1 are affected.

## Update — interview round 12 (2026-10-17)

F-A10 tiered **P0 (workspace-settings subset; `apps/admin` retired; the rest env/IaC)**, pending the row-by-row inventory review (Q-0021.2; completed in round 23, below).

## Update — interview round 23 (2026-10-18)

Accepted. The admin inventory classification is accepted wholesale: **one trimmed public config endpoint, a few workspace settings, all secrets in env/IaC, `apps/admin` retired** (Q-0021.1, Q-0021.2). Follow-up: redo the inventory's auth rows against the Rust identity choice after spike 0003.

## Update — review fixes, round 25 (2026-10-18)

Decision line, consequences and Confirmation rewritten to the accepted retire-`apps/admin` outcome; the round-12 note moved into chronological order. Substance unchanged.

## Open Questions

* Q-0021 — keep, drop, or fold.
* Q-0050.3 — which auth methods exist (drives the admin toggles).

## More Information

Related: [[ADR-0007]], [[ADR-0050]], [[ADR-0092]]. Paths: `apps/admin`, `apps/api/plane/license`.
