// Spike 0006 oracle library: the REAL headless conversion code of @plane/editor, re-exposed with
// deterministic Yjs client ids so corpus generation is reproducible byte for byte.
//
// Everything here calls the same functions apps/live and the web app call:
//   @tiptap/html generateHTML/generateJSON, y-prosemirror prosemirrorToYXmlFragment /
//   yXmlFragmentToProseMirrorRootNode, and the extension arrays from core-without-props.
import { getSchema } from "@tiptap/core";
import type { JSONContent } from "@tiptap/core";
import { generateHTML, generateJSON } from "@tiptap/html";
import { Node as PMNode } from "@tiptap/pm/model";
import type { Schema } from "@tiptap/pm/model";
import { prosemirrorToYXmlFragment, yXmlFragmentToProseMirrorRootNode } from "y-prosemirror";
import * as Y from "yjs";
import { CoreEditorExtensionsWithoutProps, DocumentEditorExtensionsWithoutProps } from "@/extensions/core-without-props";
import {
  TITLE_EDITOR_EXTENSIONS,
  getBinaryDataFromDocumentEditorHTMLString,
  getBinaryDataFromRichTextEditorHTMLString,
  getAllDocumentFormatsFromDocumentEditorBinaryData,
  getAllDocumentFormatsFromRichTextEditorBinaryData,
  convertHTMLDocumentToAllFormats,
} from "@/helpers/yjs-utils";

export type Variant = "rich" | "document";

export const RICH_EXT = CoreEditorExtensionsWithoutProps;
export const DOC_EXT = [...CoreEditorExtensionsWithoutProps, ...DocumentEditorExtensionsWithoutProps];
export const richSchema: Schema = getSchema(RICH_EXT);
export const docSchema: Schema = getSchema(DOC_EXT);
export const titleSchema: Schema = getSchema(TITLE_EDITOR_EXTENSIONS);

export const extFor = (v: Variant) => (v === "rich" ? RICH_EXT : DOC_EXT);
export const schemaFor = (v: Variant) => (v === "rich" ? richSchema : docSchema);

export {
  getBinaryDataFromDocumentEditorHTMLString,
  getBinaryDataFromRichTextEditorHTMLString,
  getAllDocumentFormatsFromDocumentEditorBinaryData,
  getAllDocumentFormatsFromRichTextEditorBinaryData,
  convertHTMLDocumentToAllFormats,
};

/** ProseMirror JSON -> Yjs update (v1), same code path as prosemirrorJSONToYDoc but with a fixed client id. */
export function jsonToBinary(schema: Schema, json: JSONContent, clientID: number, titleJson?: { json: JSONContent; clientID: number }): Uint8Array {
  const ydoc = new Y.Doc();
  ydoc.clientID = clientID;
  const frag = ydoc.get("default", Y.XmlFragment) as Y.XmlFragment;
  prosemirrorToYXmlFragment(PMNode.fromJSON(schema, json), frag);
  if (titleJson) {
    const tdoc = new Y.Doc();
    tdoc.clientID = titleJson.clientID;
    const tfrag = tdoc.get("title", Y.XmlFragment) as Y.XmlFragment;
    prosemirrorToYXmlFragment(PMNode.fromJSON(schema, titleJson.json), tfrag);
    Y.applyUpdate(ydoc, Y.encodeStateAsUpdate(tdoc));
  }
  return Y.encodeStateAsUpdate(ydoc);
}

/** Yjs update -> ProseMirror JSON (what getAllDocumentFormats...BinaryData does). */
export function binaryToJson(schema: Schema, bin: Uint8Array, fragment = "default"): JSONContent {
  const ydoc = new Y.Doc();
  Y.applyUpdate(ydoc, bin);
  return yXmlFragmentToProseMirrorRootNode(ydoc.getXmlFragment(fragment), schema).toJSON();
}

export const jsonToHtml = (v: Variant, json: JSONContent): string => generateHTML(json, extFor(v));
export const htmlToJson = (v: Variant, html: string): JSONContent => generateJSON(html, extFor(v));

/** Raw Yjs structure of a fragment: element names, attributes (as stored) and XmlText deltas. */
export function ystruct(x: Y.XmlFragment | Y.XmlElement): unknown {
  const kids = x.toArray().map((c: any) => {
    if (c instanceof Y.XmlText) return { text: c.toDelta() };
    if (c instanceof Y.XmlElement) return ystruct(c);
    return { other: String(c) };
  });
  if (x instanceof Y.XmlElement) {
    const attrs = x.getAttributes() as Record<string, unknown>;
    const a: Record<string, unknown> = {};
    // undefined-valued attributes are dropped by JSON; keep them visible as a marker
    for (const k of Object.keys(attrs).sort()) a[k] = attrs[k] === undefined ? "__undefined__" : attrs[k];
    return { el: x.nodeName, attrs: a, children: kids };
  }
  return { fragment: true, children: kids };
}

export function ystructOfBinary(bin: Uint8Array, fragment = "default") {
  const ydoc = new Y.Doc();
  Y.applyUpdate(ydoc, bin);
  return ystruct(ydoc.getXmlFragment(fragment));
}

export const b64 = (u: Uint8Array) => Buffer.from(u).toString("base64");
export const unb64 = (s: string) => new Uint8Array(Buffer.from(s, "base64"));

/** Sorted-key JSON, for stable comparisons and stable corpus files. */
export function canon(x: unknown): string {
  return JSON.stringify(x, (_k, v) => {
    if (v && typeof v === "object" && !Array.isArray(v)) {
      const o: Record<string, unknown> = {};
      for (const k of Object.keys(v).sort()) o[k] = (v as any)[k];
      return o;
    }
    return v;
  });
}

/** Node and mark type names present in a ProseMirror JSON tree. */
export function typesIn(json: any, nodes = new Set<string>(), marks = new Set<string>()) {
  if (!json || typeof json !== "object") return { nodes, marks };
  if (json.type) nodes.add(json.type);
  for (const m of json.marks ?? []) marks.add(m.type);
  for (const c of json.content ?? []) typesIn(c, nodes, marks);
  return { nodes, marks };
}

export function validIn(schema: Schema, json: JSONContent): boolean {
  try {
    PMNode.fromJSON(schema, json).check();
    return true;
  } catch {
    return false;
  }
}
