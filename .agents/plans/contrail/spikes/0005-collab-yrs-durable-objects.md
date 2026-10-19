# Spike 0005 — Collaborative editing: Yjs (yrs) on Durable Objects, up to 50 editors per page

**Gates:** ADR-0060, ADR-0061 (P1 collab; plain API save at P0).
**Questions:** Q-G4.1 (50 concurrent editors), Q-0061, Q-0061.1, Q-0061.2, Q-F5.2, Q-0060.1.
**Timebox:** 5 working days. **Depends on:** spikes 0001, 0006. **Owner:** TBD. **Tier:** P1 (run before P1 collab starts).

## Question
Can one Durable Object per page, running the Rust `yrs` CRDT (binary-compatible with Yjs), serve the existing
`@plane/editor` (TipTap/y-prosemirror) with up to 50 concurrent editors, persist snapshots, and produce the three stored
forms (binary, html, json)?

## Method
1. DO (workers-rs `#[durable_object]`) with WebSocket hibernation speaking the y-protocol (sync + awareness) to the
   unmodified `@plane/editor` provider; auth via the short-lived signed token (round 19).
2. Persistence: DO SQLite storage as primary vs periodic snapshot to DB/R2; compaction; version cadence every 10 min of
   edits (round 18); description staleness ≤ 5 s.
3. Load test: 1, 10, 50 simulated editors typing; measure fan-out latency, DO CPU/memory, message rate, cost.
4. Limits: max doc size, connections per DO, cold start/rehydration time, behaviour at DO eviction.
5. Self-hosted adapter: Hocuspocus + Redis unchanged behind the port **or** a Rust y-sweet/axum+yrs service (Q-0061.2).
6. Plain-API-save → collab migration: a page stored at P0 as binary+html+json opens collaboratively with no data
   migration (Q-F5.1).

## Exit criteria
50 editors on one page with p95 update fan-out < 250 ms and no data loss on forced DO eviction; a P0-saved page
opens in collab unchanged; cost within envelope. If hibernating WebSockets or DO limits fail, fall back to sharding
awareness or capping editors (record the cap).

## Deliverables
ADR-0060/0061 decisions; load-test report; Q-G4.1 closed with measured limits.
