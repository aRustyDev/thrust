// Spike 0006: does the REAL JS stack accept what the Rust crate wrote?
//
//   tools/spike-0006/run-oracle.sh <workdir> verify-rust-out.ts <rust-out.json> <corpus.json> [corrupt.json]
//
// For every document the Rust writer produced:
//   (a) y-prosemirror decode (yXmlFragmentToProseMirrorRootNode, what apps/live and the client run) == expected JSON
//   (b) the raw Yjs structure (element names, attributes, XmlText deltas) == the structure y-prosemirror itself writes
//   (c) a REAL TipTap Editor with the Collaboration extension (jsdom DOM, full extension set + y-prosemirror sync
//       plugin) loads the binary and its getJSON() equals the expected JSON; getHTML() equals the HTML the same
//       editor gives for the oracle's own binary.
// Also replays the Rust corruption control through Yjs to compare detection verdicts.
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import * as Y from "yjs";
import { Editor, Extension, getSchema } from "@tiptap/core";
import { Collaboration } from "@tiptap/extension-collaboration";
import { CustomStarterKitExtension } from "@/extensions/starter-kit";
import { b64, binaryToJson, canon, extFor, schemaFor, unb64, ystructOfBinary } from "./lib";

const [, , rustOutPath, corpusPath, corruptPath] = process.argv;
const rust = JSON.parse(readFileSync(rustOutPath, "utf8"));
const corpus = JSON.parse(readFileSync(corpusPath, "utf8"));
const byId = new Map<string, any>(corpus.docs.map((d: any) => [d.id, d]));

// ---- jsdom for the editor view ---------------------------------------------------------------------------
const require_ = createRequire(process.env.JSDOM_FROM ?? "/");
const { JSDOM } = require_("jsdom");
const dom = new JSDOM("<!doctype html><html><body></body></html>", { pretendToBeVisual: true });
const w = dom.window as any;
for (const k of ["window", "document", "navigator", "Node", "Element", "HTMLElement", "Text", "Range", "Selection", "DOMParser", "MutationObserver", "getComputedStyle", "requestAnimationFrame", "cancelAnimationFrame", "DocumentFragment", "XMLSerializer"]) {
  try {
    Object.defineProperty(globalThis, k, { value: w[k] ?? (k === "window" ? w : undefined), configurable: true, writable: true });
  } catch {
    /* some globals (navigator) are read-only getters on newer Node; the editor does not need them */
  }
}
const zeroRect = { x: 0, y: 0, width: 0, height: 0, top: 0, left: 0, right: 0, bottom: 0, toJSON() {} };
w.Range.prototype.getBoundingClientRect = () => zeroRect;
w.Range.prototype.getClientRects = () => ({ length: 0, item: () => null, [Symbol.iterator]: function* () {} });
w.Element.prototype.getClientRects = () => ({ length: 0, item: () => null, [Symbol.iterator]: function* () {} });
(w.document as any).elementFromPoint = () => null;

function realEditorLoad(variant: "rich" | "document", bin: Uint8Array): { json: unknown; html: string } {
  const ydoc = new Y.Doc();
  Y.applyUpdate(ydoc, bin);
  const base = extFor(variant);
  // history is replaced by y-prosemirror's undo manager when collaborating (Plane disables it the same way)
  // The product's Utility extension only supplies editor.storage.utility (read by the emoji plugin): a stub is enough.
  const Utility = Extension.create({ name: "utility", addStorage: () => ({ isTouchDevice: false, uploadInProgress: false, activeDropbarExtensions: [], assetsUploadStatus: {} }) });
  const exts = [CustomStarterKitExtension({ enableHistory: false }), ...base.slice(1), Utility, Collaboration.configure({ document: ydoc, field: "default" })];
  const el = w.document.createElement("div");
  w.document.body.appendChild(el);
  const editor = new Editor({ element: el, extensions: exts as any, editable: true });
  const out = { json: editor.getJSON(), html: editor.getHTML() };
  editor.destroy();
  el.remove();
  return out;
}

type Row = { id: string; check: string; ok: boolean; note?: string };
const rows: Row[] = [];
const note = (id: string, check: string, ok: boolean, n?: string) => rows.push({ id, check, ok, ...(n ? { note: n } : {}) });

