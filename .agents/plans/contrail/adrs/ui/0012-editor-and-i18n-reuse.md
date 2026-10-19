---
status: accepted
date: 2026-10-17
decision-makers: asmith
consulted: []
informed: []
---

# ADR-0012: Editor and i18n reuse

## Context and Problem Statement

`packages/editor` (Tiptap/ProseMirror + Yjs helpers, ~21k LOC) and `packages/i18n` are retained under ADR-0001. The editor defines the Yjs document structure and conversions between HTML, JSON and binary; `apps/live` depends on it for those conversions, and the DB stores `description_binary`, `description_html`, `description_json` together [S]. The collab server (ADR-0060) will run in a Durable Object or a native Rust service; with a Rust backend (ADR-0006) it cannot import the editor's JS conversion code, so conversions are decided in spike 0006. Locale ownership and the translation workflow are also unspecified.

## Decision Drivers

* Existing page and work-item description data must stay readable (binary format unchanged).
* The server-side conversion code must run in workerd as well as Node.
* Minimise divergence from upstream (Q-G2).
* Translation workflow stays cheap.

## Considered Options

* Option 1: Reuse `@plane/editor` unchanged; vendor locales from `@plane/i18n` **(chosen; the earlier clause "make its server-side conversions workerd-compatible" is superseded: the server converts in Rust, see Decision Outcome)**
* Option 2: Fork the editor package with Workers-specific entrypoints and fork-owned extensions (e.g. nested pages/embeds from Open-EE)
* Option 3: Replace the editor with a different framework (e.g. BlockNote or Lexical + Yjs binding)

## Decision Outcome

**Decision (accepted, round 27): Option 1 — reuse `@plane/editor` unchanged, evolving to Option 2 (a fork) only where needed — together with the conversion stance recommended by spike 0006:**

* **The Yjs binary format stays unchanged** (round 17): it is the data contract, and replacing the editor would force a content migration.
* **P0 = store the client-produced html, json and binary verbatim** (round 25, Option B), sanitise `description_html` in Rust, and record a schema version. No server-side conversion at P0; API v1 writes of `description_html` store html plus the stripped text, and the Yjs binary is created on first collab open at P1.
* **P1 conversion = a Rust implementation**, `thrust-editor-doc` (`yrs` plus the ProseMirror JSON model), behind an **`EditorDocs` port** (Option A), written to match the real headless JS editor byte for byte and kept honest by an oracle CI that regenerates the corpus at each Plane pin bump.
* **A JavaScript conversion Worker (Option C) is only a documented fallback**, a bounded exception under ADR-0006, to be used only if the owner rejects the oracle-CI cost of Option A.
* **i18n:** reuse `@plane/i18n` as shipped with the adopted React apps (F-D11, inherited). Locale ownership and the translation workflow are not decided here (Q-0012.5).

Still open inside this ADR (they do not change the decision above): **Q-0012.2** (the P0 sanitiser stance: hardened or exact parity with Plane's allow-list, the `issue-embed-component` and `rel` handling, the ammonia version drift, `description_stripped`) and **Q-0012.4** (whether a browser-flavour HTML renderer is needed, and confirmation that the eight behaviours of today's stack are reproduced).

### Consequences

* Good, because existing content works, and the editor investment is retained.
* Bad, because the server needs its own Rust implementation of the conversions: about 16 person-days to finish (range 10 to 25), plus an oracle CI that tracks upstream Plane's schema and rules (the earlier worry that the JS conversions need DOM or Node APIs is retired: spike 0006 found no DOM shim is needed, Option C needs only `nodejs_compat`).
* Neutral: Open-EE editor features (nested pages, embeds) require fork-owned extensions.

### Confirmation

The Rust conversions match the pinned JS code byte for byte on the committed corpus and fuzz seeds (regenerated at each Plane pin bump, a CI gate); the sanitiser passes the 169-payload XSS corpus; the Worker stays within its size and CPU budget; and the real JS editor opens Rust-produced binaries. Spike 0006 met all of these locally (`research/0008-editor-conversions.md`); the Cloudflare measurements are still to be done.

## Pros and Cons of the Options

### Option 1: Reuse unchanged

* Good, because zero data migration.
* Bad, because possible runtime incompatibilities.

### Option 2: Fork with Workers entrypoints

* Good, because control over extensions and runtime support.
* Bad, because merge cost with upstream.

### Option 3: Replace editor

* Good, because a fresh design.
* Bad, because content migration and huge UI rework.

## Update — interview round 17 (2026-10-17)

Direction: **keep the editor's Yjs binary format unchanged** (Q-0012). Left `proposed` because whether HTML/JSON/binary conversion runs in workerd without DOM/Node shims (Q-0012.1) is unverified and i18n reuse is not yet decided.

