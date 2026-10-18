// TEMPLATE (rust-test-logic), adapted from rust-testing-plan.zip, 2026-10-18: copy to legacy-oracle/ts-oracle.mjs; isqrt and ./legacy/isqrt.mjs are EXAMPLES: add one adapter per ported function and point the import at the built legacy package.
// JSON-lines bridge from Rust tests to the legacy TypeScript implementation.
//   Request:  {"fn": "isqrt", "args": ["16"]}
//   Response: {"ok": "4"}  or  {"error": "<kind or stack>"}
// One request per line, one response per line, in order. Exits when stdin closes.
import { createInterface } from "node:readline";
// Point this at the BUILT legacy package, e.g. "../legacy/dist/index.js".
import * as legacy from "./legacy/isqrt.mjs";

// Adapters keep the wire format simple: integers arrive and leave as strings.
const handlers = {
  isqrt: (n) => legacy.isqrt(BigInt(n)).toString(),
};

const rl = createInterface({ input: process.stdin });
rl.on("line", (line) => {
  let out;
  try {
    const { fn, args } = JSON.parse(line);
    const handler = handlers[fn];
    if (!handler) throw new Error(`unknown fn: ${fn}`);
    out = { ok: handler(...args) };
  } catch (e) {
    out = { error: String(e?.code ?? e?.stack ?? e) };
  }
  process.stdout.write(JSON.stringify(out) + "\n");
});
rl.on("close", () => process.exit(0));
