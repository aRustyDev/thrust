---
status: accepted
date: 2026-10-17
decision-makers: asmith
consulted: []
informed: []
---

# ADR-0140: Licensing posture — AGPL-3.0 and clean-room boundaries

## Context and Problem Statement

Plane Community Edition is licensed **GNU AGPL v3** (`LICENSE.txt` in this repo, verified). The hybrid approach (ADR-0001)
keeps the React apps and shared packages (derivative of AGPL code) and rewrites the backend. The Open-EE program
spec'd **clean-room** development for features Plane gates behind proprietary paid tiers, with AGPL compliance **[O]**
(`PROGRAM.md`). The new backend is a new implementation behind a compatible `/api/v1` wire contract. AGPL §13 (network
use) requires offering corresponding source to users interacting remotely with a *modified* AGPL work. This ADR is an
engineering position, **not legal advice** — legal review is an explicit gate.

## Decision Drivers

* Avoid copying proprietary EE code or EE-only design artefacts.
* Keep the project redistributable and the retained frontend compliant.
* Allow the new backend to be used/relicensed as the owner prefers, if legally defensible.
* Auditability: be able to show provenance of each module.

## Considered Options

* Option 1: Everything AGPL-3.0 (frontend retained + backend new), with a clean-room *process* only for EE-equivalent features
* Option 2: Dual posture — frontend/shared packages AGPL-3.0 (derivative); new backend under a separate (e.g. permissive or source-available) licence as an independent work, with a documented clean-room process for *all* backend code
* Option 3: Pure greenfield (no retained AGPL code) — rewrite frontend too, licence freely

## Decision Outcome

**Decision (accepted, round 14): everything stays AGPL-3.0 with no clean-room claim** — frontend and backend, one rule for contributors; implementers may read the Django source. The remaining constraint is not copying proprietary EE code or EE-only design artefacts. §13 is met by publishing the source of the deployed combination. (Earlier hypothesis, superseded: Option 1 with a clean-room *process* for EE-equivalent features.) This is an engineering position, not legal advice.

### Consequences

* Good, because compliance is uniform and easily audited; contributors need one rule.
* Bad, because forecloses a permissive backend licence unless revisited.
* Neutral: no clean-room process is required; the constraint that remains is not copying proprietary EE code or EE-only artefacts — specs for EE-equivalent features come from public documentation, black-box observation or the Open-EE specs.

### Confirmation

Per-feature provenance column in `TRACEABILITY.md` (source of behavioural spec: public docs / CE observation / Open-EE spec); deviations in `DEVIATIONS.md`;
`copyright-check.yml` (existing) extended to new packages; a release checklist item "source offer available" (§13).

## Pros and Cons of the Options

### Option 1: All AGPL-3.0

* Good, because simple, safe, consistent with the fork.
* Bad, because copyleft applies to the whole combined deployment.

### Option 2: Dual posture

* Good, because possible commercial flexibility for the backend.
* Bad, because legal uncertainty, process burden, risk of contamination claims.

### Option 3: Greenfield

* Good, because full licensing freedom.
* Bad, because contradicts ADR-0001 (hybrid) and multiplies cost.

## Update — interview round 14 (2026-10-17)

Accepted. The owner chose to **keep AGPL-3.0 for everything with no clean-room claim** (Q-G6). This is a narrower stance than Option 1: no clean-room process is required, so the implementers may read the Django source. The constraint that remains is not copying proprietary EE code or EE-only design artefacts. Q-0140.1 is moot; Q-0140.2 (name/branding) remains open.

## Update — interview round 15 (2026-10-17)

Branding (Q-0140.2): owner wants to **rename the product now** — a new name must be chosen before the first deploy (new Q-0140.3).

## Update — interview round 18 (2026-10-18)

Naming (Q-0140.3): **MVP name is Contrail** (the rewrite of Plane for Cloudflare); **v1 name is Thrust**. Credit to Plane is retained in AGPL notices.

## Update — review fixes, round 25 (2026-10-18)

Decision line rewritten to the accepted outcome; the 'clean-room process required regardless' wording is removed and the provenance artefact now lives in `TRACEABILITY.md`. Substance unchanged.

## Open Questions

* Q-0140 / Q-G6 — licensing. **Answered (round 14).** Counsel review remains an explicit gate.
* Q-0140.1 — **Moot (round 14: no clean-room claim).**
* Q-0140.2 — **Answered (round 15): rename.** Q-0140.3 — **answered (round 18): Contrail (MVP), Thrust (v1).**

## More Information

`LICENSE.txt`; `.claude/plans/plane-fork/PROGRAM.md` on `woven/plan` (clean-room and AGPL notes). Related: [[ADR-0001]], [[ADR-0004]], [[ADR-0101]].
