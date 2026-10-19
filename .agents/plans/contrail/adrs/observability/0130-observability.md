---
status: accepted
date: 2026-10-17
decision-makers: asmith
consulted: []
informed: []
---

# ADR-0130: Observability — logs, traces, metrics across adapters

## Context and Problem Statement

Plane today uses OpenTelemetry, PostHog and Scout APM **[S]**. The new system spans Workers, Queues, Workflows and DOs
on Cloudflare, and Node processes on the self-hosted twin. A request can hop API → outbox → Queue → consumer →
webhook, so trace context must survive asynchronous boundaries. Verified CF facts: Workers Logs and automatic
**tracing** (opt-in via `observability.traces`) capture fetch/binding/handler spans; Workers can export logs and traces
to any **OTLP** destination (destinations configured in the dashboard, named in Wrangler config), optionally with
`persist: false`; default head sampling rate is 100%; Workers Logpush remains available (OTel export is recommended for
new integrations); Workers Metrics now annotates releases and gradual deployments; OTel destination usage is billed
event-based. Status for Woven services is tracked in the Gatus page (`status.woven`) **[user rules]**.

## Decision Drivers

* One telemetry vocabulary on both adapter sets (ADR-0002): a `Telemetry` port.
* Trace propagation through Queue/Workflow/DO hops (CF auto-tracing may not stitch application-level async hops).
* Release correlation for gradual deployments (ADR-0110).
* Privacy: no PII/secrets in logs; tenant id as an attribute, not content.
* Cost control: sampling.

## Considered Options

* Option 1: OTel API in core behind a `Telemetry` port; CF adapter uses native Workers Observability (logs+traces) with OTLP export destinations; self-hosted adapter uses OTel SDK + collector
* Option 2: Native Cloudflare only (Workers Logs/Traces/Analytics Engine); self-hosted gets a separate stack
* Option 3: Logpush to R2/third-party plus a hosted APM SDK in code

## Decision Outcome

**Decision (accepted, round 18): Option 1** — Workers Observability plus OTLP export to the existing woven observability stack; PostHog dropped; Gatus probes health/API SLIs; W3C `traceparent` propagated across Queue, Workflow and DO hops.

(Earlier hypothesis, retained as rationale — the accepted outcome is above: Leading hypothesis: **Option 1** (confidence: medium). Messages on the `Queue` port
carry a W3C `traceparent` field set by the producer and continued by the consumer; Workflow steps and DO calls take it
as an explicit argument.)

### Consequences

* Good, because one instrumentation style; backend vendor swappable via OTLP.
* Bad, because manual propagation across async boundaries is easy to forget — enforce in the Queue port types.
* Neutral: Gatus probes remain the user-path liveness signal; tracing explains *why*, not *whether*.

### Confirmation

Contract test (ADR-0121): a message published with trace context yields a consumer span with the same trace id on
both adapters; CI check that all log calls go through the structured logger (no raw `console.log` with objects).

## Pros and Cons of the Options

### Option 1: OTel via port, native CF + OTLP

* Good, because portable and CF-native where available.
* Bad, because dual maintenance of two adapters; sampling config differs per set.

### Option 2: Native CF only

* Good, because least code on CF.
* Bad, because breaks parity with the twin; lock-in.

### Option 3: Logpush + APM SDK

* Good, because familiar APM UX.
* Bad, because SDK compatibility in workerd, cost, and logs-only traces.

## Update — interview round 18 (2026-10-18)

Accepted. **Workers Observability plus OTLP export to the existing woven observability stack; PostHog dropped; Gatus probes health/API SLIs** (Q-0130.1–.3). Proposed defaults to confirm during planning: traces sampled 10% (errors 100%), logs 30 days.

## Update — interview round 21 (2026-10-18)

Instrumentation uses the Rust `tracing` ecosystem with OTLP export; verify wasm32 support in spike 0001.

## Update — review fixes, round 25 (2026-10-18)

Outcome rewritten to the accepted decision; the traceparent propagation design is retained as rationale. Instrumentation uses the Rust `tracing` ecosystem (round 21).

## Update — owner decisions and spike results, round 27 (2026-10-18)

Spike 0004: client service-token headers (`CF-Access-Client-Id`, `CF-Access-Client-Secret`) and a client-supplied `Cf-Access-Jwt-Assertion` reach the origin on the bypass paths `/api`, `/auth` and `/live`. Logging and tracing must redact `CF-Access-Client-Secret` (and treat `Cf-Access-*` as untrusted input) before anything is written to Workers Observability or OTLP.

## Open Questions

* Q-0130 — approach.
* Q-0130.1 — Backend for OTLP (Grafana/Honeycomb/Axiom/self-hosted Tempo) and retention/sampling policy.
* Q-0130.2 — Keep PostHog product telemetry? Opt-in policy for the single-org deployment (F-D12).
* Q-0130.3 — Which SLIs/SLOs feed Gatus/status.woven for the new endpoints?

## More Information

CF docs: Workers observability (logs, traces, OTLP export, Logpush, metrics). Related: [[ADR-0002]], [[ADR-0070]], [[ADR-0110]], [[ADR-0121]].
