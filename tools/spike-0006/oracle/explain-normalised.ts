// Spike 0006: what does a REAL editor change when it loads a stored document? (the 9 cases verify-rust-out.ts flags)
//   JSDOM_FROM=<workdir>/extra/package.json tools/spike-0006/run-oracle.sh <workdir> explain-normalised.ts <corpus.json> id1 id2 ...
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import * as Y from "yjs";
import { Editor, Extension } from "@tiptap/core";
import { Collaboration } from "@tiptap/extension-collaboration";
import { CustomStarterKitExtension } from "@/extensions/starter-kit";
import { canon, extFor, unb64 } from "./lib";

const corpus = JSON.parse(readFileSync(process.argv[2], "utf8"));
const ids = process.argv.slice(3);
const { JSDOM } = createRequire(process.env.JSDOM_FROM ?? "/")("jsdom");
const dom = new JSDOM("<!doctype html><html><body></body></html>", { pretendToBeVisual: true });
const w = dom.window as any;
for (const k of ["window", "document", "Node", "Element", "HTMLElement", "Text", "Range", "Selection", "DOMParser", "MutationObserver", "getComputedStyle", "requestAnimationFrame", "cancelAnimationFrame", "DocumentFragment"]) {
  Object.defineProperty(globalThis, k, { value: w[k] ?? (k === "window" ? w : undefined), configurable: true, writable: true });
}
const zero = { x: 0, y: 0, width: 0, height: 0, top: 0, left: 0, right: 0, bottom: 0, toJSON() {} };
w.Range.prototype.getBoundingClientRect = () => zero;
w.Range.prototype.getClientRects = () => ({ length: 0, item: () => null, [Symbol.iterator]: function* () {} });
w.Element.prototype.getClientRects = () => ({ length: 0, item: () => null, [Symbol.iterator]: function* () {} });

function diff(a: any, b: any, path = ""): string | null {
  if (typeof a !== typeof b || a === null || b === null || typeof a !== "object") return canon(a) === canon(b) ? null : `${path}: stored ${canon(a)?.slice(0, 80)} -> editor ${canon(b)?.slice(0, 80)}`;
  if (Array.isArray(a) !== Array.isArray(b)) return `${path}: array/object mismatch`;
  const keys = new Set([...Object.keys(a), ...Object.keys(b)]);
  for (const k of keys) {
    if (!(k in a)) return `${path}/${k}: added by editor ${canon(b[k])?.slice(0, 80)}`;
    if (!(k in b)) return `${path}/${k}: removed by editor (was ${canon(a[k])?.slice(0, 80)})`;
    const d = diff(a[k], b[k], `${path}/${k}`);
    if (d) return d;
  }
  return null;
}

for (const id of ids) {
  const d = corpus.docs.find((x: any) => x.id === id);
  const ydoc = new Y.Doc();
  Y.applyUpdate(ydoc, unb64(d.binary));
  const Utility = Extension.create({ name: "utility", addStorage: () => ({ isTouchDevice: false, uploadInProgress: false, activeDropbarExtensions: [], assetsUploadStatus: {} }) });
  const exts = [CustomStarterKitExtension({ enableHistory: false }), ...extFor(d.variant).slice(1), Utility, Collaboration.configure({ document: ydoc, field: "default" })];
  const el = w.document.createElement("div");
  w.document.body.appendChild(el);
  const editor = new Editor({ element: el, extensions: exts as any });
  console.log(id, "=>", diff(d.json, editor.getJSON()) ?? "(equal)");
  editor.destroy();
}
