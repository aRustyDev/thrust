---
status: proposed
date: 2026-10-17
decision-makers: asmith
consulted: []
informed: []
---

# ADR-0091: Self-hosted twin — reference deployment and default adapter set

## Context and Problem Statement

ADR-0002 requires every Cloudflare primitive to have a self-hosted adapter, switchable by config/compile plus at most
a redeploy. We need a concrete, tested *reference* self-hosted deployment so the second adapter set is real, not
aspirational. Today's Plane ships `docker-compose.yml` (web, admin, space, api, worker, beat, migrator, live, proxy,
postgres 15.7, valkey 7.2, rabbitmq 3.13, minio) plus `deployments/{aio,cli,kubernetes,swarm}`; the `kubernetes` folder
holds only a README **[S]**. The woven environment already runs Kubernetes (woven-o11y EKS, k3s) and an existing
`projects.woven` instance **[O]**.

## Decision Drivers

* Same application bundle, different adapters (no forked code paths).
* Reuse of Postgres/Valkey/RabbitMQ/MinIO knowledge already in the fork; minimal new operational surface.
* Must host a workflow engine substitute (Q-0073.4) and a collab server (Hocuspocus+Redis) behind the same ports.
* Realistic for the woven cluster (K8s) yet runnable on a laptop.

## Considered Options

* Option 1: Docker Compose reference stack (evolve today's compose) — K8s manifests derived later
* Option 2: Kubernetes-first (Helm chart/Kustomize) as the reference; Compose is a dev convenience
* Option 3: Single-process "lite" distribution (Node/Bun server + embedded SQLite/files) plus Compose for scale-out

## Decision Outcome

Decision pending — see Q-0091. Leading hypothesis: **Option 1 with a Kubernetes overlay** (confidence: medium):
Compose is the cheapest place to prove adapter parity in CI (testcontainers/compose), and the same images deploy to the
woven cluster with a thin overlay. Option 3 remains a possible *additional* adapter set (SQLite/in-process queue) that
doubles as the fast test target.

### Consequences

* Good, because CI can run the full contract suite (ADR-0121) against real RabbitMQ/Postgres/Valkey/MinIO.
* Good, because operators of Plane CE recognise the shape.
* Bad, because Compose is not production-grade HA; K8s overlay is extra work.
* Neutral: workflow substitute decision (Q-0073.4) is on the critical path for the twin.

### Confirmation

A CI job brings up the reference stack, runs the port-contract suite and a smoke e2e; failing parity blocks merge
(ADR-0100/0121).

## Pros and Cons of the Options

### Option 1: Compose reference

* Good, because lowest effort, familiar, CI-friendly.
* Bad, because no HA story; K8s mapping still needed.

### Option 2: Kubernetes-first

* Good, because matches woven's runtime; supports HA and admission policies (signed images, see ADR-0101).
* Bad, because heavy for contributors; slow CI loop; the repo currently has no chart.

### Option 3: Single-process lite

* Good, because zero external deps; ideal for dev and edge cases; shares code with test fixtures.
* Bad, because a second persistence dialect to keep in parity; not a production target.

## Update — interview round 21 (2026-10-18)

The self-hosted server is a **native Rust binary** (axum/tokio) built from the same crates, not a Node server; Docker Compose remains the reference.

## Open Questions

* Q-0091 — reference target.
* Q-G7 — Kubernetes vs Compose requirement.
* Q-0073.4 — workflow-engine adapter for the twin.
* Q-0091.1 — Is a private-network (`.woven`) deployment of the twin expected, i.e. does it need external-dns two-leg hostname handling or is it public/Access-fronted?

## More Information

`deployments/` and `docker-compose.yml` in the Plane tree **[S]**. Related: [[ADR-0002]], [[ADR-0060]], [[ADR-0070]], [[ADR-0071]], [[ADR-0090]], [[ADR-0100]].
