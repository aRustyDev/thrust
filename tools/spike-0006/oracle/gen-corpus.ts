// Spike 0006: generate the corpus of triples (ProseMirror JSON, HTML, Yjs update binary) from the REAL
// headless editor code. Output: one JSON document on stdout.
//
//   tools/spike-0006/run-oracle.sh <workdir> gen-corpus.ts > corpus.json
import { authoredDocs, foreignHtml } from "./docs";
import { historyDocs } from "./history";
import * as Y from "yjs";
import {
  b64,
  binaryToJson,
  canon,
  convertHTMLDocumentToAllFormats,
  docSchema,
  htmlToJson,
  jsonToBinary,
  jsonToHtml,
  richSchema,
  titleSchema,
  typesIn,
  unb64,
  validIn,
  ystructOfBinary,
} from "./lib";

type Entry = Record<string, unknown>;
const docs: Entry[] = [];
const problems: string[] = [];

// Large documents go to a separate (git-ignored) file: `gen-corpus.ts --large`.
const LARGE = new Set(["gen-huge", "many-paragraphs", "para-long-200k", "gen-large-1", "gen-large-2", "gen-large-3"]);
const wantLarge = process.argv.includes("--large");

// 1. authored documents ---------------------------------------------------------------------------
let n = 0;
for (const d of authoredDocs()) {
  n++;
  if (LARGE.has(d.id) !== wantLarge) continue;
  if (!validIn(docSchema, d.doc)) {
    problems.push(`${d.id}: authored JSON is not valid in the document schema`);
    continue;
  }
  const clientID = 1000 + n;
  const titleArg =
    d.title !== undefined
      ? {
          json: {
            type: "doc",
            content: [{ type: "heading", attrs: { level: 1 }, ...(d.title ? { content: [{ type: "text", text: d.title }] } : {}) }],
          },
          clientID: 500000 + n,
        }
      : undefined;
  const bin = jsonToBinary(docSchema, d.doc, clientID, titleArg);
  const json = binaryToJson(docSchema, bin);
  const html = jsonToHtml("document", json);
  const { nodes, marks } = typesIn(json);
  const e: Entry = {
    id: d.id,
    kind: "authored",
    desc: d.desc,
    variant: "document",
    rich_ok: validIn(richSchema, json as any),
    nodes: [...nodes].sort(),
    marks: [...marks].sort(),
    json,
    html,
    binary: b64(bin),
    ystruct: ystructOfBinary(bin),
    // html_in is omitted when it equals html (authored docs parse their own rendering)
    html_in_json: htmlToJson("document", html),
  };
  // the authored JSON differs from the stored JSON when the Y round trip normalises it (null -> default etc.)
  if (canon(d.doc) !== canon(json)) e.authored = d.doc;
  if (d.title !== undefined) {
    e.title = d.title;
    // getAllDocumentFormatsFromDocumentEditorBinaryData decodes the title fragment with the DOCUMENT schema
    e.title_json = binaryToJson(docSchema, bin, "title");
    e.title_ystruct = ystructOfBinary(bin, "title");
  }
  docs.push(e);
}

// 2. foreign HTML (what an API v1 client writes), run through the real convertHTMLDocumentToAllFormats -------
for (const f of wantLarge ? [] : foreignHtml()) {
  const variant = f.variant ?? "rich";
  const e: Entry = { id: f.id, kind: "foreign", desc: f.desc, variant, html_in: f.html };
  try {
    e.html_in_json = htmlToJson(variant, f.html);
  } catch (err) {
    e.oracle_error = `generateJSON: ${(err as Error).message}`;
    docs.push(e);
    continue;
  }
  try {
    // The real convertHTMLDocumentToAllFormats picks a random Yjs client id, so the bytes it returns differ run to
    // run. Reproduce its three steps with a fixed id (reproducible corpus) and assert json/html equal the real output.
    const schema = variant === "rich" ? richSchema : docSchema;
    const bin = jsonToBinary(schema, e.html_in_json as any, 7000 + docs.length);
    const json = binaryToJson(schema, bin);
    const html = jsonToHtml(variant, json);
    const real = convertHTMLDocumentToAllFormats({ document_html: f.html, variant });
    if (canon(real.description_json) !== canon(json) || real.description_html !== html) problems.push(`${f.id}: deterministic path differs from convertHTMLDocumentToAllFormats`);
    const { nodes, marks } = typesIn(json);
    Object.assign(e, {
      json,
      html,
      binary: b64(bin),
      ystruct: ystructOfBinary(bin),
      nodes: [...nodes].sort(),
      marks: [...marks].sort(),
      rich_ok: validIn(richSchema, json as any),
      valid: validIn(schema, json as any),
    });
  } catch (err) {
    e.oracle_error = `convertHTMLDocumentToAllFormats: ${(err as Error).message}`;
  }
  docs.push(e);
}

