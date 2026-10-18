// TEMPLATE (rust-test-logic), adapted from rust-testing-plan.zip, 2026-10-18; for scalar units (strings, integers) only: copy to legacy-oracle/gen-fixtures.mjs; isqrt and ./legacy/isqrt.mjs are EXAMPLES: substitute your unit, the built legacy package path and crates/your_crate.
// Records golden fixtures from the legacy TypeScript implementation.
//   Run from the repo root:  TZ=UTC node legacy-oracle/gen-fixtures.mjs
// Re-run only when the legacy behaviour is the agreed reference, then commit the output.
// Never hand-edit generated files; intended differences go in the ts-to-rust exception register.
import { execSync } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
// Point this at the BUILT legacy package, e.g. "../legacy/dist/index.js".
import * as legacy from "./legacy/isqrt.mjs";

const OUT = new URL("../crates/your_crate/tests/parity/fixtures/", import.meta.url);
const LEGACY_DIR = "legacy"; // git checkout of the TypeScript code (for provenance)

const legacyCommit = (() => {
  try {
    return execSync(`git -C ${LEGACY_DIR} rev-parse HEAD`, { stdio: ["ignore", "pipe", "ignore"] }).toString().trim();
  } catch {
    return "unknown";
  }
})();

// Deterministic PRNG: regenerating with the same legacy commit yields identical files.
function mulberry32(seed) {
  return () => {
    seed = (seed + 0x6d2b79f5) | 0;
    let t = Math.imul(seed ^ (seed >>> 15), 1 | seed);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}
const rand = mulberry32(0x5eed);
const u32 = () => BigInt(Math.floor(rand() * 2 ** 32));
const randomU64 = () => (u32() << 32n) | u32();

// Record what the legacy code does, including failures (error kind/code, never message text).
// Integers are written as strings: JS numbers lose precision above 2^53.
function record(fn, input) {
  try {
    return { input: String(input), expected: String(fn(input)) };
  } catch (e) {
    return { input: String(input), error: String(e?.code ?? e?.name ?? "Error") };
  }
}

function write(name, cases) {
  mkdirSync(OUT, { recursive: true });
  const fixture = {
    source: { generator: "legacy-oracle/gen-fixtures.mjs", legacy_commit: legacyCommit },
    cases,
  };
  writeFileSync(new URL(name, OUT), JSON.stringify(fixture, null, 2) + "\n");
  console.log(`${name}: ${cases.length} cases`);
}

// ---- one block per ported unit ---------------------------------------------------------
const isqrt = (n) => legacy.isqrt(BigInt(n));
const boundaries = [0n, 1n, 2n, 3n, 4n, 15n, 16n, 17n, 2n ** 32n - 1n, 2n ** 53n + 1n, 2n ** 64n - 1n];
write("isqrt-001-boundaries.json", boundaries.map((n) => record(isqrt, n)));
write("isqrt-002-random.json", Array.from({ length: 200 }, () => record(isqrt, randomU64())));
