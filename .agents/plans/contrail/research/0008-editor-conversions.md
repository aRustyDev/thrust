# Research 0008 — Editor data model without a JS runtime: spike 0006 results

Date: 2026-10-18. Spike: `spikes/0006-editor-conversions-rust.md` (bead `plane-byd.6`). Code: thrust worktree
`/Users/asmith/repos/arustydev/thrust-wt/spike-0006-editor-conversions`, branch `feat/spike-0006-editor-conversions`
(pushed to `aRustyDev/thrust`; draft PR [#18](https://github.com/aRustyDev/thrust/pull/18), stacked on spike 0001's [#17](https://github.com/aRustyDev/thrust/pull/17) and conflicting with it in `Cargo.lock`). Commits, both signed: `60c86b9` (Rust model + oracle corpus) and `59753e0` (sanitiser, both Workers, fuzz, benchmarks, probes).

Verification tags: **[tested]** I ran it and the command is in §13 · **[doc]** a source file or page I read (path
given) · **[inferred]** reasoning, not observed · **[unverified]** not checked. `[tested]` always means "on this
Mac, local workerd / native", never "on Cloudflare's network".

Oracle = the REAL headless `@plane/editor` code from the pinned tree
`/Users/asmith/repos/woven/forks/plane/.claude/worktrees/agent-a69ad036a9c59b0a8` (tag `oracle/v1-pin-20261018`),
copied without `node_modules` into a scratch workspace and installed from Plane's own `pnpm-lock.yaml`
(`@tiptap/core` 2.27.3, `@tiptap/html` 2.26.2, `y-prosemirror` 1.3.7, `yjs` 13.6.27, `prosemirror-model` 1.25.12,
`zeed-dom` 0.15.1, `entities` 5.0.0, Node 26.9, pnpm 11.10). The Plane tree was only read.

## 1. Verdict

| Exit criterion (spike file) | Result | Evidence |
|---|---|---|
| Round-trip fidelity ≥ 99 % on the corpus for supported nodes | **Met by Option A: 100 %** (263 documents, 1337 of 1337 (document, direction) pairs; 6000 fuzzed HTML inputs × 6 directions; every one of the 23 nodes and 8 marks at 100 %) | §4 |
| Sanitiser passes the XSS corpus | **Met at parity, exceeded when hardened.** Plane's nh3 and Rust ammonia (Plane allow-list) both neutralise 116 of 125 executable payloads; the 9 survivors are allow-list holes (CSS `url()`/`expression()`, `td background`, `data-emoji-url`). A hardened Rust config closes all 125 with no change to 261/261 editor-generated documents | §7 |
| Chosen option runs within Worker CPU/size limits | **Met.** Option A: 2.61 MiB wasm (930 KB gzip), 18 ms startup profile, 0.37 ms for a typical work-item write. Option C: 1.57 MiB minified (3.09 MiB not minified), 91 ms startup profile, 1.5 ms | §9 |
| Fallback: if A is too costly choose C and record it | **A is not too costly.** The prototype already covers the whole node set. C works too, as a bounded exception | §6, §11 |

| Option | Verdict | One line |
|---|---|---|
| **A** Rust implementation | **Pass.** Recommended for P1 conversion and for the P0 sanitiser | Same bytes as the JS oracle in every direction measured; ~16 person-days (10–25) to productionise; the cost is owning a faithful port plus a CI oracle |
| **B** client forms only at P0 | **Pass as a P0 stance, with three facts the ADR must state** (§8, §5) | Needs the Rust sanitiser (works) and no conversion; but work-item descriptions carry no Yjs binary today, client HTML ≠ server HTML in 37 % of corpus documents, and client JSON carries block `id`s that server projections drop |
| **C** JS conversion Worker | **Technically passes; not recommended as primary.** Documented, bounded exception only if the oracle-CI cost of A proves unmanageable | Runs in workerd with **no DOM shim**; needs `nodejs_compat`; 3–50× the CPU of A; adds a JS toolchain and a pinned Plane tag |

## 2. Inventory

### 2.1 Where the three forms are written and read today

| Surface | Writes | Reads/emits | Source |
|---|---|---|---|
| Pages (web, collab) | `apps/live` Hocuspocus `store` → `PATCH …/pages/<id>/description/` with **binary + html + json** (`PageBinaryUpdateSerializer`: base64 binary validated, html sanitised by nh3) | `GET …/description/` (binary) | `apps/live/src/extensions/database.ts`, `apps/api/plane/app/serializers/page.py` **[doc]** |
| Pages, collab socket down | web `use-page-fallback.ts` builds the binary client-side (`getBinaryDataFromDocumentEditorHTMLString`) and PATCHes all three | | `apps/web/hooks/use-page-fallback.ts` **[doc]** |
| Pages, first collab open with empty binary | `apps/live` converts `description_html` → binary (and writes all three back) | | `database.ts` `fetchDocument` **[doc]** |
| Work items (web editor) | **`description_html` + `description_json` only.** The only web file mentioning `description_binary` is `use-page-fallback.ts` | | `components/editor/rich-text/description-input/root.tsx` and a grep of `apps/web` **[doc]** |
| `/api/v1` work items | `description_html`: lxml parse + re-serialise, then nh3, then stored; `description_json`/`description_stripped` excluded from the serializer; `description_binary` accepted (validated) | v1 responses carry `description_html` and `description_binary`; **`description_binary` is `null` in 64 of 64 v1 work-item objects** | `plane/api/serializers/issue.py`; `oracle/corpus/work_items.json` **[doc]** |
| Duplicate page/issue | Celery `copy_s3_objects_of_description_and_assets` POSTs `description_html` to `apps/live` **`/convert-document`** (`variant`: `rich` for PAGE, `document` for ISSUE) and stores returned json + binary | | `bgtasks/copy_s3_object.py`, `apps/live/src/controllers/document.controller.ts` **[doc]** |
| `description_stripped` | recomputed on **every model save** as Django `strip_tags(description_html)` | | `db/models/{issue,page,description,…}.py` **[doc]** |

So the conversions that exist today are exactly: **html → {json, binary, html}** (`convertHTMLDocumentToAllFormats`, both
`/convert-document` and the lazy first-open path) and **binary → {json, html}** (every collab store). Both live in
`packages/editor/src/helpers/yjs-utils.ts` and use `@tiptap/html` `generateJSON`/`generateHTML` (zeed-dom, **not** a
browser DOM) plus `y-prosemirror` `prosemirrorJSONToYDoc`/`yXmlFragmentToProseMirrorRootNode` **[doc]**.

### 2.2 Schemas (three, all built by `getSchema`)

* `rich` = `CoreEditorExtensionsWithoutProps` (work-item descriptions, comments).
* `document` = rich + `DocumentEditorExtensionsWithoutProps` = + `issue-embed-component` (pages).
* `title` = `doc(heading)` with `heading.levels=[1]`; a page's title is a **second Yjs fragment named `title`** in the same update (`getBinaryDataFromDocumentEditorHTMLString(html, title)`).
* The `default` fragment name is `"default"`. Verified by dump `crates/thrust-editor-doc/tests/corpus/schema.json` **[tested]**.

**The client schema is not the server schema.** The client's `CoreEditorExtensions` also loads `UniqueID`, which adds a global
`id` attribute (uuid, rendered as `data-id`) to every block in `BLOCK_NODE_TYPES` (paragraph, heading, blockquote, codeBlock,
horizontalRule, the lists and items, table, image, imageComponent, callout, embed) and syncs it through Yjs
(`extensions/extensions.ts`, `extensions/unique-id/extension.ts` **[doc]**). The headless server set has no such extension, so
every server-side decode **silently drops those ids** (`computeAttrs` ignores unknown attributes); measured with a client-shaped
binary (`legacy-client-block-ids`): the oracle JSON has no block ids, and the Rust reader matches **[tested]**. Plane's nh3
allow-list lists `data-id` for this reason.

### 2.3 Supported-node matrix

Derived from the machine dump of the real schema (`schema.json`), plus what was learned from `toDOM`/`parseDOM`. Y shape for every
node: `Y.XmlElement(<node name>)` carrying every non-null attribute (any JSON value; `undefined` is written as an explicit
`undefined`), children as XmlElement/XmlText. Text: one `Y.XmlText` per run of text nodes, delta attributes
`{ <markName>: <markAttrs object> }`. Fidelity column = documents containing the type that pass D1/D2/D3/D4 (§4): **all 100 %**.

| node | content | attrs (default) | HTML out (server) | HTML in (parse rule) | risk |
|---|---|---|---|---|---|
| `doc` | `block+` | – | – | – | – |
| `paragraph` | `inline*` | textAlign=null | `<p class="editor-paragraph-block">` | `p`; textAlign from `style.textAlign` ∈ left/center/right | server HTML **drops `style`**, so alignment is not in server HTML |
| `heading` | `inline*` | textAlign=null, level=1 | `<hN class="editor-heading-block">` | `h1`–`h6`; level from the tag | as paragraph |
| `text` | – | – | escaped (`&apos; &quot; &nbsp; &shy;` too) | text | zeed-dom escapes `'` as `&apos;` |
| `hardBreak` | – | – | `<br>` | `br`; newlines inside preserved-whitespace text become hard breaks (`linebreakReplacement`) | – |
| `bulletList` / `orderedList` / `listItem` | `listItem+` / `paragraph block*` | ol: start=1, type=null | `<ul class=…>`, `<ol class=… start type>` | `ul`/`ol`/`li` with PM `normalizeList` (a list directly inside a list moves into the previous item); `start` via `parseInt` | – |
| `taskList` / `taskItem` | `taskItem+` / `paragraph block*` | checked=false | `<ul data-type=taskList>`, `<li class=flex data-checked data-type=taskItem><label><input type=checkbox [checked]><span></span></label><div>…</div></li>` | priority-51 rules `ul[data-type=taskList]`, `li[data-type=taskItem]` | boolean attribute form differs between zeed-dom (`data-checked`) and a browser |
| `blockquote` | `block+` | – | `<blockquote>` | `blockquote` | – |
| `codeBlock` | `text*`, marks none | language=null | `<pre><code class="language-x">` | `pre` (whitespace preserved) | **server parse never recovers `language`** (see §5) |
| `horizontalRule` | – | – | `<div class="py-4 border-strong-1" data-type=horizontalRule><div></div></div>` | that div, or `hr` | – |
| `image` | – | src, alt, title, width="35%", height, aspectRatio, alignment="left" | `<img …>` | `img[src]:not([src^="data:"])` | a browser lower-cases `aspectRatio`; numeric-looking attrs become numbers on parse |
| `imageComponent` | – | src, alt, title, id, width, height="auto", aspectRatio, alignment, status="pending" | `<image-component …>` | `image-component` (attribute names case-insensitive) | holds the asset id in `src`; `null` width/height are **dropped on write** (JS and Rust) so they read back as defaults |
| `mention` | – (inline atom) | id, entity_identifier, entity_name | `<mention-component …>` | `mention-component` | an all-digit id becomes a **number** on HTML parse (`fromString`) |
| `emoji` | – (inline atom) | name | `<span data-name data-type=emoji>glyph</span>` | `span[data-type=emoji]` | glyph lookup needs the 1.8k-entry GitHub emoji table (84 KB, embedded); **parse is shadowed** by the `customColor` span rule (§5) |
| `table` / `tableRow` / `tableHeader` / `tableCell` | `tableRow+` / `(cell\|header)*` / `block+` | row: background,textColor; th/td: colspan, rowspan, colwidth=[150], background(, textColor) | `<table><tbody>`, raw `background`/`textColor`/`colwidth` attributes | `table`,`tr`,`th`,`td` | parse keeps only the **first** number of `colwidth`; `style` colours never reach server HTML; the live editor's `fixTables` rewrites bad spans on open |
| `calloutComponent` | `block+` | id, data-icon-color, data-icon-name (undefined), data-emoji-unicode="128161", data-emoji-url, data-logo-in-use="emoji", data-background (undefined), data-block-type | `<div data-… >` | `div[data-block-type=callout-component]` | `"128161"` becomes the number `128161` after an HTML round trip |
| `issue-embed-component` (document only) | – | five ids/names, all undefined | `<issue-embed-component …>` | same tag | **stripped by Plane's current sanitiser allow-list** (§7) |

| mark | rank | attrs (default) | HTML out | parse rules | risk |
|---|---|---|---|---|---|
| `link` | 0 | href, target="_blank", rel="noopener noreferrer nofollow", class | `<a target rel class href>` (tiptap `mergeAttributes` order) | `a[href]`; `javascript:`/`data:`/`vbscript:` hrefs are rejected on parse and blanked on render | nh3 rewrites `rel` to `noopener noreferrer` (drops `nofollow`) |
| `textStyle` | 1 | – | `<span>` | `span` with a `style` attribute | – |
| `bold` `italic` `strike` `underline` | 2–5 | – | `strong em s u` | tags plus `font-weight`, `font-style`, `text-decoration` style rules (`consuming:false` for the last two) | – |
| `code` | 6 (excludes all) | – | `<code class=… spellcheck=false>` | `code` | – |
| `customColor` | 7 | color, backgroundColor | `<span data-text-color data-background-color>` (`style` dropped) | **every `span`** (rule never returns `false`) | every plain `<span>` becomes a `customColor` mark with null attrs |

## 3. Oracle and corpus

* `tools/spike-0006/setup-oracle.sh <plane-tree> <workdir>`: copies `packages/{editor,blocks,constants,hooks,types,utils,…}` (no `node_modules`), `pnpm install --ignore-scripts --filter "@plane/editor..."` against Plane's lockfile; jsdom 23.2.0 (the catalog pin) in a sibling dir for the real-editor check.
* `tools/spike-0006/gen-all.sh <workdir>` regenerates everything deterministically (fixed Yjs client ids; two consecutive runs gave the same md5 **[tested]**).
* **Corpus `crates/thrust-editor-doc/tests/corpus/corpus.json`**, 263 documents, synthetic content only, each a triple (ProseMirror JSON after the Yjs round trip, HTML, Yjs update v1) plus the raw Yjs structure:
  151 authored (every node/mark and attribute variant, unicode, 10 k and deep nesting, tables, callouts, embeds, titles, three realistic pages, 55 generated), 67 **foreign HTML** snippets (what an `/api/v1` client sends; run through the real `convertHTMLDocumentToAllFormats`; asserted equal to my fixed-id reproduction of it), 40 **history** binaries (hundreds of edits replayed through `updateYFragment`: tombstones, GC, two concurrent clients, a WAL-shaped merged update), 5 **legacy** binaries (attributes missing, unknown node and mark types, wrong value types, invalid content, client `UniqueID` block ids). Large documents (200 k characters, 3000 paragraphs, 2000 blocks, three 100–180 KB generated pages) are in `target/spike-0006/corpus-large.json` (git-ignored, regenerated).
* There is no real Plane database here: "stored binaries" are history/legacy binaries built with the real y-prosemirror code, not rows from a production instance **[unverified for real data]**.
* Differential fuzz (not committed, regenerated): `fuzz-html.json` 2000 inputs (seed 20261018, used while tuning) and `fuzz-html-heldout.json` 4000 inputs (seed 777, generated **after** tuning); `entities-fuzz.json` 6000 strings.

## 4. Option A — Rust implementation (`crates/thrust-editor-doc`)

What exists (≈ 4 300 lines): `schema.rs` (hand-transcribed schema + a port of ProseMirror's content-expression DFA: `matchType`, `findWrapping`, `fillBefore`), `pm.rs` (Node/Mark, `computeAttrs`, `createChecked`, `addToSet`, JSON), `ydoc.rs` (`yrs` XmlFragment read/write), `html.rs` (renderer byte-compatible with `generateHTML`), `vdom.rs` (port of the zeed-dom regex parser incl. its quirks), `domparse.rs` (port of ProseMirror's `DOMParser` plus the Tiptap rule semantics), `sanitize.rs`. Builds natively and for `wasm32-unknown-unknown` **[tested]**.

### 4.1 Fidelity (every comparison is against what the real JS code produced for the same input)

| Dir | What | Pass / total |
|---|---|---|
| D1 | json → yjs binary → json (Rust writer + Rust reader) | 263 / 263 |
| D1s | json → Yjs structure identical to what y-prosemirror writes (element names, attributes, XmlText deltas, explicit `undefined`) | 218 / 218 (history/legacy excluded) |
| D2 | binary written by JS → json (incl. 40 history/tombstone/two-client binaries, 5 legacy) | 263 / 263 |
| D3 | json → html, **byte-identical** to `generateHTML` | 263 / 263 |
| D4 | html → json, identical to `generateJSON` | 263 / 263 |
| D5 | v1 write: html → json → binary → json/html identical to `convertHTMLDocumentToAllFormats` | 67 / 67 |
| all | (document, direction) pairs | **1337 / 1337 = 100 %**; instance-weighted also 100 % |
| large docs | six documents, 24 274 node instances, all directions | 30 / 30 |
| fuzz (6000 mutated HTML inputs: dropped/swapped tags, wrong nesting, odd attributes, truncation, stray entities) | D4, D1, D1s, D2, D3, D5 each | **6000 / 6000** each (held-out seed 4000 / 4000) |
| entities | `unescape_html` vs zeed-dom `unescapeHTML` | 6000 / 6000 |

Per node/mark type every row is 100 % on D1–D4 (the per-type table is printed by `examples/fidelity.rs`). Number of documents containing the type ranges from 4 (`hardBreak`) to 263 (`doc`); the rarest are `hardBreak` 4, `image` 5, `textStyle` 6, `issue-embed-component` 6 (each also covered by the fuzz).

### 4.2 Controls that must fail (they do)

* **Schema mutants**: ten deliberate default changes (`image.width`, `heading.level`, `taskItem.checked`, `tableCell.colwidth`, `calloutComponent.data-emoji-unicode`, `orderedList.start`, `imageComponent.status`, `tableHeader.background`, `link.rel`, `customColor.color`). The corpus alone detected **7 of 10** at first; that exposed a blind spot (defaults never used because every document carries explicit values), so the "legacy" binaries were added, after which the corpus detects **10 of 10**, and the dump-diff (`Schema::diff_against_dump`) detects 10 of 10 independently.
* **Cross-document control**: decoding document *i*'s binary and comparing with document *i+1*'s JSON: 0 false matches in 193 comparisons.
* **Corrupted binaries** (truncate 50 %, drop last byte, flip a byte, zero 4 bytes, garbage, empty; 195 documents each): Rust rejected or changed the result in all truncation/garbage/empty cases and in 194/195 flips and 186/195 zero-runs; the rest are bytes with no semantic effect. **Yjs (JS) and yrs agreed on detected-vs-not in all 1170 cases** **[tested]**.
* The harness checks itself with a non-vacuity test (458 932 bytes of JSON compared over 237 non-trivial inputs).

### 4.3 Does the real JS editor open Rust-produced binaries? **Yes**

`tools/spike-0006/oracle/verify-rust-out.ts` loads what Rust wrote into the real stack **[tested]**:

* y-prosemirror decode (`yXmlFragmentToProseMirrorRootNode`) equals the expected JSON: 263 / 263 (json → binary) and 67 / 67 (html → binary, the v1 path).
* A **real TipTap `Editor`** (jsdom DOM, all headless extensions, `Collaboration` sync plugin) opens the Rust binary: `getJSON()` equals what the same editor gives for the oracle's own binary in 223 / 223 (+67 / 67 v1) and `getHTML()` likewise 223 / 223 (+67 / 67).
* 9 of 290 editor loads differ from the stored JSON, **identically for the oracle's binary**: the live editor normalises on load (emoji characters → `emoji` nodes, `fixTables` rewrites `colwidth`/`rowspan`, empty `customColor` marks removed). Not a Rust effect.

### 4.4 Robustness

* **Panics**: 31 440 mutated Yjs binaries and 6 000 damaged HTML inputs through parse + encode + decode + render under `catch_unwind`: **0 panics** (4 086 decoded, 27 354 rejected). On wasm32 a panic aborts and resets the instance (research 0004 §6.4), so this matters; it is evidence, not proof, for `yrs` **[tested, native]**.
* **Nesting depth**: the JS oracle fails (`RangeError`) between 1 000 and 5 000 levels depending on shape; the Rust Worker answers up to 1 000 and then dies with `memory access out of bounds` (stack overflow; the runtime reports the request as "hung"), 2 000+ for blockquote/list/inline and 3 000+ for div **[tested in workerd]**. Same practical ceiling, but a Rust overflow is an **instance reset**, not a catchable error: P1 needs an explicit depth cap (say 200) before parsing. `serde_json::from_str` also has a built-in depth limit of 128 for JSON text input.
* `Schema::new` costs 32 µs (release): build once per isolate.

### 4.5 Unsupported or lossy constructs (everything I know of)

1. **Browser-flavour HTML** is not produced (§5.3); only the zeed-dom flavour that the server emits today.
2. Overlapping-mark hashed Yjs attribute names (`mark--hash`) are read but never written (no mark in this schema is overlapping).
3. `ychange` attributes / snapshot rendering (version diff views) and Yjs subdocuments, hooks, embeds inside XmlText: not supported (the editor does not produce them).
4. JS `Number.prototype.toString` is approximated for attribute values (exact for integers and ordinary decimals, not for exponent forms) **[inferred]**.
5. Entity decoding is exact on 6000 fuzzed strings drawn from ~140 entity names; the full 2 231-name HTML5 table comes from the `htmlentity` crate, the 106 legacy no-semicolon names are a hard-coded list **[tested on the fuzz only]**.
6. The zeed-dom `^` + multiline-flag regex quirk is reproduced by using the same flags in the `regex` crate; a stray end tag popping the element stack is reproduced (it drops everything after it, as today).
7. Yjs update format v2 was not exercised.
8. No depth/size guards yet (§4.4).

## 5. Oracle behaviour the port reproduces on purpose (decisions for the owner)

Fidelity is measured against what the product does, defects included. Each row is a **[tested]** property of today's stack; "port" = what the Rust code does now. Each is a one-line switch to change when the decision is to fix rather than match.

| # | Today's behaviour | Evidence | Effect |
|---|---|---|---|
| 1 | Server `generateHTML` **drops every `style` attribute** (ProseMirror assigns `dom.style.cssText`, zeed-dom ignores it): text alignment, hex colours, table cell colours are not in server HTML (JSON and binary keep them) | `para-align-center`, `table-colors` | Pages whose `description_html` is projected by `apps/live` lose alignment/colour in HTML |
| 2 | Server HTML → JSON **never recovers a code block `language`** (`element.firstElementChild` does not exist in zeed-dom, so the parse hook is dead) | `f-pre-code`, `code-lang` | Any `/api/v1` write of `<pre><code class="language-js">` loses the language in the binary/JSON |
| 3 | **Every `<span>` becomes a `customColor` mark**, and `<span data-type="emoji">` never reaches the emoji rule | `f-span`, `f-emoji` | Emoji HTML does not round trip as an emoji node |
| 4 | `colwidth="100,200"` parses to `[100]` | `f-table-span` | multi-column widths lost |
| 5 | `fromString` coercion: numeric-looking attribute strings become numbers (callout `data-emoji-unicode "128161"` → `128161`, an all-digit mention id) | `callout-default`, `mention-user` | type change after an HTML round trip |
| 6 | Server projections drop the client's block `id`s (§2.2) | `legacy-client-block-ids` | client JSON/HTML ≠ server JSON/HTML |
| 7 | Plane's sanitiser strips `issue-embed-component` and its five attributes (not in `CUSTOM_TAGS`) | §7 | page HTML loses work-item embeds when saved through `PageBinaryUpdateSerializer` (binary/JSON keep them) |
| 8 | nh3 rewrites `a rel` to `noopener noreferrer` | §7 | 61 of 195 editor HTML documents change their parsed `link.rel` |

### 5.3 The two HTML flavours (matters for ADR-0061 Q-0061.4 and Option B)

P0 stores the **client-produced** `description_html` (`editor.getHTML()`, a real DOM). Server projections (`apps/live` today, any Rust/JS conversion later) are the **zeed-dom flavour**. Comparing `getHTMLFromFragment` on a real DOM (jsdom) with the zeed-dom output for the same 263 documents **[tested]**: identical for 164, different for 99 (37 %). Reasons (non-exclusive): `style` attributes present only in the browser flavour (74), boolean attribute form `data-checked` vs `data-checked="true"` (27), attribute-name case (browser lower-cases `aspectRatio`, `textColor`) (14), `'` as `&apos;` and `"` in text (7), other (5). Both parse back to the same document in the cases checked, but the bytes differ, so any equality test that mixes the two will fail spuriously. The Rust renderer reproduces the zeed-dom flavour exactly; a browser-flavour mode is not built (§11, effort).

## 6. Option C — JS conversion Worker (`tools/spike-0006/jsworker`)

The real `packages/editor/src/helpers/yjs-utils.ts` functions (`convertHTMLDocumentToAllFormats`, `getAllDocumentFormats…BinaryData`) bundled by esbuild (the version already in Plane's lockfile) with **no source edits and no DOM shim**, run in `wrangler dev --local` (wrangler 4.123.0 / workerd).

| Question (Q-0012.1) | Answer |
|---|---|
| Runs in workerd without jsdom or any DOM shim? | **Yes.** In workerd `window`, `document`, `DOMParser`, `Node`, `localStorage` are all `undefined`; `navigator`, `Buffer`, `TextEncoder` exist. `@tiptap/html` uses zeed-dom, a pure-JS DOM-like tree |
| Needs Node APIs? | **Yes, one:** `@plane/constants` reads `process.env.VITE_*` at module load. Without the `nodejs_compat` flag the Worker does not start (`ReferenceError: process is not defined`). Either set `compatibility_flags = ["nodejs_compat"]` or define those four keys at bundle time |
| Bundle | 3 238 425 B (3.09 MiB) unminified, gzip 640 KB; **1 651 352 B (1.57 MiB) minified**, gzip 444 KB. Drivers: `@tiptap/extension-emoji` data 654 KB, `highlight.js` 332 KB, `prosemirror-view` 197 KB, `yjs` 184 KB, `entities` 146 KB, plus React/propel/postcss/tailwind-merge pulled in only through `@plane/utils` and node-view modules that headless conversion never executes |
| Startup (`wrangler check startup`, local V8) | profile window 90.7 ms, active 50.3 ms (2.5 ms GC) |

A fork-owned headless entrypoint (ADR-0012 Option 2) would drop most of the 1.57 MiB; that was not built.

**Shim contract if C is ever used** (bounded exception to "all Rust", ADR-0006):
* Port: `EditorDocs { html_to_all(variant, html) -> {json, binary, html}; binary_to_all(variant, bytes) -> {json, html}; }` called over a **service binding** from the Rust API/DO. No other JS in the request path.
* Input/output: UTF-8 html or base64 binary in, JSON out. No business logic, no storage, no secrets, no network.
* Source of truth for the code: a fork-owned pinned copy of `packages/editor` conversion helpers at one Plane tag, built by esbuild in CI; version string returned in a response header and stored with each projection (schema version).
* Acceptance: the same corpus, fuzz sets and controls as A run against the Worker.
* Exit plan: replace the adapter with Option A (`thrust-editor-doc`) behind the same port; no data migration because both produce byte-identical output on the corpus.

## 7. Sanitisation

**Plane today** (`apps/api/plane/utils/content_validator.py`, loaded unmodified from the pinned tree for the oracle): `nh3.clean(html, tags=nh3.ALLOWED_TAGS | {mention-component, label, input, image-component}, attributes=ATTRIBUTES, url_schemes={http,https,mailto,tel})`, plus on `/api/v1` an lxml parse/serialise before it. `nh3` 0.2.18 is a binding to **ammonia 4.0.0** (read from its sdist `Cargo.lock`). `ATTRIBUTES` *replaces* ammonia's per-tag defaults; `style` is allowed on every element; comments stripped; `link_rel="noopener noreferrer"`.

**Rust** (`sanitize.rs`): the same allow-list on ammonia; builds natively and for wasm32 **[tested]**. Three further configurations measured: ammonia 4.0.0 (probe crate `probes/ammonia-400`, same source), latest 4.2.4, and a hardened variant.

XSS corpus `tests/corpus/xss-corpus.json`: 169 payloads, 125 labelled executable (OWASP Filter Evasion subset: basic, encodings, event handlers, 42 tag/element vectors, CSS, mXSS; plus Plane constructs: mention/image/embed/callout/table/task/code/link attributes) and 44 inert controls. Checker: independent of both sanitisers (libxml2 via lxml and html5lib, fragment and document parses, then a second parse after re-serialising).

| Control (must fail) | Result |
|---|---|
| Unsanitised input: the checker flags every executable payload | **125 / 125** flagged, 0 false positives on the 44 inert ones (an earlier labelling pass had 22 misses; those were mislabelled or parse-context-dependent payloads and were relabelled, not hidden) |
| Identity "sanitiser" | 125 / 125 unsafe |
| `<script>` regex stripper | 113 / 125 unsafe |
| `<script>` + `on*=` regex stripper | 73 / 125 unsafe |

| Sanitiser | Executable payloads neutralised |
|---|---|
| nh3 0.2.18, app path (`validate_html_content`) | **116 / 125** |
| nh3 0.2.18, `/api/v1` path (lxml first) | 116 / 125 |
| Rust ammonia 4.0.0, Plane allow-list | 116 / 125, **byte-identical to nh3 on 430 / 430** inputs (169 payloads + 261 corpus HTML documents) |
| Rust ammonia 4.2.4, Plane allow-list | 116 / 125, byte-identical on 423 / 430 |
| Rust ammonia 4.2.4 + value filters (hardened) | **125 / 125**; output byte-identical to the plain config on 195 / 195 editor-generated documents |

The 9 payloads that survive Plane's allow-list in *every* implementation (all legacy-browser or CSS-injection class, none a script element or event handler; **not exploitable in evergreen browsers [inferred]**): six `style` values (`url(javascript:)`, `expression()`, `behavior:`, `-moz-binding`, `list-style:url(javascript:)`, `background-image:url('javascript:…')`), `td background="javascript:…"`, `td style` with `url(javascript:)`, and a callout `data-emoji-url="javascript:…"`. The hardened variant (`hardened_builder`) filters `style` (dangerous CSS), `background` and `data-emoji-url` values; `style="position:fixed…"` UI redress and `id`/`name` clobbering remain allowed (advisory).

**Version drift 4.0.0 → 4.2.4** (the reason output is not byte-identical): 4.0.0 emits `<` and `>` raw inside attribute values (`<p id="</p><script>alert(1)</script>">`), 4.2.4 escapes them; 4.2.4 removes whole `svg`/`math` payloads that 4.0.0 leaves partly behind. One editor document differs (`<alt>` inside an `alt` attribute), parse-equivalent. So moving off nh3 changes stored bytes in rare cases and is a security improvement.

**What Plane's allow-list does to real editor HTML** (parse before vs after, 195 editor-generated documents, **[tested]**): 129 unchanged, **66 change, for two reasons only**: `link.rel` loses `nofollow` (61) and `issue-embed-component` is removed (5). With `issue-embed-component` and `rel` added to the allow-list the difference disappears; decide whether that is wanted.

**Other derived-field parity**: `strip_tags` (Django's `HTMLParser`-based function behind `description_stripped`) ported; **391 / 391** identical to the real Django function on the same cleaned inputs. `validate_binary_data` and the 10 MB cap ported.

**CPU**: sanitising a 2.4 KB document takes 0.06 ms, 125 KB 2.9 ms, 745 KB 17 ms (workerd, §9).

## 8. API v1 writes: can the server produce a binary that collab can open?

**Yes.** Path under test: raw html → ammonia → parse → `yrs` encode → decode/render.
* Against Plane's real chain (nh3 → real `generateJSON` → y-prosemirror → real `generateHTML`), 391 inputs (payloads + corpus HTML): **388 identical**; the 3 that differ are the svg/math payloads that 4.2.x removes entirely (§7).
* The binary written for the 67 foreign HTML documents (the v1-style snippets) is accepted by the real TipTap editor in jsdom with the Collaboration plugin: JSON and HTML equal to the editor's result on the oracle's binary, 67 / 67 each (§4.3).
* Facts the ADR must carry: (i) today a v1 write leaves `description_binary` null and the binary is created lazily on first collab open (ADR-0012 round 25 already says so); (ii) because of §5 rows 2, 3, 4, 6 the converted document is not always what the writer meant (code languages, emoji, column widths); (iii) work-item descriptions have no collab path today, so "open a work item description in collab" is new scope (ADR-0060), and the first open must convert html → binary.

## 9. Size, CPU and cost (local workerd; Apple silicon; not Cloudflare's network)

Method: workerd freezes `Date.now()` during synchronous work, so each route repeats the operation *n* times in one request, the client subtracts the same request with the work disabled and divides by *n* (median of 7). Source: `tools/spike-0006/bench`.

**Bundle** (limit 64 MiB, target < 20 MiB; spike 0001's core Worker is 1.58 MiB):

| Build | Size | gzip |
|---|---|---|
| Option A, whole Worker (conversions + sanitiser + strip_tags + emoji table) | **2 735 457 B (2.61 MiB)** wasm + 20.9 KB glue | 930 KB |
| Option A without ammonia/html5ever (`--no-default-features`) | 2 073 114 B (1.98 MiB) | 704 KB |
| Option C, esbuild, minified | 1 651 352 B (1.57 MiB) | 444 KB |
| Option C, unminified | 3 238 425 B (3.09 MiB) | 640 KB |

The sanitiser adds 0.63 MiB. Either option plus the core Worker stays under 5 MiB.

**Startup** (`wrangler check startup`, local V8): A 18.2 ms window / 11.4 ms active; C 90.7 ms / 50.3 ms. Limit 1 s.

**Per-operation CPU, median ms per operation** (v1 write = sanitise + parse + encode + decode + render + strip_tags for A; for like-for-like with C the "no sanitise" column is `convertHTMLDocumentToAllFormats`'s steps; projection = binary → json + html):

| Document | html size | A v1 write | A no-sanitise | C html→all | A projection | C projection | A sanitise |
|---|---|---|---|---|---|---|---|
| one paragraph | 49 B | 0.016 | 0.013 | 0.69 | 0.0038 | 0.32 | 0.002 |
| work-item description | 2.4 KB | **0.37** | 0.29 | 1.49 | 0.09 | 0.57 | 0.063 |
| page | 2.8 KB | 0.48 | 0.41 | 1.83 | 0.14 | 0.63 | 0.077 |
| 11 KB page | 11 KB | 1.7 | 1.4 | 3.7 | 0.50 | 1.49 | 0.25 |
| 125 KB page | 125 KB | 18.6 | 15.3 | 32.7 | 5.1 | 11.8 | 2.9 |
| 344 KB (3000 paragraphs) | 344 KB | 45 | 41 | 63 | 7.2 | 29.9 | 3.3 |
| 745 KB (2000 blocks, 1 MB binary) | 745 KB | 118 | 99 | 254 | 29.9 | 94.9 | 17 |

A is 2–50× faster than C (50× only on tiny inputs where C is dominated by fixed overhead; 2–3× on large documents). The Workers free plan's 10 ms CPU cap would be exceeded by a ~50 KB document in A and ~25 KB in C, so the paid plan already assumed in research 0004 is required.

**Projected monthly CPU cost at the sizing target (≤ 50 users, ≤ 100 k work items)** — arithmetic from the table, assumptions mine: one-off import of 100 k work items (2.4 KB each); 30 000 `/api/v1` description writes per month; P1 collab projection on every debounced store, 50 users × 22 days × 720 stores/day (2 h editing, one store per 10 s) = 792 000 stores/month, half small (3.6 KB) and half medium (16 KB) pages.

| | A | C |
|---|---|---|
| Import (once) | 37 s CPU | 155 s |
| Per month | 264 s = 0.26 M CPU-ms | 887 s = 0.89 M CPU-ms |
| Cost at $0.02 per M CPU-ms | **$0.005** | **$0.018** |
| At 10× the volume | $0.05 | $0.18 |

Both sit inside the 30 M CPU-ms included with the paid plan (research 0004 §9 baseline: 51.7 M ms for 5 M requests), so the marginal bill is $0 until usage is far larger; at most it is cents. Option C also adds one service-binding request per call (≈ 0.8 M/month, inside the 10 M included). Durable Object CPU is not separately modelled **[unverified]**. The cost is not a deciding factor; the deciding factors are fidelity, ownership and operational complexity.

## 10. Implication for the Dioxus editor (spike 0007)

* **Yjs binary stays the long-term source of truth for any collaboratively edited document.** It is the only form that merges and carries identity; JSON and HTML are projections. JSON is exactly derivable from the binary (D2: 263/263), the reverse loses history and client identity, and HTML loses more (§5).
* The ProseMirror-JSON model in `thrust-editor-doc` (`Node`, `Mark`, schema tables, content DFA, `serde` friendly) is the right shared **intermediate representation** for a Dioxus editor: the same crate compiles to wasm32 for the web target and natively for desktop/mobile, and talks to the server's `yrs` document without a conversion service. The ProseMirror *view layer* (selection, input rules, IME, drag handles) is what Rust lacks; that is Q-0013.2, not a data-model gap.
* Dioxus should not parse HTML as an input format. HTML import (paste, v1) is a lossy, quirk-ridden ingest path that belongs on the server; the editor should read/write XmlFragment and JSON.
* Keep the schema versioned. The editor's `UniqueID` block ids (§2.2) mean the client and headless schemas already differ; a Dioxus client adds a third unless the schema crate is the single definition and generates both.

## 11. Recommendations

**ADR-0012.** Accept: *P0 = Option B; P1 conversion = Option A, implemented by `thrust-editor-doc` behind an `EditorDocs` port; Option C recorded as the documented, bounded exception (contract in §6) to be used only if the oracle-CI cost of A is rejected.* Keep "binary format unchanged" (D1s/D2: the Rust writer and reader are structurally identical to y-prosemirror's). Confidence: high on feasibility, medium on cost of tracking upstream.

**P0 stance (B).** Store client-produced html/json/binary verbatim; sanitise `description_html` with Rust ammonia using Plane's allow-list. Decide four items now: (1) parity with nh3 or hardened (recommended: hardened; zero effect on the 195 editor documents); (2) add `issue-embed-component` and its attributes (and `rel`) to the allow-list or keep stripping; (3) accept that ammonia ≥ 4.1 is not byte-identical to nh3 4.0.0; (4) compute `description_stripped` with the ported `strip_tags` (391/391 parity). Record that work-item descriptions carry no binary and that client and server html/json differ (§5.3, §2.2).

**P1 stance.** Collab Durable Object in Rust with `yrs` projects html/json synchronously at store time: 0.09–0.5 ms for typical pages (A) versus a service-binding hop and 0.6–1.5 ms plus a 91 ms isolate cold start (C). **Q-0061.4**: the ≤ 5 s staleness allowance is not needed for CPU reasons; strong consistency inside the store transaction is affordable. Add an explicit depth cap, a size cap and a "schema version" column.

**Effort to finish A for the full node set (person-days, central 16, range 10–25):**

| Work | Days |
|---|---|
| Depth/size guards, error taxonomy, panic audit (`unwrap`s), longer `yrs` fuzz, cap policy | 3 |
| `EditorDocs` port, adapter, v1 handlers, collab DO integration | 3 |
| Oracle CI: regenerate corpus on each Plane pin bump, drift gates (`diff_against_dump`, fuzz), upstream merge process | 3 |
| Decide and implement parity-vs-fix for §5 rows 1–8, tests | 2 |
| Browser-flavour HTML mode (only if P0 equality with client HTML is required) | 3 (optional) |
| Emoji/entity/number-formatting residue; docs | 1.5 |
| Stage 2 on Cloudflare (real CPU, DO, memory), wasm size trim | 1.5 |

Uncertainty drivers: how often upstream Plane changes the schema or the rules (the CI cost), the §5 decisions, `yrs` robustness on adversarial input, and whether the browser flavour is required. The prototype (≈ 4 300 lines of Rust plus ≈ 3 600 lines of tests and tooling) already passes 100 % of what was measured, so most of the remaining risk is operational, not algorithmic.

**Suggested edits for the coordinator** (I changed none): ADR-0012 *Confirmation*: replace "run conversions in workerd" with the §1 table and the §6 answers; ADR-0061 *Confirmation (d)*: "projection equals today's server output" is met by A byte-for-byte for the zeed-dom flavour, with the §5.3 caveat; Q-0012.1 answered (no DOM shim; `nodejs_compat` or a build-time define); Q-F5.1: add "work items carry no binary today"; Q-0061.4: answered above; ADR-0006: record C as a conditional exception only; spike 0007: take §10.

## 12. What is not proven

* **Nothing ran on Cloudflare.** All timings are local workerd on one laptop; production CPU, cold start, memory and Durable Object behaviour are **[unverified]**.
* **No real Plane data.** The "stored" binaries are produced with the real y-prosemirror code, including edit histories and a two-client merge, not exported from a live instance. Binaries from older Plane versions, other collab servers or the Y update v2 format were not tried.
* The fuzz generators are mine; a different mutation strategy could find a divergence the 6000+ inputs did not (the held-out set was 4000 inputs with 0 failures after the tuning set exposed 5).
* The XSS checker uses libxml2 and html5lib, not real browsers; "not exploitable in evergreen browsers" for the 9 surviving payloads is **[inferred]**. The mXSS cases that depend on scripting-enabled parsing (e.g. `noscript`) are not observable by this checker and are labelled inert.
* `yrs` panic-freedom is supported by 31 k mutations, not proven. Memory use under adversarial documents was not measured.
* Entity decoding exactness is shown on strings drawn from ~140 names, not exhaustively.
* The browser HTML flavour was compared with jsdom, not Chrome/Safari.
* Not built: browser-flavour renderer, fork-owned minimal headless JS entry, the real `EditorDocs` port, load test with concurrent requests.

## 13. Reproduce

```
tools/spike-0006/setup-oracle.sh <plane-tree> <workdir>                  # once: oracle workspace from Plane's lockfile
tools/spike-0006/gen-all.sh <workdir>                                    # corpus, schema dump, emoji table, fuzz sets (deterministic)
scripts/rs cargo test -p thrust-editor-doc                               # fidelity, fuzz, controls, entities, panics, sanitiser
scripts/rs cargo run --release -p thrust-editor-doc --example fidelity   # tables in §4 ; --large for the big documents
JSDOM_FROM=<workdir>/extra/package.json tools/spike-0006/run-oracle.sh <workdir> verify-rust-out.ts \
  target/spike-0006/rust-out.json crates/thrust-editor-doc/tests/corpus/corpus.json target/spike-0006/corrupt.json   # §4.3
tools/spike-0006/sanitize/run-all.sh <plane-tree> [uv-env-dir]           # §7 (uv; nh3 0.2.18, lxml 6.1.0, bs4 4.12.3, html5lib 1.1)
scripts/rs worker-build --release crates/thrust-editor-worker            # Option A bundle ; add --no-default-features for the lean size
tools/spike-0006/bench/run-bench.sh rust-a crates/thrust-editor-worker wrangler.toml        # §9
NODEJS_COMPAT=1 tools/spike-0006/jsworker/build.sh <workdir> <outdir> && \
  tools/spike-0006/bench/run-bench.sh js-c <outdir> wrangler.toml        # Option C ; MINIFY=1 for the minified size
tools/spike-0006/bench/startup.sh <label> <dir> ; tools/spike-0006/bench/deep-probe.sh
```

Files: `crates/thrust-editor-doc/` (library, tests, `examples/fidelity.rs`, `examples/sanitize_check.rs`, corpus), `crates/thrust-editor-worker/` (Option A Worker), `probes/ammonia-400/`, `tools/spike-0006/{oracle,sanitize,jsworker,bench}/`.