// 3. stored-like binaries with history ----------------------------------------------------------------------
for (const h of wantLarge ? [] : historyDocs()) {
  let json;
  try {
    json = binaryToJson(docSchema, h.binary);
  } catch (err) {
    problems.push(`${h.id}: oracle cannot decode its own history binary: ${(err as Error).message}`);
    continue;
  }
  const { nodes, marks } = typesIn(json);
  docs.push({
    id: h.id,
    kind: "history",
    desc: h.desc,
    variant: "document",
    nodes: [...nodes].sort(),
    marks: [...marks].sort(),
    json,
    html: jsonToHtml("document", json),
    binary: b64(h.binary),
    ystruct: ystructOfBinary(h.binary),
    html_in_json: htmlToJson("document", jsonToHtml("document", json)),
  });
}

// 4. "legacy" binaries: written by an editor whose schema differs from the current one (a different schema version).
//    Missing attributes must fall back to the current defaults; unknown nodes/marks must be dropped like y-prosemirror drops them.
if (!wantLarge) {
  const mk = (id: string, desc: string, clientID: number, build: (f: Y.XmlFragment) => void) => {
    const ydoc = new Y.Doc();
    ydoc.clientID = clientID;
    build(ydoc.getXmlFragment("default"));
    const bin = Y.encodeStateAsUpdate(ydoc);
    const json = binaryToJson(docSchema, bin);
    const { nodes, marks } = typesIn(json);
    docs.push({ id, kind: "legacy", desc, variant: "document", nodes: [...nodes].sort(), marks: [...marks].sort(), json, html: jsonToHtml("document", json), binary: b64(bin), html_in_json: htmlToJson("document", jsonToHtml("document", json)) });
  };
  const el = (name: string, attrs: Record<string, unknown>, kids: (Y.XmlElement | Y.XmlText)[]) => {
    const e = new Y.XmlElement(name);
    for (const [k, v] of Object.entries(attrs)) e.setAttribute(k, v as any);
    if (kids.length) e.insert(0, kids);
    return e;
  };
  const tx = (s: string, attrs?: Record<string, unknown>) => {
    const t = new Y.XmlText();
    t.insert(0, s, attrs as any);
    return t;
  };
  mk("legacy-missing-attrs", "elements written without any attributes (older schema): defaults must apply", 810001, (f) => {
    f.insert(0, [
      el("heading", {}, [tx("Heading with no level")]),
      el("paragraph", {}, [tx("plain ", {}), tx("link", { link: { href: "https://example.com" } }), tx(" colour", { customColor: {} })]),
      el("orderedList", {}, [el("listItem", {}, [el("paragraph", {}, [tx("one")])])]),
      el("taskList", {}, [el("taskItem", {}, [el("paragraph", {}, [tx("task")])])]),
      el("table", {}, [el("tableRow", {}, [el("tableHeader", {}, [el("paragraph", {}, [tx("h")])]), el("tableCell", {}, [el("paragraph", {}, [tx("c")])])])]),
      el("codeBlock", {}, [tx("code")]),
      el("imageComponent", {}, []),
      el("calloutComponent", {}, [el("paragraph", {}, [tx("callout")])]),
    ]);
  });
  mk("legacy-unknown-types", "unknown node types and unknown marks written by a newer editor", 810002, (f) => {
    f.insert(0, [
      el("paragraph", {}, [tx("kept")]),
      el("futureWidget", { a: 1 }, [el("paragraph", {}, [tx("dropped with its parent")])]),
      el("paragraph", {}, [tx("text with an unknown mark", { sparkle: { glitter: true } })]),
      el("paragraph", {}, [tx("also kept")]),
    ]);
  });
  mk("legacy-wrong-types", "attribute values of unexpected JSON types are kept verbatim (no coercion)", 810003, (f) => {
    f.insert(0, [
      el("heading", { level: "2", textAlign: 5 }, [tx("string level")]),
      el("orderedList", { start: "3", type: 1 }, [el("listItem", {}, [el("paragraph", {}, [tx("x")])])]),
      el("tableCell", { colspan: "2", colwidth: "100" }, []),
    ]);
  });
  mk("legacy-invalid-content", "elements whose content violates the schema are dropped (listItem outside a list, text in a doc)", 810004, (f) => {
    f.insert(0, [
      el("paragraph", {}, [tx("ok")]),
      el("listItem", {}, [el("paragraph", {}, [tx("orphan item")])]),
      el("bulletList", {}, [el("paragraph", {}, [tx("paragraph directly in a list")])]),
      el("paragraph", {}, [tx("last")]),
    ]);
  });
}

const versions = (name: string) => {
  try {
    // eslint-disable-next-line @typescript-eslint/no-var-requires
    return JSON.parse(require("node:fs").readFileSync(require.resolve(`${name}/package.json`), "utf8")).version;
  } catch {
    return "unknown";
  }
};

process.stdout.write(
  canon({
    meta: {
      generator: "tools/spike-0006/oracle/gen-corpus.ts",
      plane_pin: "oracle/v1-pin-20261018",
      note: "Synthetic content only. binary = Yjs update v1 (base64). json = ProseMirror JSON after the Y round trip (what the server stores).",
      node: process.version,
      counts: { total: docs.length, authored: docs.filter((d) => d.kind === "authored").length, foreign: docs.filter((d) => d.kind === "foreign").length, history: docs.filter((d) => d.kind === "history").length },
      problems,
    },
    docs,
  }) + "\n"
);
