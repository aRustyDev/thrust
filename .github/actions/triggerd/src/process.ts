import { spawn } from "node:child_process";
import { TriggerdError } from "./errors.js";

export type ProcessResult = Readonly<{ stdout: Uint8Array }>;
export type ProcessOptions = Readonly<{ timeoutMs: number; signal?: AbortSignal }>;
export type ProcessDependencies = Readonly<{ executable?: string; environment?: NodeJS.ProcessEnv }>;
const stdoutLimit = 64 * 1024 * 1024;
const fail = (code: "DEADLINE_EXCEEDED" | "SOURCE_INVALID" | "SOURCE_UNAVAILABLE"): never => {
  throw new TriggerdError(code, code === "DEADLINE_EXCEEDED" ? "Git process deadline exceeded" : "Git process failed");
};

export async function runGit(argv: readonly string[], options: ProcessOptions, dependencies: ProcessDependencies = {}): Promise<ProcessResult> {
  if (!Array.isArray(argv) || !argv.length || argv.some((value) => typeof value !== "string" || value.includes("\0") || value === "-c" || value.startsWith("--config"))) fail("SOURCE_INVALID");
  if (!Number.isSafeInteger(options.timeoutMs) || options.timeoutMs <= 0) fail("DEADLINE_EXCEEDED");
  return new Promise((resolve, reject) => {
    const child = spawn(dependencies.executable ?? "git", [...argv], { shell: false, stdio: ["ignore", "pipe", "pipe"], env: dependencies.environment });
    const chunks: Buffer[] = []; let size = 0; let settled = false;
    const finish = (error?: unknown): void => { if (settled) return; settled = true; clearTimeout(timer); options.signal?.removeEventListener("abort", abort); error ? reject(error) : resolve(Object.freeze({ stdout: new Uint8Array(Buffer.concat(chunks)) })); };
    let aborted: TriggerdError | undefined;
    const abort = (): void => { aborted ??= new TriggerdError("DEADLINE_EXCEEDED", "Git process deadline exceeded"); child.kill("SIGKILL"); };
    const timer = setTimeout(abort, options.timeoutMs);
    options.signal?.addEventListener("abort", abort, { once: true });
    if (options.signal?.aborted) abort();
    child.stdout.on("data", (chunk: Buffer) => { size += chunk.length; if (size > stdoutLimit) { child.kill("SIGKILL"); finish(new TriggerdError("LIMIT_EXCEEDED", "Git output limit exceeded")); } else chunks.push(chunk); });
    child.on("error", () => finish(new TriggerdError("SOURCE_UNAVAILABLE", "Git process failed")));
    child.on("close", (code) => aborted ? finish(aborted) : code === 0 ? finish() : finish(new TriggerdError("SOURCE_UNAVAILABLE", "Git process failed")));
  });
}