for (const r of rust.docs as any[]) {
  const d = byId.get(r.id);
  if (!d) continue;
  const variant = d.variant as "rich" | "document";
  const schema = schemaFor(variant);
  for (const [field, expectJson, expectStruct, label] of [
    ["binary", d.json, d.ystruct, "Rust writer (json -> binary)"],
    ["v1_binary", d.json, undefined, "Rust v1 pipeline (html -> binary)"],
  ] as const) {
    if (!r[field]) continue;
    const bin = unb64(r[field]);
    let json: unknown;
    try {
      json = binaryToJson(schema, bin);
    } catch (e) {
      note(r.id, `${label}: y-prosemirror decode`, false, String(e));
      continue;
    }
    note(r.id, `${label}: y-prosemirror decode == oracle json`, canon(json) === canon(expectJson), canon(json) === canon(expectJson) ? undefined : "json differs");
    if (expectStruct && d.kind !== "history" && d.kind !== "legacy") {
      note(r.id, `${label}: yjs structure == y-prosemirror's`, canon(ystructOfBinary(bin)) === canon(expectStruct));
    }
  }
}

// (c) the real editor, on a sample (full set is slow in jsdom): every authored doc without huge size + all foreign
let editorChecked = 0;
for (const r of rust.docs as any[]) {
  const d = byId.get(r.id);
  if (!d || !r.binary || d.kind === "history") continue;
  if ((d.binary as string).length > 60000) continue;
  const variant = d.variant as "rich" | "document";
  const bins: [string, string][] = [["binary", r.binary]];
  if (r.v1_binary) bins.push(["v1_binary", r.v1_binary]);
  for (const [label, rb] of bins) {
    let theirs, mine;
    try {
      theirs = realEditorLoad(variant, unb64(d.binary));
      mine = realEditorLoad(variant, unb64(rb));
    } catch (e) {
      note(r.id, `real TipTap editor opens Rust ${label}`, false, String(e).slice(0, 200));
      continue;
    }
    editorChecked++;
    note(r.id, `real TipTap editor opens Rust ${label}: getJSON equal to editor on oracle binary`, canon(mine.json) === canon(theirs.json));
    note(r.id, `real TipTap editor opens Rust ${label}: getHTML equal to editor on oracle binary`, mine.html === theirs.html);
    note(r.id, `real TipTap editor opens Rust ${label}: getJSON == expected stored json`, canon(mine.json) === canon(d.json), canon(mine.json) === canon(d.json) ? undefined : "editor normalised the document");
  }
}

// corruption replay
const corr: any[] = corruptPath ? JSON.parse(readFileSync(corruptPath, "utf8")) : [];
const cstats: Record<string, { n: number; jsErr: number; jsDiff: number; jsSame: number; agreeDetect: number }> = {};
for (const c of corr) {
  const bin = unb64(c.binary);
  let verdict = "same";
  try {
    const got = binaryToJson(schemaFor("document"), bin);
    verdict = canon(got) === canon(c.expected_json) ? "same" : "differs";
  } catch {
    verdict = "error";
  }
  const s = (cstats[c.corruption] ??= { n: 0, jsErr: 0, jsDiff: 0, jsSame: 0, agreeDetect: 0 });
  s.n++;
  if (verdict === "error") s.jsErr++;
  else if (verdict === "differs") s.jsDiff++;
  else s.jsSame++;
  const rustDetected = c.rust !== "undetected";
  const jsDetected = verdict !== "same";
  if (rustDetected === jsDetected) s.agreeDetect++;
}

const failed = rows.filter((r) => !r.ok);
const by: Record<string, [number, number]> = {};
for (const r of rows) {
  const k = r.check.replace(/^real TipTap editor opens Rust (binary|v1_binary)/, "real TipTap editor opens Rust $1");
  (by[k] ??= [0, 0])[1]++;
  if (r.ok) by[k][0]++;
}
console.log(JSON.stringify({ summary: by, editorChecked, failed: failed.slice(0, 60), corruption: cstats }, null, 1));
void b64;
void getSchema;
