import { lstatSync, readFileSync, realpathSync } from "node:fs";
import { isAbsolute, relative, resolve, sep } from "node:path";
import { parsePolicy } from "./config.js";
import { parseRuntimeContext } from "./context.js";
import { createGithubPrClient, collectPrChanges, verifyPrMetadata } from "./github.js";
import { withFetchedHistory } from "./git-fetch.js";
import { collectLocalDiff, collectPushDiff } from "./git-local.js";
import { LIMITS } from "./limits.js";
import { serializeEvaluation } from "./report.js";
import { runEvaluationWorker } from "./worker.js";
import { TriggerdError } from "./errors.js";
import type { ChangeRecord } from "./changes.js";

type Core = {
  readonly getInput: (name: string, options?: { readonly required?: boolean; readonly trimWhitespace?: boolean }) => string;
  readonly setSecret: (value: string) => void;
  readonly setOutput: (name: string, value: string) => void;
  readonly warning: (message: string) => void;
  readonly summary: { readonly addRaw: (text: string, addEol?: boolean) => { readonly write: () => Promise<unknown> } };
};
type GitHubContext = { readonly eventName: string; readonly payload: unknown; readonly sha: string };
type Octokit = Parameters<typeof createGithubPrClient>[0];
export type ActionDependencies = {
  readonly core: Core;
  readonly context: GitHubContext;
  readonly cwd: string;
  readonly getOctokit: (token: string) => Octokit;
  readonly now?: () => number;
  readonly fetchHistory?: typeof withFetchedHistory;
  readonly collectLocalDiff?: typeof collectLocalDiff;
  readonly collectPushDiff?: typeof collectPushDiff;
};

const input = (): never => { throw new TriggerdError("INPUT_INVALID", "Action input invalid"); };
const source = (): never => { throw new TriggerdError("SOURCE_INVALID", "Rules source invalid"); };
const deadline = (): never => { throw new TriggerdError("DEADLINE_EXCEEDED", "Action deadline exceeded"); };
const incomplete = (error: unknown): boolean => typeof error === "object" && error !== null && "code" in error && (error as { readonly code?: unknown }).code === "SOURCE_INCOMPLETE";

function inside(root: string, candidate: string): boolean {
  const path = relative(root, candidate);
  return path === "" || (!path.startsWith(`..${sep}`) && path !== "..");
}

function readRules(cwd: string, logicalPath: string): Uint8Array {
  if (!logicalPath || isAbsolute(logicalPath) || logicalPath.includes("\0")) input();
  const root = realpathSync(cwd);
  const candidate = resolve(root, logicalPath);
  if (!inside(root, candidate)) source();
  try {
    const status = lstatSync(candidate);
    if (!status.isFile() || status.isSymbolicLink() || status.size > LIMITS.policyBytes) source();
    const resolved = realpathSync(candidate);
    if (!inside(root, resolved)) source();
    const bytes = readFileSync(resolved);
    if (bytes.byteLength > LIMITS.policyBytes) source();
    return bytes;
  } catch (error) {
    if (error instanceof TriggerdError) throw error;
    return source();
  }
}

function fullCheck(value: string): boolean {
  if (value === "" || value === "false") return false;
  if (value === "true") return true;
  return input();
}

export async function executeAction(deps: ActionDependencies): Promise<void> {
  const now = deps.now ?? Date.now;
  const startedAt = now();
  const token = deps.core.getInput("github-token", { required: true });
  if (!token) input();
  deps.core.setSecret(token);
  const rulesFile = deps.core.getInput("rules-file") || ".github/triggerd.yml";
  const scope = deps.core.getInput("scope");
  const forcedFullCheck = fullCheck(deps.core.getInput("full-check", { trimWhitespace: false }));
  const policyBytes = readRules(deps.cwd, rulesFile);
  const policy = parsePolicy(policyBytes, rulesFile);
  const runtime = parseRuntimeContext(deps.context.eventName, deps.context.payload, deps.context.sha);
  const isFullCheck = forcedFullCheck || runtime.fullCheck;
  const context = Object.freeze({
    event: runtime.event,
    activity: runtime.activity,
    scope,
    fullCheck: isFullCheck,
    ...(isFullCheck ? { notCollectedReason: forcedFullCheck ? "explicit-full-check" as const : runtime.notCollectedReason } : {})
  });
  const remaining = (): number => {
    const value = LIMITS.totalMs - (now() - startedAt);
    if (!Number.isFinite(value) || value <= 0) deadline();
    return value;
  };
  const fetchHistory = (deps.fetchHistory ?? withFetchedHistory) as typeof withFetchedHistory;
  const localDiff = deps.collectLocalDiff ?? collectLocalDiff;
  const pushDiff = deps.collectPushDiff ?? collectPushDiff;
  let sourceDetails = runtime.source;
  let changes: readonly import("./changes.js").ChangeRecord[] | null;
  if (isFullCheck) changes = null;
  else if (runtime.event === "push") {
    const [owner, repository] = runtime.source.repository.split("/");
    const before = runtime.source.before;
    if (!owner || !repository || typeof before !== "string" || !runtime.source.base) return source();
    const result = await fetchHistory<{ readonly changes: readonly ChangeRecord[] }>(
      { owner, repository, base: before, head: runtime.source.head, token, timeoutMs: remaining() },
      async (bare) => pushDiff(bare, before, runtime.source.head, remaining())
    );
    changes = result.changes;
  } else {
    const expected = { base: runtime.source.base ?? input(), head: runtime.source.head, changedFiles: runtime.changedFiles ?? input() };
    const client = createGithubPrClient(deps.getOctokit(token), runtime.source.repository, runtime.prNumber ?? input());
    try { changes = await collectPrChanges(client, expected, LIMITS.totalMs, startedAt, { now }); }
    catch (error) {
      if (!incomplete(error)) throw error;
      await verifyPrMetadata(client, expected, LIMITS.totalMs, startedAt, { now });
      const [owner, repository] = runtime.source.repository.split("/");
      if (!owner || !repository) source();
      const result = await fetchHistory<Readonly<{ mergeBase: string; changes: readonly ChangeRecord[] }>>(
        { owner, repository, base: expected.base, head: expected.head, token, timeoutMs: remaining() },
        async (bare) => localDiff(bare, expected.base, expected.head, remaining())
      );
      await verifyPrMetadata(client, expected, LIMITS.totalMs, startedAt, { now });
      changes = result.changes;
      sourceDetails = Object.freeze({ ...runtime.source, adapter: "pr-git" as const, mergeBase: result.mergeBase });
    }
  }
  const remainingMs = remaining();
  const evaluation = await runEvaluationWorker({ policy, context, changes }, Math.min(LIMITS.workerMs, remainingMs), { remainingMs });
  const serialized = serializeEvaluation(evaluation, policyBytes, { scope, event: runtime.event, activity: runtime.activity, source: sourceDetails });
  deps.core.setOutput("rules", serialized.rules);
  deps.core.setOutput("details", serialized.details);
  try { await deps.core.summary.addRaw(serialized.summary, true).write(); }
  catch { deps.core.warning("triggerd: summary unavailable"); }
}
