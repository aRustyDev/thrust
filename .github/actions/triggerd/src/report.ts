import { createHash } from "node:crypto";
import { TriggerdError } from "./errors.js";
import { LIMITS } from "./limits.js";
import type { Evaluation } from "./evaluate.js";

type Source = {
  readonly repository: string;
  readonly adapter: "pr-api" | "pr-git" | "git" | "full";
  readonly before: string | null;
  readonly base: string | null;
  readonly head: string | null;
  readonly mergeBase: string | null;
  readonly policyRevision: string;
  readonly actionRevision: string;
  readonly testRevision: string;
};
export type ReportContext = { readonly scope: string; readonly event: string; readonly activity: string; readonly source: Source };
export type SerializedEvaluation = { readonly rules: string; readonly details: string; readonly summary: string };

const compare = (left: string, right: string): number => left < right ? -1 : left > right ? 1 : 0;
const fail = (): never => { throw new TriggerdError("OUTPUT_INVALID", "Evaluation output invalid"); };

function canonical(value: unknown): unknown {
  if (Array.isArray(value)) return value.map(canonical);
  if (!value || typeof value !== "object") return value;
  return Object.fromEntries(Object.keys(value).sort(compare).map((key) => [key, canonical((value as Record<string, unknown>)[key])]));
}

function validContext(context: ReportContext): boolean {
  if (!context || typeof context !== "object") return false;
  const source = context.source;
  if (!source || typeof source !== "object") return false;
  const sha = /^[0-9a-f]{40}$/;
  return typeof context.scope === "string" && typeof context.event === "string" && typeof context.activity === "string" && !/[<&:]/.test(context.activity)
    && typeof source?.repository === "string" && /^(pr-api|pr-git|git|full)$/.test(source.adapter)
    && [source.before, source.base, source.head, source.mergeBase].every((value) => value === null || sha.test(value))
    && [source.policyRevision, source.actionRevision, source.testRevision].every((value) => sha.test(value));
}

export function serializeEvaluation(evaluation: Evaluation, rawPolicy: Uint8Array, context: ReportContext): SerializedEvaluation {
  if (!validContext(context)) fail();
  const rules = Object.fromEntries(Object.keys(evaluation.decisions).sort(compare).map((id) => [id, evaluation.decisions[id].selected]));
  const details = canonical({
    version: 1,
    scope: context.scope,
    event: context.event,
    activity: context.activity,
    source: context.source,
    configSha256: createHash("sha256").update(rawPolicy).digest("hex"),
    mode: evaluation.changeSet.kind === "complete" ? "selective" : "full",
    changeSet: evaluation.changeSet,
    decisions: evaluation.decisions,
    coverageWarnings: [...evaluation.coverageWarnings].sort(compare)
  });
  const rulesJson = JSON.stringify(rules);
  const detailsJson = JSON.stringify(details);
  const summary = Object.keys(rules).sort(compare).map((id) => `- ${id}: ${rules[id] ? "execute" : "not selected"}`).join("\n");
  if (Buffer.byteLength(rulesJson) + Buffer.byteLength(detailsJson) > LIMITS.machineOutputBytes || Buffer.byteLength(summary) > LIMITS.summaryBytes) fail();
  return Object.freeze({ rules: rulesJson, details: detailsJson, summary });
}
