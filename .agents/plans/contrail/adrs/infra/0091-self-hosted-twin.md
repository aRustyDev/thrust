---
status: accepted
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

**Decision (accepted, round 29): one binary, one database, one object store; a Compose file with at most three
services. Per FR-19, `reviews/2026-10-19-fable-architecture-review.md` §5.**

This is Option 1 (a Compose reference; Kubernetes uses the same images, Q-G7) with Option 3's single-process shape,
in Rust rather than Node/Bun: the native binary holds the API, the outbox queue and its sweeper, and the in-process
scheduler, so no broker, cache or worker process runs beside it. Option 3's SQLite comes back only if spike 0002's
`worker-do` cell wins (FR-04); see the round-29 note.

### Consequences

* Good, because CI runs the full contract suite (ADR-0121) against the binary on real Postgres: the `native-pg` cell.
* Good, because operators of Plane CE recognise the shape.
* Bad, because Compose is not production-grade HA; K8s overlay is extra work.
* Neutral: no workflow substitute is needed (FR-05; ADR-0071, round 29), so Q-0073.4 no longer gates the twin.

### Confirmation

The `native-pg` contract cell runs the port-contract suite against the binary on Postgres, and on every Rust-relevant
pull request `gate` blocks merge unless every planned cell passes (`docs/src/architecture/cicd.md:21-28`, `:47-50`;
ADR-0100/0121).

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

## Update — review round 29 (2026-10-19)

Per FR-19 and FR-05 (`reviews/2026-10-19-fable-architecture-review.md`; the owner's decision is its §5 table: apply now, local-disk blob store at P0, S3 adapter at P1), this ADR is accepted as "one binary, one database, one object store; a Compose file with at most three services". The P0 twin is: Postgres, or SQLite on `rusqlite` if spike 0002's `worker-do` cell wins (FR-04); the outbox as queue, with the in-process fast path and the sweeper in a single process (ADR-0070, round 29); a local-disk `BlobStore`, with the S3-compatible adapter for MinIO at P1 (ADR-0080, round 29); SMTP through `lettre`; and the in-process timer as scheduler (ADR-0072). Sessions are rows in the database (FR-03). RabbitMQ, Valkey, Hocuspocus and the separate worker process are cut (FR-05), and the P1 collab room runs inside the binary (ADR-0060, round 29).

Why: for 50 users or fewer none of RabbitMQ, Valkey, a durable-execution engine or a Node collab server earns its keep (FR-05). What runs "in parallel from Phase 0" is exactly the `native-pg` contract cell that already runs in CI; nothing else is built per port until a self-hoster asks (FR-19). SPEC R-P3, SPEC §8 and FEATURES F-X02 say the same (round 29).

Today's server is not yet this twin (`docs/src/architecture/infra.md:86-99`): in `postgres` mode only the database changes, and sessions, blobs, the rate limiter and the queue stay in process memory, so a restart loses sessions and attachments. The database-backed sessions (FR-03) and the local-disk `BlobStore` close that gap; neither is built yet. Outbox rows are already in Postgres, so the sweeper delivers what the in-process queue lost.

This supersedes the third decision driver (a workflow-engine substitute and a Hocuspocus+Redis collab server behind the same ports), the RabbitMQ, Valkey and MinIO stack in the Consequences, and the reference-stack CI job in the Confirmation, which is now the `native-pg` cell. Q-0091 is answered.

## Open Questions

* Q-0091 — reference target. (answered round 29, FR-19)
* Q-G7 — Kubernetes vs Compose requirement.
* Q-0073.4 — workflow-engine adapter for the twin. (superseded round 29, FR-05)
* Q-0091.1 — Is a private-network (`.woven`) deployment of the twin expected, i.e. does it need external-dns two-leg hostname handling or is it public/Access-fronted?

## More Information

`deployments/` and `docker-compose.yml` in the Plane tree **[S]**. Related: [[ADR-0002]], [[ADR-0060]], [[ADR-0070]], [[ADR-0071]], [[ADR-0090]], [[ADR-0100]].
