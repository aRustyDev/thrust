// Spike 0006: CPU and latency of the editor conversions inside local workerd.
//
//   node bench-worker.mjs <base-url> <label> <corpus.json> <corpus-large.json> [first-request-ms-file]
//
// workerd freezes Date.now()/performance.now() during synchronous work, so the Worker cannot time itself. The client
// asks the Worker to repeat an operation n times inside ONE request and divides the request time by n, after
// subtracting the same request with the work disabled (/noop, same body). Reported: median of 7 requests.
import { readFileSync } from "node:fs";

const [, , base, label, corpusPath, largePath] = process.argv;
const corpus = JSON.parse(readFileSync(corpusPath, "utf8"));
let docs = new Map(corpus.docs.map((d) => [d.id, d]));
try {
  for (const d of JSON.parse(readFileSync(largePath, "utf8")).docs) docs.set(d.id, d);
} catch {
  /* large corpus is optional */
}

async function post(path, body) {
  const t0 = performance.now();
  const r = await fetch(base + path, { method: "POST", body });
  const text = await r.text();
  const ms = performance.now() - t0;
  if (!r.ok) throw new Error(`${path} -> ${r.status}: ${text.slice(0, 300)}`);
  return { ms, text };
}
const median = (a) => [...a].sort((x, y) => x - y)[Math.floor(a.length / 2)];

async function measure(op, variant, body, extra = "") {
  const url = (n) => `/${op}?variant=${variant}&n=${n}${extra}`;
  const noop = (n) => `/noop?variant=${variant}&n=${n}`;
  // calibrate: grow n by 4x until the work (request minus the same request with no work) takes >= 150 ms
  await post(url(1), body); // warm the isolate and its caches
  let n = 1;
  for (;;) {
    const w = (await post(url(n), body)).ms - (await post(noop(n), body)).ms;
    if (w >= 150 || n >= 100000) break;
    n *= w < 5 ? 16 : 4;
  }
  const times = [];
  const nt = [];
  for (let i = 0; i < 7; i++) {
    times.push((await post(url(n), body)).ms);
    nt.push((await post(noop(n), body)).ms);
  }
  const perOp = (median(times) - median(nt)) / n;
  return { n, request_ms: +median(times).toFixed(2), noop_ms: +median(nt).toFixed(2), per_op_ms: +perOp.toFixed(4) };
}

const plan = JSON.parse(process.env.BENCH_PLAN ?? "null") ?? [
  // [doc id, variant, ops]
  ["para-plain", "rich", ["html", "binary"]],
  ["real-bug-report", "rich", ["html", "binary"]],
  ["real-spec-page", "document", ["html", "binary"]],
  ["gen-medium-12", "document", ["html", "binary"]],
  ["gen-large-1", "document", ["html", "binary"]],
  ["many-paragraphs", "document", ["html", "binary"]],
  ["gen-huge", "document", ["html", "binary"]],
];

const results = [];
for (const [id, variant, ops] of plan) {
  const d = docs.get(id);
  if (!d) continue;
  for (const op of ops) {
    try {
      if (op === "html") {
        const r = await measure("html-to-all", variant, d.html);
        results.push({ label, doc: id, op: "html -> json+binary+html (v1 write)", html_bytes: d.html.length, ...r });
      } else if (op === "htmllean") {
        const r = await measure("html-to-all", variant, d.html, "&lean=1");
        results.push({ label, doc: id, op: "html -> json+binary+html, no sanitise (= convertHTMLDocumentToAllFormats)", html_bytes: d.html.length, ...r });
      } else if (op === "binary") {
        const r = await measure("binary-to-all", variant, d.binary);
        results.push({ label, doc: id, op: "binary -> json+html (collab projection)", binary_b64_bytes: d.binary.length, ...r });
      } else if (op === "sanitize") {
        const r = await measure("sanitize", variant, d.html);
        results.push({ label, doc: id, op: "sanitize html", html_bytes: d.html.length, ...r });
      }
    } catch (e) {
      results.push({ label, doc: id, op, error: String(e).slice(0, 300) });
    }
  }
}
console.log(JSON.stringify(results, null, 1));
