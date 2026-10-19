---
status: accepted
date: 2026-10-17
decision-makers: asmith
consulted: []
informed: []
---

# ADR-0060: Yjs collaboration behind a `Collab` port (Durable Objects adapter + Hocuspocus/Redis adapter)

## Context and Problem Statement

Plane's collaborative editing runs in `apps/live`: a Node/Express server using Hocuspocus + Yjs, with a Redis extension for
multi-instance fan-out and a `force-close` handler **[S]**. Persistence is a thin HTTP shim over Django
(`extensions/database.ts` → `fetchDescriptionBinary(pageId)` via `services/page/*`); on store it writes
`description_binary`, `description_html`, `description_json` together **[S]**. The kept frontend (`@plane/editor`) speaks the
Hocuspocus/Yjs wire protocol. ADR-0002 requires every Cloudflare primitive to sit behind a port with a self-hosted twin.
The interview ([I] round 3) fixed: "Pages editor collab on Durable Objects (Yjs)" **and** "DO vs Redis + Hocuspocus" must be
a config/compile + ≤ redeploy switch.

## Decision Drivers

* Keep `@plane/editor` and existing Yjs binaries unchanged (hybrid approach, ADR-0001).
* Must run on Cloudflare **and** self-hosted with the same application code (ADR-0002).
* One room per document with serialized updates — Durable Objects map naturally to this.
* Auth must go through the permission seam (ADR-0051), not re-implemented in the collab server.
* Hibernation to keep idle-room cost near zero.

## Considered Options

* Option 1: `Collab` port with two adapters — Durable Object (Yjs, hibernating WebSockets) and Hocuspocus + Redis
* Option 2: Hocuspocus everywhere (run Hocuspocus Node server for both; CF deployment keeps a container/VM)
* Option 3: DO-only collab, no self-hosted twin (violates ADR-0002)
* Option 4: Replace Yjs with another CRDT/OT (Automerge, Liveblocks, PartyKit-style hosted)

## Decision Outcome

Chosen option: **Option 1**, because it satisfies the interview constraint (CF-native by default, self-hostable by config)
and keeps the editor's Yjs document format unchanged (the wire *framing* needs the Hocuspocus caveat below). Source: interview round 3, answer to "CF primitives" question.

The port is *transport + room* oriented, not "Yjs library" oriented:

```rust
// Illustrative Rust sketch, not final signatures.
pub trait CollabPort {
    /// driving side: accept an upgraded socket for a document room after authorization
    async fn connect(&self, doc: DocId, session: AuthenticatedSession, socket: CollabSocket)
        -> Result<(), CollabError>;
}
pub trait CollabPersistence {   // implemented by core on top of Database/BlobStore ports (ADR-0061)
    async fn load(&self, doc: DocId) -> Result<Option<Vec<u8>>, StoreError>;
    async fn store(&self, doc: DocId, update: DocUpdate /* binary + html + json */) -> Result<(), StoreError>;
}
pub trait CollabAuthorizer {
    async fn authorize(&self, session: &AuthenticatedSession, doc: DocId, mode: AccessMode)
        -> Result<bool, AuthzError>;
}
```

**Protocol caveat (round 25).** The kept editor client connects with `HocuspocusProvider` (`packages/editor/src/core/hooks/use-yjs-setup.ts`), i.e. Hocuspocus framing (document name, auth message, stateless messages), not bare y-protocol. A collab Durable Object must therefore **speak Hocuspocus framing, or the editor's collab provider must change** — the one allowed React change area "collab provider" (Q-R2). Also, `apps/live` today serves only **project pages** (`project_page`), not work-item descriptions, so descriptions over the collab path are new scope, not parity; and it carries extra duties the port must account for: the `title-sync` extension (page titles over stateless messages), the admin `force-close` command over Redis, a 413 'content too large' force-close path, and the PDF-export controller.

### Consequences

* Good, because the editor and awareness protocol stay untouched; adapters differ only in hosting.
* Good, because DO gives one single-threaded room per document and WebSocket Hibernation (idle rooms cost no duration charges).
* Bad, because two adapters must be kept behavior-identical — mitigated by the shared contract suite (ADR-0121).
* Bad, because DO has no Redis-style cross-instance pub/sub need (a room lives in exactly one DO) while Hocuspocus needs Redis; the
  *topology* differs even if the port is the same.
* Neutral: Yjs doc state lives where ADR-0061 says (open).

### Confirmation

* Contract test (ADR-0121): two simulated clients converge, awareness propagates, unauthorized socket is refused, persistence
  `store` receives binary+html+json — run against both adapters.
* Compatibility test: a Yjs binary produced by current Plane (`description_binary`) loads and re-serializes losslessly.

## Pros and Cons of the Options

### Option 1: Port + DO adapter + Hocuspocus/Redis adapter

