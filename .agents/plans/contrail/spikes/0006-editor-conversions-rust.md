# Spike 0006 — Editor data model without a JS runtime (html ⇄ json ⇄ Yjs binary)

**Gates:** ADR-0012, ADR-0061, ADR-0013.
**Questions:** Q-0012.1, Q-F5.1, Q-F5.2, Q-0061.4.
**Timebox:** 4 working days. **Depends on:** spike 0001. **Owner:** TBD. **Tier:** P0 gate for ADR-0012 (round 25: P0 stores client-produced forms verbatim, Option B; conversion needed by P1 collab). Run right after spike 0001.

## Question
Plane stores each rich-text field as Yjs binary + HTML + JSON (round 11). Today the JS editor package produces/consumes
these. In a Rust backend, who converts, and can it be done without TipTap/ProseMirror in the server?

## Options
- **A. Rust implementation of the schema** (ProseMirror JSON ↔ HTML ↔ `yrs` XmlFragment) for the node/mark set in
  `@plane/editor`; client always sends all three forms at P0, server validates/sanitises.
- **B. Client-produced forms only at P0** (server stores what the editor sends, sanitises HTML), conversion service only
  when a non-editor writer exists (API v1 writes HTML).
- **C. JS conversion Worker** (bundle `@plane/editor` extensions headless) service-bound from the Rust API.

## Method
1. Inventory every node/mark/extension in `packages/editor` and which forms the API v1 and the app accept/emit.
2. Prototype A for a representative subset (paragraph, headings, lists, code, mentions, embeds, tables); round-trip a corpus
   of real Plane pages (from the pinned oracle instance with synthetic content) and diff.
3. Prototype C in workerd: does the headless editor run without DOM shims (jsdom-free)? bundle size, startup, CPU.
4. HTML sanitisation in Rust (ammonia) vs Plane's current sanitiser; XSS test corpus.
5. API v1 writes (`description_html`) — can the server produce the binary form so collab (spike 0005) can open it?
6. Implication for the **Dioxus editor** (spike 0007): which representation is the source of truth long-term.

## Exit criteria
Round-trip fidelity ≥ 99% on the corpus for supported nodes; sanitiser passes XSS corpus; chosen option runs within
Worker CPU/size limits. If A is too costly, choose C (a documented, bounded exception to "all Rust") and record it.

## Deliverables
ADR-0012 decision; supported-node matrix; Q-0012.1 closed.

## Result (round 27)

**Done.** Option A (Rust) passes at 100 % fidelity against the real JS code; Option B is a valid P0 stance; Option C works but is not recommended as primary. Findings, the supported-node matrix and the effort estimate: `research/0008-editor-conversions.md`; ADR-0012 *Update* has the verdict and the recommendation. Q-0012.1 is answered; the owner accepted ADR-0012 as recommended (Q-0012.3); Q-0012.2 (sanitiser) and Q-0012.4 (browser-flavour renderer) are still open.
