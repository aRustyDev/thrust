---
status: accepted
date: 2026-10-18
decision-makers: asmith
consulted: []
informed: []
---

# ADR-0048: Audit logging

## Context and Problem Statement

Audit logging is a P0 foundation (F-E06, round 4) and the only consumer of the event/outbox backbone (ADR-0046) with a
compliance flavour. Plane CE has per-entity activity (`IssueActivity` …) and API request logs, not an instance audit
trail; the Open-EE spec added who/what/when for administrative and security-relevant actions **[O]**.

## Decision Drivers

* Tamper-resistance and completeness for security-relevant actions (auth, role changes, API key lifecycle, settings, publish/unpublish of spaces).
* Reuse the outbox; no separate write path that can drift from data changes.
* Retention and export on both adapter sets; no formal compliance driver (Q-0141.1).

## Considered Options

* Option 1: An outbox consumer writes an append-only `audit_log` table (actor, action, target, workspace, ip/ua, before/after summary, request id), with optional hash chaining
* Option 2: Write audit rows synchronously in the same transaction as the change
* Option 3: Ship audit events to an external log store only (OTLP/Logpush)

## Decision Outcome

**Decision (accepted, round 29): at P0 the retained outbox is the audit trail (ADR-0046: outbox rows kept 90 days), with no separate audit table; at P1, when OIDC and custom roles arrive, the leading hypothesis below applies: Option 2 for security-relevant actions plus Option 1 for the rest. F-E06 moves to P1. Per FR-18, `reviews/2026-10-19-fable-architecture-review.md` §5.**

Leading hypothesis (now the P1 design): **Option 2 for security-critical actions plus Option 1 for the rest** (confidence: low-medium): synchronous rows guarantee completeness where it matters; the outbox covers high-volume activity. See the round-29 note.

### Consequences

* Good: completeness for auth/role/key/settings changes; reuses outbox for the rest.
* Bad: two paths to document; table growth needs retention.

### Confirmation

Tests: every security-relevant use-case emits an audit row (checked by a registry of action names); API keys and secrets never appear in `before/after`.

## Update — spike results, round 26 (2026-10-18)

Spike 0004: `/api`, `/auth` and `/live` are Access bypass applications (ADR-0050), and Access does not log bypass requests `[doc]`. The application audit log therefore has to cover every action on those paths, including failed sign-ins and API-key use; there is no second source.

## Update — review round 29 (2026-10-19)

Per FR-18 (`reviews/2026-10-19-fable-architecture-review.md`; the owner's decision is its §5 table: backlog, plus two tier edits, "F-E06 audit logging to P1"), this ADR is accepted and Q-0048 is answered. FR-17 lists it among the ADRs to accept "with F-E06 at P1". FEATURES F-E06 moves to P1 (round 29).

* **P0: the retained outbox is the audit trail.** Every tracked mutation already writes an envelope in its own transaction (`actor_id`, `workspace_id`, `event_type`, `entity_type`, `entity_id`, `occurred_at`, `payload`, `trace_id`; ADR-0046, round 29), and outbox rows are kept 90 days (ADR-0046, round 16). No `audit_log` table, no audit consumer and no synchronous audit rows are built at P0.
* **P1: the leading hypothesis.** Synchronous audit rows in the change's own transaction (Option 2) for the security-relevant actions in the first Decision Driver arrive with OIDC and custom roles (M6, review §3), and an outbox consumer writing `audit_log` (Option 1) covers the rest.
* **Retention and export (Q-0048.1):** 90 days, with a JSON export at P1.
* **What is audited at P1:** the security-relevant actions in the first Decision Driver, including every action on the Access bypass paths (round-26 note above).
* **Hash chaining** stays optional, as Option 1 has it; no formal compliance driver requires it (third Decision Driver, Q-0141.1). It is not built at P0. The rest of FR-18 is backlog (issue #51).

Why: FR-18 counts audit logging among the foundations that can be cheaper at P0. The outbox already records who changed what and when, in the change's own transaction.

Consequences of the P0 stance: the outbox carries events for tracked entity mutations only (ADR-0046), so a failed sign-in or an API-key use leaves no row in it. The round-26 requirement above (the application log must cover every action on the Access bypass paths) therefore binds from the Access phase, which is also P1 (ADR-0090, round 25; FR-13), and the P1 synchronous rows meet it. The Confirmation's test (every security-relevant use-case emits an audit row) applies from P1.

## Open Questions

* Q-0048 — write path (sync vs outbox), hash chaining, and what is audited. (answered round 29, FR-18)
* Q-0048.1 — retention and export format. (answered round 29, FR-17)

## More Information

Related: [[ADR-0046]], [[ADR-0051]], [[ADR-0130]], [[ADR-0141]]. Features: F-E06.
