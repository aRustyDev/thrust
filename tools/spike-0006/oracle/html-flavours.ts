// Spike 0006: the two HTML flavours that exist in Plane today.
//   server  = generateHTML() on zeed-dom (apps/live, convertHTMLDocumentToAllFormats): what the Rust renderer reproduces
//   browser = editor.getHTML() in a real DOM (what the P0 client sends as description_html): jsdom stands in for the browser
// How often do they differ, and why?
//
//   tools/spike-0006/run-oracle.sh <workdir> html-flavours.ts <corpus.json>   (JSDOM_FROM must point into <workdir>/extra)
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { getHTMLFromFragment } from "@tiptap/core";
import { Node as PMNode } from "@tiptap/pm/model";
import { schemaFor } from "./lib";

const corpus = JSON.parse(readFileSync(process.argv[2], "utf8"));
const { JSDOM } = createRequire(process.env.JSDOM_FROM ?? "/")("jsdom");
const dom = new JSDOM("<!doctype html><html><body></body></html>");
const doc = dom.window.document;
// @tiptap/core's getHTMLFromFragment is what editor.getHTML() calls; it reads the global document
for (const k of ["window", "document"]) Object.defineProperty(globalThis, k, { value: k === "window" ? dom.window : doc, configurable: true, writable: true });

let same = 0;
let total = 0;
const reasons: Record<string, number> = {};
const examples: Record<string, string> = {};
for (const d of corpus.docs) {
  if (!d.json || !d.html) continue;
  const schema = schemaFor(d.variant);
  const node = PMNode.fromJSON(schema, d.json);
  const browser = getHTMLFromFragment(node.content, schema);
  total++;
  if (browser === d.html) {
    same++;
    continue;
  }
  const r: string[] = [];
  if (/ style="/.test(browser) && !/ style="/.test(d.html)) r.push("style attribute present only in browser html");
  if (/&apos;/.test(d.html)) r.push("zeed-dom escapes ' as &apos;");
  if (/&quot;/.test(d.html) && !/&quot;/.test(browser)) r.push("zeed-dom escapes \" in text");
  if (/[A-Z]/.test((d.html.match(/<[^>]+>/g) ?? []).join("")) && !/[A-Z]/.test((browser.match(/<[^>]+>/g) ?? []).join(""))) r.push("attribute name case (browser lower-cases)");
  if (/data-checked(?=[ >])/.test(d.html) && /data-checked="/.test(browser)) r.push("boolean attribute form (data-checked)");
  if (!r.length) r.push("other");
  for (const k of r) {
    reasons[k] = (reasons[k] ?? 0) + 1;
    examples[k] ??= d.id;
  }
}
console.log(JSON.stringify({ total, identical: same, differing: total - same, reasons, example_doc_per_reason: examples }, null, 1));
