// Spike 0006: nesting-depth behaviour of the REAL code. Where does the JS stack give out, and what does it return up to there?
//
//   tools/spike-0006/run-oracle.sh <workdir> deep.ts > deep.json
// Result bodies are summarised (depth, node count) because JSON.stringify itself overflows the stack on deep trees.
import { binaryToJson, htmlToJson, jsonToBinary, jsonToHtml, richSchema } from "./lib";

const depths = [10, 50, 100, 127, 128, 129, 200, 500, 1000, 2000, 3000, 5000, 8000];
const shapes: Record<string, (d: number) => string> = {
  blockquote: (d) => "<blockquote>".repeat(d) + "<p>x</p>" + "</blockquote>".repeat(d),
  list: (d) => "<ul><li>".repeat(d) + "x" + "</li></ul>".repeat(d),
  div: (d) => "<div>".repeat(d) + "<p>x</p>" + "</div>".repeat(d),
  inline: (d) => "<p>" + "<strong><em>".repeat(d) + "x" + "</em></strong>".repeat(d) + "</p>",
};

function summary(json: any): { depth: number; nodes: number } {
  let depth = 0;
  let nodes = 0;
  const stack: [any, number][] = [[json, 1]];
  while (stack.length) {
    const [n, d] = stack.pop()!;
    nodes++;
    if (d > depth) depth = d;
    for (const c of n.content ?? []) stack.push([c, d + 1]);
  }
  return { depth, nodes };
}

const rows: unknown[] = [];
for (const [shape, f] of Object.entries(shapes)) {
  for (const d of depths) {
    const html = f(d);
    const row: Record<string, unknown> = { shape, depth: d };
    try {
      const json = htmlToJson("rich", html);
      row.parse = "ok";
      row.parse_summary = summary(json);
      try {
        const bin = jsonToBinary(richSchema, json, 700000 + d);
        const stored = binaryToJson(richSchema, bin);
        const h = jsonToHtml("rich", stored);
        row.pipeline = "ok";
        row.stored_summary = summary(stored);
        row.html_bytes = h.length;
      } catch (e) {
        row.pipeline = `error: ${(e as Error).message.slice(0, 80)}`;
      }
    } catch (e) {
      row.parse = `error: ${(e as Error).message.slice(0, 80)}`;
    }
    rows.push(row);
  }
}
process.stdout.write(JSON.stringify(rows) + "\n");
