---
status: accepted
date: 2026-10-17
decision-makers: asmith
consulted: []
informed: []
---

# ADR-0084: `AI` port — Plane AI, agents and MCP

## Context and Problem Statement

Plane CE has OpenAI wired in via env vars (`OPENAI_API_KEY`, `OPENAI_API_BASE`, `GPT_ENGINE`) **[S]**, and the fork's `features.md` lists "Plane AI (Agents, MCP client/app, AI assistant)" as desired **[S]**. Whether AI is in
scope, and how it is abstracted, is open (Q-0084; F-X08). The decision must respect ADR-0002 (swappable between Cloudflare and self-hosted) and tenant data boundaries (customer content going to a model provider).

## Decision Drivers

* Provider portability (OpenAI-compatible APIs, Anthropic, Workers AI, self-hosted models).
* Data-governance: which tenant data may leave, logging/retention of prompts.
* MCP: expose Plane as an MCP server (tools over work items) and optionally consume MCP servers.
* Cost control and rate limiting per tenant.

## Considered Options

* Option 1: **Thin `AI` port** (`complete`, `embed`, `moderate`) with adapters: Workers AI / AI Gateway-routed providers on CF; OpenAI-compatible HTTP on self-hosted
* Option 2: **Provider-direct only** (OpenAI-compatible endpoint configured by env, no CF-specific adapter), like Plane CE today
* Option 3: **Agent runtime in the port**: Cloudflare Agents SDK (DO-based agents) on CF and a self-hosted agent loop — port covers agent sessions/tools
* Option 4: **Out of scope for first release**; reserve the port name and event hooks only

## Decision Outcome

**Decision (accepted, rounds 18 and 19): AI is default-off with a per-workspace opt-in before any content reaches a model provider; P0 exposes only the public API; the MCP server (P1) is a separate stateless driving adapter; the assistant (P2) uses the thin `AI` port with AI Gateway for both adapter sets (Option 4 for P0, Option 1 as the later shape).** Rationale from the original hypothesis — AI is not on Plane CE's critical path, and the port is trivial to add later if
events/permissions seams exist. MCP *server* exposure (Plane tools to external agents) is a separate, likelier-earlier item and does not need the `AI` (model) port at all — it is another driving adapter over the
use-case layer.

### Consequences

* Good: avoids premature coupling to a model vendor; AI Gateway can later give routing, usage/cost tracking and logging for all model calls (CF reference architecture).
* Bad: deferral risks AI features arriving as ad-hoc fetch calls; mitigate with a lint rule against direct model-provider URLs outside the adapter.
* Neutral: current Cloudflare docs mark the stateful `McpAgent` class as deprecated/feature-frozen in favor of stateless handlers (`createMcpHandler`) — if an MCP driving adapter is built on Workers, prefer the stateless
  handler; verify at implementation time.

### Confirmation

* If P1: port contract tests with a fake model (deterministic) across both adapters; tenant-policy test that disallowed tenants never trigger outbound calls.
* MCP adapter: tool schema snapshot tests and authz tests using the permission seam (ADR-0051).

## Pros and Cons of the Options

### Option 1: Thin AI port

* Good: small, testable, portable across providers. * Bad: lowest-common-denominator features (tool calling, streaming nuances).

### Option 2: Provider-direct

* Good: parity with current CE, zero abstraction. * Bad: no governance hook; CF adapter gains nothing from AI Gateway.

### Option 3: Agent runtime in port

* Good: enables agents/assistant features. * Bad: heavy, couples to the Agents SDK/DO model; no self-hosted equivalent without building one.

### Option 4: Defer

* Good: scope control. * Bad: stakeholder expectation risk (`features.md` lists AI).

## Update — interview round 7 (2026-10-17): phasing

The owner phased AI access: **P0 — API-based access** (read here as: the public API, ADR-0031, is the AI/agent surface at P0 with no AI features inside the product — *confirm*, Q-0084.3); **P1 — MCP server access** (Plane exposed as an MCP server; note Cloudflare marks `McpAgent` deprecated in favour of stateless `createMcpHandler`); **P2 — AI Assistant via Cloudflare AI Gateway**. So the AI port's Cloudflare adapter is **AI Gateway** (model/provider routing, caching, logging) — a self-hosted adapter is still needed behind the port (Q-0084.4). Data-governance policy for sending content to model providers (Q-0084.1) must be settled before P2. The "Plane as MCP server" vs "AI assistant" distinction (Q-0084.2) is answered: they are separate phases (P1 vs P2).

## Update — interview round 18 (2026-10-18)

Self-hosted AI adapter (Q-0084.4): **AI Gateway for both the Cloudflare and self-hosted sets.** Consequence: the self-hosted twin needs outbound access to Cloudflare for AI features (P2 only). Data-governance policy (Q-0084.1) and MCP-vs-assistant distinction (Q-0084.2) are not yet decided.

## Update — interview round 19 (2026-10-18)

Accepted. **Default-off; a per-workspace opt-in is required before any content reaches a model provider** (Q-0084.1). The **MCP server (P1) is a separate stateless API surface** reusing API keys and the `Authorizer`; the **assistant is P2** (Q-0084.2). Adapter: AI Gateway for both sets (round 18).

## Update — review fixes, round 25 (2026-10-18)

Decision line now states the accepted outcome (the original rationale is kept). Substance unchanged.

## Open Questions

* Q-0084 — AI scope and shape (decision-pending).
* Q-0084.1 — data-governance policy for sending workspace content to model providers (per-tenant opt-in? redaction?).
* Q-0084.2 — is "Plane exposed as an MCP server" a P0/P1 feature distinct from "Plane AI assistant"?

## More Information

* Plane: `.env.example` (`OPENAI_*`, `GPT_ENGINE`); `features.md`.
* CF: Agents MCP <https://developers.cloudflare.com/agents/tools/mcp/>, McpAgent API <https://developers.cloudflare.com/agents/model-context-protocol/apis/agent-api/>, AI Gateway (reference architecture) <https://developers.cloudflare.com/reference-architecture/diagrams/ai/enterprise-ai-agent-workspace/>.
* Related: [[ADR-0002]], [[ADR-0051]], [[ADR-0083]], [[ADR-0085]].
