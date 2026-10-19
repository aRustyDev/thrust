---
status: proposed
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

Decision pending — see Q-0048. Leading hypothesis: **Option 2 for security-critical actions plus Option 1 for the rest** (confidence: low-medium): synchronous rows guarantee completeness where it matters; the outbox covers high-volume activity. Confirm with the owner.

### Consequences

* Good: completeness for auth/role/key/settings changes; reuses outbox for the rest.
* Bad: two paths to document; table growth needs retention.

### Confirmation

Tests: every security-relevant use-case emits an audit row (checked by a registry of action names); API keys and secrets never appear in `before/after`.

## Update — spike results, round 26 (2026-10-18)

Spike 0004: `/api`, `/auth` and `/live` are Access bypass applications (ADR-0050), and Access does not log bypass requests `[doc]`. The application audit log therefore has to cover every action on those paths, including failed sign-ins and API-key use; there is no second source.

## Open Questions

* Q-0048 — write path (sync vs outbox), hash chaining, and what is audited.
* Q-0048.1 — retention and export format.

## More Information

Related: [[ADR-0046]], [[ADR-0051]], [[ADR-0130]], [[ADR-0141]]. Features: F-E06.
