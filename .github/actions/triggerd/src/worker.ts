import { Worker } from "node:worker_threads";
import { join } from "node:path";
import { TriggerdError } from "./errors.js";
import { LIMITS } from "./limits.js";
import type { ChangeRecord } from "./changes.js";
import type { Context, Evaluation } from "./evaluate.js";

type WorkerInput = { readonly policy: Record<string, any>; readonly context: Context; readonly changes: readonly ChangeRecord[] | null };
type WorkerMessage = { readonly ok: boolean; readonly evaluation?: Evaluation; readonly rss?: number };
type WorkerOptions = { readonly remainingMs?: number; readonly workerPath?: string };
export type WorkerEvaluation = { readonly evaluation: Evaluation; readonly workerRss: number };

const deadlineError = (): TriggerdError => new TriggerdError("DEADLINE_EXCEEDED", "Evaluation deadline exceeded");
const inputError = (): TriggerdError => new TriggerdError("INPUT_INVALID", "Evaluation failed");

function runWorker(input: WorkerInput, workerBudgetMs: number, options: WorkerOptions = {}): Promise<WorkerEvaluation> {
  const remainingMs = options.remainingMs ?? workerBudgetMs;
  const deadlineMs = Math.min(LIMITS.workerMs, workerBudgetMs, remainingMs);
  if (!Number.isFinite(deadlineMs) || deadlineMs <= 0) return Promise.reject(deadlineError());

  return new Promise((resolve, reject) => {
    const worker = new Worker(options.workerPath ?? join(__dirname, "worker-main.js"));
    let settled = false;
    const settle = (callback: () => void, terminate: boolean): void => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      if (terminate) void worker.terminate().catch(() => undefined).finally(callback);
      else callback();
    };
    const timer = setTimeout(() => settle(() => reject(deadlineError()), true), deadlineMs);
    worker.once("message", (message: WorkerMessage) => settle(
      () => message.ok && message.evaluation && typeof message.rss === "number" ? resolve(Object.freeze({ evaluation: message.evaluation, workerRss: message.rss })) : reject(inputError()),
      true
    ));
    worker.once("error", () => settle(() => reject(inputError()), true));
    worker.once("exit", () => settle(() => reject(inputError()), false));
    try { worker.postMessage(input); }
    catch { settle(() => reject(inputError()), true); }
  });
}

export function runEvaluationWorker(input: WorkerInput, workerBudgetMs: number, options: WorkerOptions = {}): Promise<Evaluation> {
  return runWorker(input, workerBudgetMs, options).then((result) => result.evaluation);
}

export function runEvaluationWorkerMeasured(input: WorkerInput, workerBudgetMs: number, options: WorkerOptions = {}): Promise<WorkerEvaluation> {
  return runWorker(input, workerBudgetMs, options);
}
