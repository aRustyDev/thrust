import { parentPort } from "node:worker_threads";
import { evaluatePolicy, type Context } from "./evaluate.js";
import type { ChangeRecord } from "./changes.js";

if (!parentPort) throw new Error("triggerd worker requires a parent port");
parentPort.on("message", (input: { policy: Record<string, any>; context: Context; changes: readonly ChangeRecord[] | null }) => {
  try { parentPort?.postMessage({ ok: true, evaluation: evaluatePolicy(input.policy, input.context, input.changes), rss: process.memoryUsage().rss }); }
  catch { parentPort?.postMessage({ ok: false }); }
});
