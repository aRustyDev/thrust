import { TriggerdError } from "./errors.js";
import type { Context } from "./evaluate.js";

type RecordValue = Record<string, unknown>;
export type RuntimeContext = Context & {
  readonly activity: string;
  readonly prNumber: number | null;
  readonly changedFiles: number | null;
  readonly source: {
    readonly repository: string;
    readonly adapter: "pr-api" | "pr-git" | "git" | "full";
    readonly before: string | null;
    readonly base: string | null;
    readonly head: string;
    readonly mergeBase: string | null;
    readonly policyRevision: string;
    readonly actionRevision: string;
    readonly testRevision: string;
  };
};

const sha = /^[0-9a-f]{40}$/;
const zero = /^0{40}$/;
const fail = (): never => { throw new TriggerdError("INPUT_INVALID", "Runtime context invalid"); };
const stale = (): never => { throw new TriggerdError("SOURCE_STALE", "Runtime source stale"); };
const record = (value: unknown): RecordValue => value && !Array.isArray(value) && typeof value === "object" ? value as RecordValue : fail();
const fullName = (value: unknown): string => typeof value === "string" && /^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/.test(value) ? value : fail();
const revision = (value: unknown): string => typeof value === "string" && sha.test(value) ? value : fail();

export function parseRuntimeContext(event: string, payload: unknown, checkoutRevision: string): RuntimeContext {
  const revisionId = revision(checkoutRevision);
  const root = record(payload);
  const repository = fullName(record(root.repository).full_name);
  if (event === "workflow_dispatch" || event === "schedule") {
    return Object.freeze({
      event, activity: "", scope: "", fullCheck: true,
      notCollectedReason: event === "workflow_dispatch" ? "manual" : "schedule",
      prNumber: null, changedFiles: null,
      source: Object.freeze({ repository, adapter: "full", before: null, base: null, head: revisionId, mergeBase: null, policyRevision: revisionId, actionRevision: revisionId, testRevision: revisionId })
    });
  }
  if (event === "push") {
    if (root.deleted !== false || typeof root.ref !== "string" || !root.ref.startsWith("refs/heads/")) fail();
    const before = revision(root.before);
    const head = revision(root.after);
    if (head !== revisionId) stale();
    const newBranch = zero.test(before);
    return Object.freeze({
      event, activity: "", scope: "", fullCheck: newBranch,
      ...(newBranch ? { notCollectedReason: "new-branch" as const } : {}),
      prNumber: null, changedFiles: null,
      source: Object.freeze({ repository, adapter: newBranch ? "full" as const : "git" as const, before: newBranch ? null : before, base: newBranch ? null : before, head, mergeBase: null, policyRevision: revisionId, actionRevision: revisionId, testRevision: revisionId })
    });
  }
  if (event !== "pull_request") fail();
  const pr = record(root.pull_request);
  const base = revision(record(pr.base).sha);
  const head = revision(record(pr.head).sha);
  const prNumber = typeof root.number === "number" ? root.number : pr.number;
  if (typeof prNumber !== "number" || !Number.isInteger(prNumber) || prNumber <= 0) return fail();
  if (typeof pr.changed_files !== "number" || !Number.isInteger(pr.changed_files) || pr.changed_files < 0) return fail();
  if (typeof root.action !== "string") return fail();
  return Object.freeze({
    event, activity: root.action, scope: "", fullCheck: false, prNumber, changedFiles: pr.changed_files,
    source: Object.freeze({ repository, adapter: "pr-api", before: null, base, head, mergeBase: base, policyRevision: revisionId, actionRevision: revisionId, testRevision: revisionId })
  });
}
