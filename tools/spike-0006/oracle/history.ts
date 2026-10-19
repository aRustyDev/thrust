// Spike 0006: "stored-like" Yjs binaries. A fresh prosemirrorToYXmlFragment() encoding has no tombstones, one
// client and no formatting churn; a binary that a collab server persisted has all of those. These sessions
// replay many small ProseMirror edits through y-prosemirror's own updateYFragment (the function the sync plugin
// calls on every keystroke), optionally with two concurrent clients, then encode the full state.
import type { JSONContent as J } from "@tiptap/core";
import { Node as PMNode, Fragment } from "@tiptap/pm/model";
import type { Schema } from "@tiptap/pm/model";
import { Transform } from "@tiptap/pm/transform";
import { updateYFragment } from "y-prosemirror";
import * as Y from "yjs";
import { authoredDocs } from "./docs";
import { docSchema } from "./lib";

function rng(seed: number) {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let x = Math.imul(a ^ (a >>> 15), 1 | a);
    x = (x + Math.imul(x ^ (x >>> 7), 61 | x)) ^ x;
    return ((x ^ (x >>> 14)) >>> 0) / 4294967296;
  };
}
const WORDS = "lorem ipsum dolor sit amet consectetur adipiscing elit sed do eiusmod tempor incididunt ut labore".split(" ");
const MARK_NAMES = ["bold", "italic", "strike", "underline", "code"];

function randomEdit(doc: PMNode, r: () => number, schema: Schema): PMNode {
  const blocks: { pos: number; node: PMNode }[] = [];
  doc.descendants((n, pos) => {
    if (n.isTextblock && n.type.name !== "codeBlock") blocks.push({ pos, node: n });
  });
  if (!blocks.length) return doc;
  const pick = blocks[Math.floor(r() * blocks.length)];
  const base = pick.pos + 1;
  const size = pick.node.content.size;
  const off = Math.floor(r() * (size + 1));
  const tr = new Transform(doc);
  try {
    const kind = r();
    if (kind < 0.34) {
      const word = (size ? " " : "") + WORDS[Math.floor(r() * WORDS.length)];
      const marks = doc.resolve(base + off).marks();
      tr.insert(base + off, schema.text(word, marks));
    } else if (kind < 0.5 && size > 2) {
      const a = Math.floor(r() * (size - 1));
      const b = Math.min(size, a + 1 + Math.floor(r() * 6));
      tr.delete(base + a, base + b);
    } else if (kind < 0.66 && size > 2) {
      const a = Math.floor(r() * (size - 1));
      const b = Math.min(size, a + 1 + Math.floor(r() * 8));
      const mt = schema.marks[MARK_NAMES[Math.floor(r() * MARK_NAMES.length)]];
      if (r() < 0.7) tr.addMark(base + a, base + b, mt.create());
      else tr.removeMark(base + a, base + b, mt);
    } else if (kind < 0.76) {
      tr.split(base + off);
    } else if (kind < 0.84 && doc.childCount > 1) {
      // delete the whole top-level block that contains the pick
      const $p = doc.resolve(base);
      const start = $p.before(1);
      tr.delete(start, start + $p.node(1).nodeSize);
    } else if (kind < 0.92 && pick.node.type.name === "heading") {
      tr.setNodeMarkup(pick.pos, undefined, { ...pick.node.attrs, level: 1 + Math.floor(r() * 6) });
    } else {
      // insert a new paragraph after the top-level block
      const $p = doc.resolve(base);
      const after = $p.after(1);
      tr.insert(after, schema.nodes.paragraph.create(null, schema.text(WORDS[Math.floor(r() * WORDS.length)])));
    }
  } catch {
    return doc;
  }
  try {
    tr.doc.check();
  } catch {
    return doc;
  }
  return tr.doc;
}

type Client = { ydoc: Y.Doc; frag: Y.XmlFragment; meta: any };
function mkClient(clientID: number, base?: Uint8Array): Client {
  const ydoc = new Y.Doc();
  ydoc.clientID = clientID;
  if (base) Y.applyUpdate(ydoc, base);
  return { ydoc, frag: ydoc.get("default", Y.XmlFragment) as Y.XmlFragment, meta: { mapping: new Map(), isOMark: new Map() } };
}
const push = (c: Client, node: PMNode) => updateYFragment(c.ydoc, c.frag, node, c.meta);

export type HistoryDoc = { id: string; desc: string; binary: Uint8Array };

export function historyDocs(): HistoryDoc[] {
  const out: HistoryDoc[] = [];
  const seeds = authoredDocs().filter((d) =>
    ["para-two", "list-nested-3", "task-nested", "table-2x2", "mark-overlap", "real-bug-report", "real-spec-page", "real-meeting-notes", "callout-blocks", "quote-nested", "gen-small-3", "gen-small-7", "gen-medium-2", "gen-medium-5"].includes(d.id)
  );
  let cid = 90000;
  // single-client sessions of increasing length
  for (const seed of seeds) {
    for (const steps of [15, 60]) {
      const r = rng(steps * 131 + seed.id.length * 7 + cid);
      const c = mkClient(cid++);
      let cur = PMNode.fromJSON(docSchema, seed.doc);
      push(c, cur);
      for (let i = 0; i < steps; i++) {
        cur = randomEdit(cur, r, docSchema);
        push(c, cur);
      }
      out.push({ id: `hist-${seed.id}-s${steps}`, desc: `${steps} edits replayed through updateYFragment from ${seed.id} (single client, tombstones kept)`, binary: Y.encodeStateAsUpdate(c.ydoc) });
    }
  }
  // two concurrent clients on a shared base
  for (const seed of seeds.slice(0, 8)) {
    const r = rng(seed.id.length * 977 + cid);
    const a0 = mkClient(cid++);
    let curA = PMNode.fromJSON(docSchema, seed.doc);
    push(a0, curA);
    const base = Y.encodeStateAsUpdate(a0.ydoc);
    const b0 = mkClient(cid++, base);
    let curB = PMNode.fromJSON(docSchema, JSON.parse(JSON.stringify(seed.doc)));
    push(b0, curB); // no-op-ish: aligns mapping with the shared content
    for (let i = 0; i < 12; i++) {
      curA = randomEdit(curA, r, docSchema);
      push(a0, curA);
      curB = randomEdit(curB, r, docSchema);
      push(b0, curB);
    }
    Y.applyUpdate(a0.ydoc, Y.encodeStateAsUpdate(b0.ydoc));
    Y.applyUpdate(b0.ydoc, Y.encodeStateAsUpdate(a0.ydoc));
    out.push({ id: `hist-${seed.id}-2clients`, desc: `two concurrent clients, 12 edits each, merged (from ${seed.id})`, binary: Y.encodeStateAsUpdate(a0.ydoc) });
  }
  // incremental updates concatenated (what a WAL replays): merge of many small updates
  for (const seed of seeds.slice(0, 4)) {
    const r = rng(seed.id.length * 31 + cid);
    const c = mkClient(cid++);
    const updates: Uint8Array[] = [];
    c.ydoc.on("update", (u: Uint8Array) => updates.push(u));
    let cur = PMNode.fromJSON(docSchema, seed.doc);
    push(c, cur);
    for (let i = 0; i < 25; i++) {
      cur = randomEdit(cur, r, docSchema);
      push(c, cur);
    }
    out.push({ id: `hist-${seed.id}-wal`, desc: `${updates.length} incremental updates merged with Y.mergeUpdates (WAL replay shape)`, binary: Y.mergeUpdates(updates) });
  }
  return out;
}