* Good, because DO limits fit: SQLite-backed DO storage up to 10 GB per object; hibernation API keeps sockets while evicted;
  per-socket `serializeAttachment` ≤ 16 KiB; WebSocket message limit raised to 32 MiB (CF changelog 2025-10-31).
* Good, because the self-hosted adapter reuses current `apps/live` code.
* Bad, because hibernation resets in-memory Yjs state — the DO must reload from storage on wake (cold-start cost; Q-0061.1).

### Option 2: Hocuspocus everywhere

* Good, because only one implementation.
* Bad, because a CF deployment would need a non-Workers host (Containers/VM), defeating "Cloudflare-native" and adding a stateful tier.

### Option 3: DO-only

* Good, because simplest CF design.
* Bad, because violates ADR-0002 and strands self-hosters.

### Option 4: Different collaboration model

* Good, because could remove DO/Redis coupling.
* Bad, because forces a rewrite of `@plane/editor` internals and a one-time data conversion; contradicts ADR-0001.

## Update — interview round 6 (2026-10-17): phasing

The port decision stands, but **neither the Durable Object adapter nor the Hocuspocus adapter ships at P0**: page and work-item-description editing is a plain API save at P0 (Q-F5), and the DO adapter arrives with multi-user collaboration in P1. The `Collab` port and its contract tests are still defined at P0 only if cheap; otherwise define the port at the start of P1. Storage format chosen at P0 must not force a data migration at the P1 switch (Q-F5.1). Self-hosted collab adapter is P1 as well (Q-F6).

## Update — interview round 13 (2026-10-17)

Sizing (Q-G4.1): design for **up to 50 concurrent editors per page**; the DO spike must stress-test connection count, snapshot cadence and DO limits.

## Update — interview round 19 (2026-10-18)

Collab WebSocket auth (Q-0060.1): the API mints a **short-lived signed token** presented on the DO upgrade; works with or without Access in front.

## Update — interview round 21 (2026-10-18)

Collab Durable Object in Rust with `yrs` (Yjs-binary-compatible) is the candidate; spike 0005. Self-hosted adapter can be Hocuspocus+Redis (TS) or a Rust y-sweet/axum+yrs service (Q-0061.2).

## Update — review fixes, round 25 (2026-10-18)

Replaced the TypeScript interfaces with Rust trait sketches and recorded the Hocuspocus-framing caveat and the `apps/live` scope facts verified in the review. Decision unchanged.

## Update — spike results, round 26 (2026-10-18)

Spike 0001 and spike 0004 findings that touch the collab Worker (`[tested]` locally unless noted): **Durable Object classes must be exported from the same Wasm module as `fetch`**, so the collab DO needs its own Worker root (or a re-export), see ADR-0090. WebSocket hibernation is native in workers-rs: after 14 s idle the object was re-instantiated while the socket stayed open and its SQLite rows and socket attachment survived (local timing only). **Binary frames arrive as `Blob`** with compat date 2026-03-17 or later, so set `ws.binaryType = "arraybuffer"` (hibernation handlers always receive `ArrayBuffer`): relevant to Yjs sync frames. **Spike 0004 run 1, real Cloudflare edge, no Access:** text, binary, 64 KiB and 1 MiB echoes and a WebSocket through a service binding all worked, but a **plain-Worker WebSocket closed before 80 s although a message was echoed every 20 s** (cause unknown; Q-0060.2), so a reconnect-on-close client is required either way (runtime updates alone give existing connections up to 20 minutes). Worker-level Access answers WebSocket upgrades with 403 `[doc]`; use hostname-based apps or the bypass path (ADR-0050).

## Update — owner decisions and spike results, round 27 (2026-10-18)

Spike 0004 run 2: the unexplained 80 s drop of a plain-Worker WebSocket in run 1 was **not reproduced** — two 120 s soaks on the bypass path (a message every 20 s) passed, and WebSockets worked on a gated path with a service token and across a service-binding hop. Q-0060.2 stays open at low priority; a reconnect-on-close client is still required.

## Open Questions

* Q-0061.1 — presence limits per DO, max doc size, cold-start behavior.
* Q-0061.2 — can the self-hosted adapter reuse Hocuspocus + Redis unchanged behind this port?
* Q-0060.1 — how is the editor's auth token delivered to the DO upgrade request (cookie vs ticket/JWT) given Cloudflare Access in front?

## More Information

* Plane: `apps/live/src/hocuspocus.ts`, `extensions/database.ts`, `controllers` (collaboration, document, health, PDF export).
* CF: Durable Objects WebSocket Hibernation <https://developers.cloudflare.com/durable-objects/best-practices/websockets/>.
* Related: [[ADR-0002]], [[ADR-0051]], [[ADR-0061]], [[ADR-0121]].
