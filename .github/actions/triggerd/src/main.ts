import { lstatSync, readFileSync } from "node:fs";
import { parsePolicy } from "./config.js";
import { normalizeChanges, type ChangeRecord } from "./changes.js";
import { runEvaluationWorker } from "./worker.js";
import { serializeEvaluation } from "./report.js";
import { LIMITS } from "./limits.js";
import { TriggerdError } from "./errors.js";
import { executeAction } from "./action.js";

type Flags = Record<string, string>;
const syntax = (): never => { throw new Error("CLI_SYNTAX"); };
const source = (limit: number): ((path: string) => Buffer) => (path) => {
  try {
    const status = lstatSync(path);
    if (!status.isFile() || status.isSymbolicLink() || status.size > limit) throw new Error();
    const bytes = readFileSync(path);
    if (bytes.byteLength > limit) throw new Error();
    return bytes;
  } catch { throw new TriggerdError("SOURCE_INVALID", "Input source invalid"); }
};

function parseFlags(args: readonly string[], allowed: readonly string[]): Flags {
  const flags: Flags = Object.create(null);
  for (let index = 0; index < args.length; index += 2) {
    const flag = args[index];
    const value = args[index + 1];
    if (!flag?.startsWith("--") || value === undefined || !allowed.includes(flag) || Object.hasOwn(flags, flag)) syntax();
    flags[flag] = value;
  }
  return flags;
}

function required(flags: Flags, flag: string): string { return flags[flag] ?? syntax(); }
function parseJson(bytes: Uint8Array): unknown { try { return JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(bytes)); } catch { throw new TriggerdError("INPUT_INVALID", "JSON input invalid"); } }

export async function runCli(argv: readonly string[]): Promise<string> {
  const [command, ...args] = argv;
  if (command === "validate") {
    const flags = parseFlags(args, ["--rules", "--logical-rules-path"]);
    const policyBytes = source(LIMITS.policyBytes)(required(flags, "--rules"));
    parsePolicy(policyBytes, flags["--logical-rules-path"] ?? ".github/triggerd.yml");
    return "{\"valid\":true}";
  }
  if (command === "evaluate") {
    const flags = parseFlags(args, ["--rules", "--logical-rules-path", "--context", "--changes", "--scope", "--full-check"]);
    const fullCheck = (flags["--full-check"] ?? "false") === "true" ? true : (flags["--full-check"] ?? "false") === "false" ? false : syntax();
    const policyBytes = source(LIMITS.policyBytes)(required(flags, "--rules"));
    const policy = parsePolicy(policyBytes, flags["--logical-rules-path"] ?? ".github/triggerd.yml");
    const context = parseJson(source(1024 * 1024)(required(flags, "--context")));
    if (!context || Array.isArray(context) || typeof context !== "object") throw new TriggerdError("INPUT_INVALID", "Context invalid");
    const changesRaw = parseJson(source(64 * 1024 * 1024)(required(flags, "--changes")));
    if (fullCheck ? changesRaw !== null : !Array.isArray(changesRaw)) throw new TriggerdError("INPUT_INVALID", "Changes invalid");
    const raw = context as Record<string, unknown>;
    const scope = flags["--scope"] ?? (typeof raw.scope === "string" ? raw.scope : "");
    const notCollectedReason = fullCheck ? raw.event === "workflow_dispatch" ? "manual" : raw.event === "schedule" ? "schedule" : "explicit-full-check" : undefined;
    const activity = typeof raw.activity === "string" ? raw.activity : "";
    const evaluation = await runEvaluationWorker({ policy, context: { event: raw.event as string, activity, scope, fullCheck, notCollectedReason }, changes: fullCheck ? null : normalizeChanges(changesRaw as readonly ChangeRecord[]) }, LIMITS.workerMs);
    const serialized = serializeEvaluation(evaluation, policyBytes, { scope, event: raw.event as string, activity, source: raw.source as any });
    return JSON.stringify({ rules: JSON.parse(serialized.rules), details: JSON.parse(serialized.details) });
  }
  return syntax();
}

if (process.env.GITHUB_ACTIONS === "true") {
  void (async () => {
    const core = await import("@actions/core");
    try {
      const github = await import("@actions/github");
      await executeAction({ core, context: github.context, cwd: process.cwd(), getOctokit: github.getOctokit });
    } catch { core.setFailed("triggerd: request failed"); }
  })();
} else {
  void (async () => {
    try { process.stdout.write(`${await runCli(process.argv.slice(2))}\n`); }
    catch (error) {
      const exit = error instanceof Error && error.message === "CLI_SYNTAX" ? 2 : 1;
      process.stderr.write(`triggerd: ${exit === 2 ? "invalid command" : "request failed"}\n`);
      process.exitCode = exit;
    }
  })();
}
