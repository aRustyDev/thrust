// Spike 0006: dump the REAL ProseMirror schema built from @plane/editor's headless extension sets.
// Runs inside <workdir>/plane-copy/packages/editor/spike0006/ (see tools/spike-0006/setup-oracle.sh).
//   node <tsx> --tsconfig ../tsconfig.json schema-dump.ts > schema.json
import { getSchema } from "@tiptap/core";
import type { Schema, NodeType, MarkType } from "@tiptap/pm/model";
import { CoreEditorExtensionsWithoutProps, DocumentEditorExtensionsWithoutProps } from "@/extensions/core-without-props";
import { TITLE_EDITOR_EXTENSIONS } from "@/helpers/yjs-utils";

function attrsOf(t: NodeType | MarkType) {
  const out: Record<string, unknown> = {};
  for (const [k, a] of Object.entries(t.spec.attrs ?? (t as any).attrs ?? {})) {
    const hasDefault = Object.prototype.hasOwnProperty.call(a as object, "default");
    out[k] = hasDefault ? { default: (a as any).default === undefined ? "__undefined__" : (a as any).default } : { required: true };
  }
  return out;
}

function parseRules(spec: any) {
  return (spec.parseDOM ?? []).map((r: any) => ({
    tag: r.tag ?? null,
    style: r.style ?? null,
    priority: r.priority ?? null,
    preserveWhitespace: r.preserveWhitespace ?? null,
    context: r.context ?? null,
    hasGetAttrs: typeof r.getAttrs === "function",
    ignore: r.ignore ?? null,
    skip: r.skip ?? null,
    consuming: r.consuming ?? null,
  }));
}

function dump(schema: Schema) {
  const nodes: Record<string, unknown> = {};
  for (const [name, t] of Object.entries(schema.nodes)) {
    const s: any = t.spec;
    nodes[name] = {
      content: s.content ?? null,
      group: s.group ?? null,
      marks: s.marks ?? null,
      inline: !!t.isInline,
      atom: !!t.isAtom,
      leaf: !!t.isLeaf,
      textblock: !!t.isTextblock,
      code: !!s.code,
      defining: !!s.defining,
      isolating: !!s.isolating,
      whitespace: s.whitespace ?? null,
      attrs: attrsOf(t),
      parseDOM: parseRules(s),
      hasToDOM: typeof s.toDOM === "function",
    };
  }
  const marks: Record<string, unknown> = {};
  for (const [name, t] of Object.entries(schema.marks)) {
    const s: any = t.spec;
    marks[name] = {
      excludes: s.excludes ?? null,
      inclusive: s.inclusive ?? null,
      group: s.group ?? null,
      spanning: s.spanning ?? null,
      attrs: attrsOf(t),
      rank: (t as any).rank,
      parseDOM: parseRules(s),
    };
  }
  return { topNode: schema.topNodeType.name, nodes, marks };
}

const rich = getSchema(CoreEditorExtensionsWithoutProps);
const document = getSchema([...CoreEditorExtensionsWithoutProps, ...DocumentEditorExtensionsWithoutProps]);
const title = getSchema(TITLE_EDITOR_EXTENSIONS);
console.log(JSON.stringify({ rich: dump(rich), document: dump(document), title: dump(title) }, null, 1));
