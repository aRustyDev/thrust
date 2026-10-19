// Spike 0006: differential fuzz corpus for HTML -> ProseMirror JSON. Mutates the corpus HTML (and the foreign API-client
// snippets) in ways real writers get wrong: dropped/swapped tags, wrong nesting, stray text, odd attributes, truncation.
// The REAL generateJSON (zeed-dom + ProseMirror DOMParser) produces the expected output for each input.
//
//   tools/spike-0006/run-oracle.sh <workdir> fuzz-html.ts [count] [seed] > fuzz-html.json
import { authoredDocs, foreignHtml } from "./docs";
import { b64, binaryToJson, canon, docSchema, htmlToJson, jsonToBinary, jsonToHtml, richSchema, ystructOfBinary } from "./lib";

const COUNT = Number(process.argv[2] ?? 1500);
const SEED = Number(process.argv[3] ?? 20261018);

function rng(seed: number) {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let x = Math.imul(a ^ (a >>> 15), 1 | a);
    x = (x + Math.imul(x ^ (x >>> 7), 61 | x)) ^ x;
    return ((x ^ (x >>> 14)) >>> 0) / 4294967296;
  };
}
const r = rng(SEED);
const pick = <T>(a: T[]): T => a[Math.floor(r() * a.length)];

// seeds: small authored docs (their server HTML) + foreign snippets
const seeds: string[] = [];
let n = 0;
for (const d of authoredDocs()) {
  if (d.id.startsWith("gen-") && !d.id.startsWith("gen-small")) continue;
  if (["para-long-200k", "many-paragraphs", "gen-huge", "table-wide-10x10", "code-long", "para-long-10k", "quote-deep-40"].includes(d.id)) continue;
  const bin = jsonToBinary(docSchema, d.doc, 100 + n++);
  seeds.push(jsonToHtml("document", binaryToJson(docSchema, bin)));
}
for (const f of foreignHtml()) seeds.push(f.html);

const TAGS = ["p", "div", "span", "ul", "ol", "li", "h1", "h2", "h3", "strong", "em", "b", "i", "u", "s", "a", "pre", "code", "table", "tr", "td", "th", "blockquote", "br", "hr", "img", "label", "input", "font", "section", "mention-component", "image-component", "issue-embed-component"];
const FRAGS = ["text", " ", "\n", "&amp;", "&lt;", "&nbsp;", "&copy;", "<!-- c -->", "<b>", "</b>", "<p>", "</p>", "<br>", "<li>", "</div>", "<td>", "x y z", "\t", "</ul>", "<ul>"];
const ATTRS = [' style="font-weight: bold"', ' style="text-align: center"', ' style="color: red"', ' class="x"', ' data-text-color="gray"', ' data-type="taskItem"', ' data-checked="true"', ' href="https://e.x"', ' style="text-decoration: underline"', ' style="font-style: italic"', ' colspan="2"', ' start="3"', ' id="a"', ' hidden'];

function tokens(s: string): string[] {
  return s.match(/<[^>]*>|[^<]+/g) ?? [s];
}

function mutate(html: string): string {
  const t = tokens(html);
  const ops = 1 + Math.floor(r() * 3);
  let out = t.slice();
  for (let k = 0; k < ops; k++) {
    const op = Math.floor(r() * 10);
    const i = Math.floor(r() * Math.max(1, out.length));
    if (op === 0) out.splice(i, 1);
    else if (op === 1 && out.length > 1) {
      const j = Math.min(out.length - 1, i + 1);
      [out[i], out[j]] = [out[j], out[i]];
    } else if (op === 2) out.splice(i, 0, pick(FRAGS));
    else if (op === 3) {
      const tag = pick(TAGS);
      out = [`<${tag}>`, ...out, `</${tag}>`];
    } else if (op === 4 && out[i]?.startsWith("<") && !out[i].startsWith("</") && !out[i].startsWith("<!")) {
      out[i] = out[i].replace(/^<([^\s>/]+)/, `<${pick(TAGS)}`);
    } else if (op === 5 && out[i]?.startsWith("<") && !out[i].startsWith("</")) {
      out[i] = out[i].replace(/^<([^\s>/]+)/, (m) => m + pick(ATTRS));
    } else if (op === 6) {
      out = out.slice(0, Math.max(1, i));
    } else if (op === 7) {
      out.splice(i, 0, ...out.slice(i, i + 3));
    } else if (op === 8 && out[i] && !out[i].startsWith("<")) {
      const cps = Array.from(out[i]); // code-point aware: never split a surrogate pair
      const p = Math.floor(r() * (cps.length + 1));
      out[i] = cps.slice(0, p).join("") + pick(FRAGS) + cps.slice(p).join("");
    } else if (op === 9) {
      out.splice(i, 0, pick(seeds));
    }
  }
  return out.join("");
}

const rows: unknown[] = [];
for (let k = 0; k < COUNT; k++) {
  const html = mutate(pick(seeds));
  const variant = r() < 0.8 ? "rich" : "document";
  const row: Record<string, unknown> = { id: `fz-${k}`, variant, html };
  try {
    const json = htmlToJson(variant as any, html);
    row.json = json;
    // downstream of the parse, as the v1 write path does: binary (fixed client id), stored json, server html
    const schema = variant === "rich" ? richSchema : docSchema;
    const bin = jsonToBinary(schema, json, 900000 + k);
    row.binary = b64(bin);
    row.stored_json = binaryToJson(schema, bin);
    row.stored_html = jsonToHtml(variant as any, row.stored_json as any);
    row.ystruct = ystructOfBinary(bin);
  } catch (e) {
    row.error = (e as Error).message;
  }
  rows.push(row);
}
process.stdout.write(canon({ meta: { generator: "fuzz-html.ts", count: COUNT, seed: SEED, seeds: seeds.length }, rows }) + "\n");
