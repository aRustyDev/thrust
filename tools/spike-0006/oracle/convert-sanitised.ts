// Spike 0006: the /api/v1 write path end to end in the ORACLE stack: Plane's nh3 output (from xss-oracle.json) converted
// by the real convertHTMLDocumentToAllFormats steps. The Rust side runs ammonia -> parse -> yjs -> json/html on the same
// raw inputs and compares.
//
//   tools/spike-0006/run-oracle.sh <workdir> convert-sanitised.ts <xss-oracle.json> <corpus.json> > sanitised-converted.json
import { readFileSync } from "node:fs";
import { binaryToJson, canon, htmlToJson, jsonToBinary, jsonToHtml, schemaFor } from "./lib";

const oracle = JSON.parse(readFileSync(process.argv[2], "utf8")).results;
const corpus = JSON.parse(readFileSync(process.argv[3], "utf8"));
const variantOf = new Map<string, "rich" | "document">(corpus.docs.map((d: any) => [`doc:${d.id}`, d.variant]));
const out: Record<string, unknown> = {};
let i = 0;
for (const [k, v] of Object.entries<any>(oracle)) {
  const clean = v.app.clean;
  if (!v.app.valid || !clean) continue;
  const variant = variantOf.get(k) ?? "rich";
  try {
    const schema = schemaFor(variant);
    const json = htmlToJson(variant, clean);
    const bin = jsonToBinary(schema, json, 600000 + i++);
    const stored = binaryToJson(schema, bin);
    out[k] = { variant, json: stored, html: jsonToHtml(variant, stored) };
  } catch (e) {
    out[k] = { variant, error: String(e).slice(0, 200) };
  }
}
process.stdout.write(canon(out) + "\n");
