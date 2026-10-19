---
status: accepted
date: 2026-10-17
decision-makers: asmith
consulted: []
informed: []
---

# ADR-0071: `WorkflowEngine` port — Cloudflare Workflows adapter and a self-hosted engine

## Context and Problem Statement

Some Plane background work is *workflow-shaped*: multi-step, long-running, retried per step, sometimes sleeping — workspace/issue/analytics
exports (`export_task.issue_export_task`, `analytic_plot_export`), workspace seeding, importers, backfills
(`issue_version_sync.schedule_issue_version` re-queues itself with `countdown=300` — a hand-rolled workflow) **[S]**. Cloudflare Workflows
provides durable step execution. ADR-0002 requires a self-hosted adapter. The ADR must decide the port shape (accepted) and analyse the
options for the **self-hosted engine** (the decision of which engine is recorded as an open sub-question with a leading hypothesis).

## Decision Drivers

* Application code defines a workflow once (steps with retry/timeout/sleep), runs on either engine.
* Step results small and serializable; large outputs go to `BlobStore`.
* Idempotent steps (engines retry).
* Self-hosted engine must be operable by a small team on existing infra (Postgres, RabbitMQ/Valkey already present).

## Considered Options

Port decision (accepted): one narrow `WorkflowEngine` port.

* Option A (CF adapter): Cloudflare Workflows.

Self-hosted engine options (analysed here; selection tracked in Q-0071.1):
* Option B: **Postgres-backed step runner** (own small library: workflow instance + step-result tables, claim via `SELECT … FOR UPDATE SKIP LOCKED`, timers via a scheduler)
* Option C: **Temporal** (self-hosted cluster)
* Option D: **Celery-equivalent / queue-chained** steps over the `Queue` port (each step = message; state in DB)
* Option E: Embedded engine libraries (e.g. Inngest/Trigger.dev/DBOS-style Postgres durable execution)

## Decision Outcome

Chosen (accepted): **a narrow `WorkflowEngine` port with a Cloudflare Workflows adapter**, plus a self-hosted adapter whose engine is chosen via
Q-0071.1. Source: interview round 3 ("Celery vs Workflows" as swappable interface).