## Update — interview round 21 (2026-10-18)

Owner direction (round 21): **backend in Rust from the start; Plane's React frontend adopted with minimal changes; a Dioxus (Rust) clone of the UI follows when backend and React frontend are stable, for multi-target builds** (ADR-0013). The editor stays `@plane/editor` (TipTap/Yjs) in phase 1. The **server** can no longer assume a JS editor runtime: html/json/binary conversion in Rust is spike 0006 (Q-0012.1). The Dioxus editor strategy is Q-0013.2.

## Update — review decisions, round 25 (2026-10-18)

P0 rich text (round 25): **Option B — store client-produced forms verbatim, sanitise HTML in Rust, version the schema**; no server-side conversion at P0. API v1 writes of `description_html` store html (+ stripped text); the Yjs binary is created lazily on first collab open at P1. The server never rewrites stored binary before collab ships. Spike 0006 runs right after 0001 so this ADR can be accepted.

## Update — spike 0006 results, round 27 (2026-10-18)

**Spike 0006 result (`research/0008-editor-conversions.md`, bead `plane-byd.6`; thrust branch `feat/spike-0006-editor-conversions`, two signed commits, local).** Everything is local (native and local workerd, never Cloudflare's network); the oracle is the **real headless `@plane/editor` code** from the pinned tree. The owner accepted the recommendation below in round 27 (Q-0012.3, see the next update).

| Option | Verdict | Evidence |
|---|---|---|
| **A** Rust (`thrust-editor-doc`, `yrs` plus the ProseMirror JSON model) | **Pass; recommended for P1 conversion and for the P0 sanitiser** | 263 documents, 1,337 of 1,337 (document, direction) pairs identical to the JS oracle, every one of the 23 nodes and 8 marks at 100 %; 6,000 fuzzed HTML inputs times 6 directions identical (including a 4,000-input held-out set); Yjs structure identical to y-prosemirror on 218 of 218; JS-written binaries (40 with edit history, 2-client) decode 263 of 263; the v1 write path (HTML to all formats) 67 of 67 |
| **B** client forms verbatim at P0 | **Pass as the P0 stance**, with three facts to record (below) | needs only the Rust sanitiser; no conversion |
| **C** JS conversion Worker | Technically passes; **not recommended as primary** | runs in workerd with no DOM shim but needs `nodejs_compat` (`@plane/constants` reads `process.env` at load); 91 ms startup profile and 2 to 50 times the CPU of A; adds a JavaScript toolchain and a pinned Plane tag |

* **Controls that must fail do fail:** 10 of 10 schema mutants caught (the corpus alone caught 7 of 10 until legacy binaries were added), 0 cross-document false matches, and Rust and Yjs agree on corrupted-binary detection in 1,170 of 1,170 cases.
* **The real JS editor opens Rust-produced binaries:** a TipTap editor with the collaboration extension (jsdom) gives the same `getJSON` and `getHTML` as for the oracle's binaries (223 of 223, and 67 of 67 for the v1 path). So a v1 write of `description_html` can produce a binary that collab opens.
* **Q-0012.1 answered:** no DOM shim is needed; Option C needs `nodejs_compat` or a build-time define.
* **Sizes and CPU (local workerd):** Option A is 2.61 MiB wasm (1.98 MiB without the sanitiser, 930 KB gzip), 18 ms startup; a typical work-item write takes 0.37 ms, a page projection 0.09 to 0.5 ms, a 745 KB document 118 ms. Projected CPU at the sizing target: about $0.005/month for A and $0.018/month for C, both inside the included allowance.
* **Three facts for Option B (P0):** work-item descriptions carry **no Yjs binary** today (v1 returns it null in 64 of 64 objects); client-produced HTML differs from server-produced HTML in **37 % of corpus documents** (`style` attributes only in the browser flavour 74, boolean attribute form 27, attribute-name case 14, entity quoting 7); client JSON carries block `id`s from the `UniqueID` extension that the headless server projections drop.
* **Sanitiser:** Plane's nh3 0.2.18 (a binding to ammonia 4.0.0) and Rust ammonia with the same allow-list each neutralise **116 of 125** executable payloads and are byte-identical on 430 of 430 inputs; the 9 survivors (CSS `url()`, `expression()`, `td background`, `data-emoji-url`) are legacy-browser or CSS-injection class and **probably not exploitable in evergreen browsers** `[inferred]`. A **hardened** Rust variant (filters dangerous `style`, `background` and `data-emoji-url` values) neutralises 125 of 125 with **no change** to 195 of 195 editor-generated documents. Plane's allow-list also strips `issue-embed-component` and rewrites `a rel` to `noopener noreferrer` (66 of 195 editor documents change, for those two reasons only). Ammonia 4.2.4 differs from 4.0.0 on 7 of 430 inputs (a security improvement: it escapes `<` and `>` in attributes). `strip_tags` (behind `description_stripped`) is ported with 391 of 391 identical to Django's.
* **Robustness:** 31,440 mutated Yjs binaries and 6,000 damaged HTML inputs gave 0 panics `[tested, native]` (evidence, not proof). **The Rust Worker dies with a stack overflow, an instance reset, at about 1,000 nesting levels**, so P1 needs an explicit depth cap (say 200) and a size cap before parsing.
* **Unsupported or lossy:** browser-flavour HTML (only the server flavour is rendered), Yjs update v2, version-diff and subdocument features, exponent-form number formatting, `mark--hash` attributes are read but never written.
* **Eight behaviours of today's stack are reproduced on purpose** (research/0008 section 5; each is a one-line switch): server `generateHTML` drops every `style` attribute; server HTML to JSON never recovers a code block's `language`; every `<span>` becomes a `customColor` mark; `colwidth="100,200"` parses to `[100]`; numeric-looking attribute strings become numbers; server projections drop the client's block `id`s; the sanitiser strips `issue-embed-component`; nh3 rewrites `rel`. They are quirks of the stack, so the default is **reproduce** (round 27).
* **Effort to finish A:** about 16 person-days (range 10 to 25), mostly oracle CI and parity decisions: guards, error taxonomy and panic audit 3; `EditorDocs` port, adapter, v1 handlers, collab DO integration 3; oracle CI (regenerate the corpus at each Plane pin bump, drift gates, upstream merge process) 3; parity-versus-fix tests 2; browser-flavour mode 3 (optional); residue 1.5; Stage 2 on Cloudflare 1.5.

**Recommendation (accepted by the owner, round 27, Q-0012.3):** P0 = Option B (client forms verbatim, sanitised in Rust, schema version recorded); P1 conversion = Option A behind an `EditorDocs` port implemented by `thrust-editor-doc`; Option C stays a documented, bounded exception to be used only if the oracle-CI cost of A is rejected. Confidence: high on feasibility, medium on the cost of tracking upstream. **Replacement Confirmation** (when accepted): the Rust conversions match the pinned JS code byte-for-byte on the committed corpus and fuzz seeds (regenerated at each pin bump), the sanitiser passes the 169-payload XSS corpus, and the Worker stays within the size and CPU budget.

**Not proven:** anything on Cloudflare (CPU, cold start, memory, Durable Object behaviour); real Plane data and binaries from older versions or other collab servers; Yjs update v2; the XSS checker uses libxml2 and html5lib, not browsers; no concurrent load test; the fuzz generators are the spike's own.

## Update — owner decision, round 27 (2026-10-18)

Owner decision (**Q-0012.3**): **"Accept ADR-0012 as recommended."** The ADR moves from `proposed` to `accepted`. The accepted outcome is stated in *Decision Outcome* above: reuse `@plane/editor` with an unchanged Yjs binary format; P0 stores the client forms verbatim and sanitises in Rust; P1 conversion is the Rust `thrust-editor-doc` behind an `EditorDocs` port, backed by an oracle CI; the JavaScript conversion Worker stays a documented fallback only. The acceptance covers that recommendation, not the two sub-decisions that remain open (Q-0012.2, the sanitiser; Q-0012.4, the browser-flavour renderer), which keep the leading recommendations recorded above (hardened sanitiser, no browser-flavour mode unless P0 needs byte equality with the client's HTML). Follow-ups for planning: build the `EditorDocs` port and `thrust-editor-doc` (about 16 pd), the oracle CI (regenerate the corpus when the Plane pin changes), depth and size caps, and the schema-version column.

## Open Questions

* Q-0012 — editor reuse approach: **answered (round 27)**; i18n ownership: Q-0012.5.
* Q-0061 — collab persistence (binary/html/json triple-write parity).
* Q-0012.1 — **answered (round 27, spike 0006): no DOM shim needed.**
* Q-0012.2 — P0 sanitiser stance (owner, open). Q-0012.3 — **answered (round 27): accepted as recommended.** Q-0012.4 — browser-flavour HTML mode and the eight reproduced behaviours (owner, open). Q-0012.5 — i18n locale ownership and translation workflow (open, not blocking).

## More Information

Related: [[ADR-0060]], [[ADR-0061]]. Paths: `packages/editor`, `packages/i18n`, `apps/live/src/extensions/database.ts`.
