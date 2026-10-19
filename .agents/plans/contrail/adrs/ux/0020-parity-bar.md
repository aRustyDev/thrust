---
status: accepted
date: 2026-10-17
decision-makers: asmith
consulted: []
informed: []
---

# ADR-0020: Parity bar — how "equivalent" to Plane must behavior be

## Context and Problem Statement

The goal is an "equivalent" application, but "equivalent" is undefined (Q-G1). Because the UI is retained (ADR-0001), visual parity is largely inherited; the open issue is behavioral parity of the backend: ordering, pagination, filter semantics, side-effects (activities, notifications, webhooks), and edge cases such as soft delete and archived/draft visibility [S]. The parity bar sets test cost and determines what counts as a regression.

## Decision Drivers

* Test effort grows steeply with strictness.
* Users migrating from Plane should not see surprises.
* Intentional improvements (and Open-EE features) will diverge.
* A parity oracle must exist (ADR-0122).

## Considered Options

* Option 1: Strict behavioral parity for P0 features, verified against a live Plane via recorded golden responses
* Option 2: Functional equivalence — same capabilities and data model semantics, differences allowed in non-visible details
* Option 3: Parity only on the public `/api/v1` contract and on user-visible flows; internals free

## Decision Outcome

**Decision (accepted, rounds 14 and 23): functionally equivalent UI, with `/api/v1` as the strict contract (Option 3).** The Dioxus clone (ADR-0013) targets Plane UI/UX parity first, then diverges. A short list of high-risk invariants (sequence ids, soft-delete visibility, activity generation) is proposed for golden tests (Option 1 applied narrowly); that list is to be curated in `TRACEABILITY.md`. (Earlier hypothesis: Option 3 with Option 1 on high-risk behaviours — consistent with, but more specific than, the owner's answer.)

### Consequences

* Good, because focused test investment where parity matters.
* Bad, because the invariant list must be curated and can miss edge cases.
* Neutral: differences are recorded as deliberate deviations in `DEVIATIONS.md`.

### Confirmation

A parity matrix in `TRACEABILITY.md` lists each feature with its bar (strict / functional / free) and its oracle; deviations are recorded in `DEVIATIONS.md` and need an ADR or a QUESTIONS.md row.

## Pros and Cons of the Options

### Option 1: Strict for P0

* Good, because the highest confidence in migration.
* Bad, because expensive; requires a live Plane and a diff harness.

### Option 2: Functional equivalence

* Good, because cheap and flexible.
* Bad, because vague; regressions are hard to define.

### Option 3: API + flows

* Good, because aligned with contracts and user value.
* Bad, because internal behaviors can drift unnoticed.

## Update — interview round 14 (2026-10-17)

Accepted direction (Q-0020): **functionally equivalent UI**; the `/api/v1` contract is the strict bar (formally accepted in round 23, below).

## Update — interview round 23 (2026-10-18)

Accepted: **functionally equivalent UI** with `/api/v1` as the strict contract (round 14). The Dioxus clone (ADR-0013) targets Plane UI/UX parity first, then diverges.

## Update — review fixes, round 25 (2026-10-18)

Decision line states the accepted outcome; artefact promises now point to `TRACEABILITY.md` and `DEVIATIONS.md` instead of SPEC.md; the round-23 note moved into chronological order. Substance unchanged.

## Open Questions

* Q-0020 — per-tier parity bar.
* Q-G1 — which Plane release defines "equivalent".

## More Information

Related: [[ADR-0031]], [[ADR-0122]], [[ADR-0120]].