```rust
// crate: thrust-ports (round 25: Rust sketch; replaces the earlier TypeScript sketch)
#[async_trait]
pub trait WorkflowEngine: Send + Sync {
    async fn start(&self, def: WorkflowName, input: serde_json::Value, opts: StartOpts) -> Result<WorkflowHandle, WfError>;
    async fn status(&self, id: &WorkflowId) -> Result<WorkflowStatus, WfError>;
    async fn pause(&self, id: &WorkflowId) -> Result<(), WfError>;
    async fn resume(&self, id: &WorkflowId) -> Result<(), WfError>;
    async fn terminate(&self, id: &WorkflowId) -> Result<(), WfError>;
}
// Definitions are written against a step context:
//   step.run(name, StepOpts { retries, timeout }, |input| async { … }) -> Result<Output>   // result memoised, must be serialisable
//   step.sleep(name, Duration)
//   step.wait_for_event(name, EventSpec { kind, timeout })
// Step closures are `'static` and own their inputs (they cross replays); they do not borrow the step context.
```

Leading hypothesis for the self-hosted engine: **Option B** (confidence: medium) — smallest operational footprint, state already in
Postgres, and the required feature subset (`do`/`sleep`/`waitForEvent`/retry) is small. Temporal is the fallback if the workflow
catalogue grows.

### Consequences

* Good, because CF Workflows gives durable steps, `step.sleep` up to 365 days, `waitForEvent`, retries per step (up to 10,000), 10,000 steps
  per instance by default (configurable to 25,000), 1 MiB per step result, 1 GB persisted state per instance on Paid, retention 30 days Paid, 50,000
  concurrent running instances, 300 creations/s per account (CF Workflows limits). `waiting` instances don't count toward concurrency.
* Good, because the port subset (`do`, `sleep`, `waitForEvent`) is implementable on a Postgres runner.
* Bad, because per-step CPU defaults to 30 s (raisable to 5 min via `limits.cpu_ms`) — large exports must stream via R2 and stay chunked.
* Bad, because the self-hosted engine is net-new code (Option B) or a new service (Option C).

### Confirmation

* Contract suite (ADR-0121): step retry with backoff, step result memoization on replay, `sleep` resume after restart, `waitForEvent` delivery,
  terminate semantics — on Workflows (workerd/Miniflare harness, exercised from the Rust wasm32 test leg or a shim Worker) and the self-hosted engine.
* A reference workflow (export a 100k-work-item project to CSV in R2) must complete on both engines within stated limits.

## Pros and Cons of the Options

### Option A: Cloudflare Workflows

* Good, because managed, durable, integrates with Queues/R2/DO.
* Bad, because only available on Cloudflare; 1 MiB step-result and CPU-per-step limits shape designs.

### Option B: Postgres-backed runner

* Good, because no new infra; transactional with application data; easy to test.
* Bad, because we own correctness (timers, leader election, replay determinism).

### Option C: Temporal

* Good, because battle-tested durable execution, rich features (signals, queries, versioning).
* Bad, because heavy to operate (cluster + persistence + workers), different programming model (deterministic replay), overkill for ~6 workflows.

### Option D: Queue-chained steps

* Good, because reuses `Queue` port and RabbitMQ.
* Bad, because re-invents orchestration state; error handling across steps is bespoke; hard to give identical semantics to CF Workflows.

### Option E: Embedded durable-execution library

* Good, because less code than B.
* Bad, because adds a third-party dependency whose semantics must match CF Workflows; licensing and maintenance risk (AGPL/clean-room posture, Q-G6).

## Update — interview round 17 (2026-10-17)

Self-hosted adapter (Q-0071.1, Q-0073.4): **a Postgres-backed step runner inside the native server; no Temporal.** Workflows are P1, so this is not needed at P0. (Wording corrected in round 25: the native server is the Rust binary, ADR-0091; "Node server" in earlier text is obsolete.)

## Update — interview round 21 (2026-10-18)

Workflows from Rust: the workers-rs docs do not mention a Workflows binding (spike 0001 verifies). A narrow JS shim Worker or REST fallback is the expected bridge; the self-hosted Postgres-backed runner is written in Rust.

## Update — review fixes, round 25 (2026-10-18)

The TypeScript port sketch is replaced by a Rust trait sketch (ADR-0006). Self-hosted engine decision stands: **Option B, a Postgres-backed step runner in the native Rust binary** (Q-0071.1 answered round 17). The self-hosted P0 adapter set (SPEC R-P3, round 25) does not include a workflow engine — Workflows are P1.

## Update — spike results, round 26 (2026-10-18)

The `?Send` sketch above is corrected (ADR-0002).

**The Cloudflare adapter is a bounded non-Rust shim** (ADR-0006, round 22 policy). workers-rs 0.8.7 has **no Workflow API** and cannot define a Workflow class (`WorkflowEntrypoint` is JavaScript). Spike 0001 (`[tested]` locally, `worker_bindings_workflow_via_wasm_bindgen_and_js_shim`): Rust creates and polls an instance through raw `js_sys::Reflect` calls on `env.WORKFLOW`; the Workflow class lives in a 26-line JS file, `crates/thrust-worker/entry.mjs`; the two-step workflow called back into the Rust Worker through a service binding and ended `complete` with the expected output. **Shim contract:** `start` and `status` are called from Rust; step bodies call back into Rust over a service binding; no business logic lives in the JS. **Exit plan:** delete the shim when workers-rs ships a Workflow API. Pause, resume and terminate through the shim, the real engine's limits and retries are not exercised (Stage 2 step S2-6). Workflows are not used at P0.

## Open Questions

* Q-0071.1 — **answered (round 17)**: Option B (Postgres-backed runner, native Rust server).
* Q-0073.3 — export/long-job limits on Workflows for large workspaces.
* Q-0073.4 — self-hosted workflow adapter (same as Q-0071.1; keep both linked).

## More Information

* Plane: `bgtasks/export_task.py`, `analytic_plot_export.py`, `workspace_seed_task.py`, `issue_version_sync.py`.
* CF: Workflows limits <https://developers.cloudflare.com/workflows/reference/limits/>.
* Related: [[ADR-0002]], [[ADR-0070]], [[ADR-0072]], [[ADR-0073]], [[ADR-0081]].
