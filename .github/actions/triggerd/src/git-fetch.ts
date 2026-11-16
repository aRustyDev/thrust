import { existsSync, mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { TriggerdError } from "./errors.js";
import { runGit, type ProcessDependencies, type ProcessOptions, type ProcessResult } from "./process.js";

type Request = Readonly<{ owner: string; repository: string; base: string; head: string; token: string; timeoutMs: number; signal?: AbortSignal }>;
type Runner = (argv: readonly string[], options: ProcessOptions, dependencies: ProcessDependencies) => Promise<ProcessResult>;
export type FetchDependencies = Readonly<{
  runGit?: Runner;
  makeTemporary?: () => string;
  remove?: (path: string) => void;
  environment?: NodeJS.ProcessEnv;
  now?: () => number;
}>;

const segment = /^[A-Za-z0-9_.-]{1,100}$/;
const sha = /^[0-9a-f]{40}$/;
const depths = [64, 256, 1024] as const;
const invalid = (): never => { throw new TriggerdError("SOURCE_INVALID", "Git history source invalid"); };
const unavailable = (): never => { throw new TriggerdError("SOURCE_UNAVAILABLE", "Git history unavailable"); };

function origin(owner: string, repository: string): string {
  if (!segment.test(owner) || !segment.test(repository) || owner === "." || owner === ".." || repository === "." || repository === "..") invalid();
  return `https://github.com/${owner}/${repository}.git`;
}

function environment(token: string, inherited: NodeJS.ProcessEnv | undefined): NodeJS.ProcessEnv {
  if (!token || token.includes("\0") || Buffer.byteLength(token, "utf8") > 4096) invalid();
  const output: NodeJS.ProcessEnv = {
    GIT_CONFIG_NOSYSTEM: "1",
    GIT_CONFIG_GLOBAL: "/dev/null",
    GIT_CONFIG_COUNT: "1",
    GIT_CONFIG_KEY_0: "http.https://github.com/.extraheader",
    GIT_CONFIG_VALUE_0: `AUTHORIZATION: basic ${Buffer.from(`x-access-token:${token}`, "utf8").toString("base64")}`,
    GIT_TERMINAL_PROMPT: "0",
    GIT_ALLOW_PROTOCOL: "https"
  };
  for (const name of ["PATH", "SSL_CERT_FILE", "SSL_CERT_DIR", "GIT_SSL_CAINFO"] as const) {
    const value = inherited?.[name];
    if (typeof value === "string" && value.length > 0 && !value.includes("\0")) output[name] = value;
  }
  return output;
}

function isUnavailable(error: unknown): boolean {
  return typeof error === "object" && error !== null && "code" in error && (error as { readonly code?: unknown }).code === "SOURCE_UNAVAILABLE";
}

function isDeadline(error: unknown): boolean {
  return typeof error === "object" && error !== null && "code" in error && (error as { readonly code?: unknown }).code === "DEADLINE_EXCEEDED";
}

export async function withFetchedHistory<T>(request: Request, use: (bareRepository: string) => Promise<T>, dependencies: FetchDependencies = {}): Promise<T> {
  if (!sha.test(request.base) || !sha.test(request.head) || !Number.isSafeInteger(request.timeoutMs) || request.timeoutMs <= 0) invalid();
  if (request.signal?.aborted) throw new TriggerdError("DEADLINE_EXCEEDED", "Git history deadline exceeded");
  const remote = origin(request.owner, request.repository);
  const childEnvironment = environment(request.token, dependencies.environment ?? process.env);
  const makeTemporary = dependencies.makeTemporary ?? (() => mkdtempSync(join(tmpdir(), "triggerd-git-fetch-")));
  const remove = dependencies.remove ?? ((path: string) => rmSync(path, { recursive: true, force: true }));
  const bare = makeTemporary();
  if (!bare || !existsSync(bare)) invalid();
  const execute = dependencies.runGit ?? runGit;
  const now = dependencies.now ?? Date.now;
  const deadlineAt = now() + request.timeoutMs;
  const options = (): ProcessOptions => {
    const remaining = deadlineAt - now();
    if (!Number.isSafeInteger(remaining) || remaining <= 0 || request.signal?.aborted) throw new TriggerdError("DEADLINE_EXCEEDED", "Git history deadline exceeded");
    return { timeoutMs: remaining, signal: request.signal };
  };
  const processDependencies: ProcessDependencies = { environment: childEnvironment };
  try {
    await execute(["init", "--bare", bare], options(), processDependencies);
    for (const depth of depths) {
      try {
        await execute(["-C", bare, "fetch", "--no-tags", `--depth=${depth}`, remote, `+${request.base}:refs/triggerd/base`, `+${request.head}:refs/triggerd/head`], options(), processDependencies);
        await execute(["-C", bare, "cat-file", "-e", `${request.base}^{commit}`], options(), processDependencies);
        await execute(["-C", bare, "cat-file", "-e", `${request.head}^{commit}`], options(), processDependencies);
      } catch (error) { if (isDeadline(error)) throw error; unavailable(); }
      try { return await use(bare); }
      catch (error) { if (!isUnavailable(error) || depth === depths[depths.length - 1]) unavailable(); }
    }
    return unavailable();
  } finally {
    try { remove(bare); } catch { /* Cleanup cannot replace a completed fetch result. */ }
  }
}
