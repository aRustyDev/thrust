import { TriggerdError } from "./errors.js";
import { normalizeChanges, type ChangeRecord } from "./changes.js";

type Metadata = { readonly base: string; readonly head: string; readonly changedFiles: number };
type ApiRow = { readonly status: string; readonly filename: string; readonly previous_filename?: string };
type Client = {
  readonly metadata: (signal?: AbortSignal) => Promise<Metadata>;
  readonly files: (page: number, signal?: AbortSignal) => Promise<readonly ApiRow[]>;
};
type Octokit = {
  readonly rest: {
    readonly pulls: {
      readonly get: (request: { readonly owner: string; readonly repo: string; readonly pull_number: number; readonly request: { readonly signal?: AbortSignal } }) => Promise<{ readonly data: unknown }>;
      readonly listFiles: (request: { readonly owner: string; readonly repo: string; readonly pull_number: number; readonly page: number; readonly per_page: number; readonly request: { readonly signal?: AbortSignal } }) => Promise<{ readonly data: unknown }>;
    };
  };
};
type RetryOptions = {
  readonly now?: () => number;
  readonly wait?: (milliseconds: number) => Promise<void>;
  readonly retryAfter?: (error: unknown) => number | undefined;
};

const incomplete = (): never => { throw new TriggerdError("SOURCE_INCOMPLETE", "PR source incomplete"); };
const stale = (): never => { throw new TriggerdError("SOURCE_STALE", "PR source stale"); };
const invalid = (): never => { throw new TriggerdError("SOURCE_INVALID", "PR source invalid"); };
const same = (left: Metadata, right: Metadata): boolean => left.base === right.base && left.head === right.head && left.changedFiles === right.changedFiles;
const exhausted = (): never => { throw new TriggerdError("DEADLINE_EXCEEDED", "PR API deadline exceeded"); };
const sleep = (milliseconds: number): Promise<void> => new Promise((resolve) => setTimeout(resolve, milliseconds));
const object = (value: unknown): Record<string, unknown> => typeof value === "object" && value !== null && !Array.isArray(value) ? value as Record<string, unknown> : invalid();
const text = (value: unknown): string => typeof value === "string" ? value : invalid();
const sha = (value: unknown): string => /^[0-9a-f]{40}$/.test(text(value)) ? text(value) : invalid();

export function createGithubPrClient(octokit: Octokit, repository: string, prNumber: number): Client {
  const match = /^([A-Za-z0-9_.-]+)\/([A-Za-z0-9_.-]+)$/.exec(repository);
  if (match === null || match[1] === undefined || match[2] === undefined || !Number.isInteger(prNumber) || prNumber <= 0) return invalid();
  const owner: string = match[1];
  const repo: string = match[2];
  const request = (signal: AbortSignal | undefined) => ({ owner, repo, pull_number: prNumber, request: { signal } });
  return Object.freeze({
    metadata: async (signal) => {
      const data = object((await octokit.rest.pulls.get(request(signal))).data);
      const base = sha(object(data.base).sha);
      const head = sha(object(data.head).sha);
      const rawChangedFiles = data.changed_files;
      const changedFiles: number = typeof rawChangedFiles === "number" && Number.isInteger(rawChangedFiles) && rawChangedFiles >= 0 ? rawChangedFiles : invalid();
      return Object.freeze({ base, head, changedFiles });
    },
    files: async (page, signal) => {
      if (!Number.isInteger(page) || page <= 0) invalid();
      const data = (await octokit.rest.pulls.listFiles({ ...request(signal), page, per_page: 100 })).data;
      const rows: unknown[] = Array.isArray(data) ? data : invalid();
      return Object.freeze(rows.map((item: unknown) => {
        const row = object(item);
        const status = text(row.status);
        const filename = text(row.filename);
        const previousFilename: string | undefined = row.previous_filename === undefined ? undefined : typeof row.previous_filename === "string" ? row.previous_filename : invalid();
        return Object.freeze({ status, filename, ...(previousFilename === undefined ? {} : { previous_filename: previousFilename }) });
      }));
    }
  });
}

function statusOf(error: unknown): number | undefined {
  if (typeof error !== "object" || error === null || !("status" in error)) return undefined;
  const { status } = error as { readonly status?: unknown };
  return typeof status === "number" ? status : undefined;
}

function retryDelay(error: unknown, attempt: number, options: RetryOptions): number {
  const baseDelay = attempt === 0 ? 250 : 1000;
  const response = typeof error === "object" && error !== null && "response" in error ? object((error as { readonly response: unknown }).response) : undefined;
  const headers = response === undefined || !("headers" in response) ? undefined : object(response.headers);
  const header = headers?.["retry-after"];
  const retryAfter = typeof header === "string" && /^\d+(?:\.\d+)?$/.test(header) ? Number(header) * 1000 : undefined;
  const requested = options.retryAfter?.(error) ?? retryAfter;
  return typeof requested === "number" && Number.isFinite(requested) && requested > baseDelay ? requested : baseDelay;
}

function isTransportTimeout(error: unknown): boolean {
  if (typeof error !== "object" || error === null) return false;
  const value = error as { readonly name?: unknown; readonly code?: unknown };
  return value.name === "AbortError" || value.code === "ETIMEDOUT" || value.code === "UND_ERR_CONNECT_TIMEOUT";
}

async function request<T>(operation: (signal: AbortSignal) => Promise<T>, options: RetryOptions, deadlineAt: number): Promise<T> {
  const now = options.now ?? Date.now;
  const wait = options.wait ?? sleep;
  for (let attempt = 0; attempt < 3; attempt += 1) {
    if (now() >= deadlineAt) exhausted();
    const controller = new AbortController();
    const timeout = setTimeout(() => controller.abort(), deadlineAt - now());
    try {
      const result = await operation(controller.signal);
      if (now() >= deadlineAt) exhausted();
      return result;
    }
    catch (error) {
      if (error instanceof TriggerdError) throw error;
      if (controller.signal.aborted) exhausted();
      const status = statusOf(error);
      if (status === 401 || status === 403) throw new TriggerdError("AUTH_DENIED", "PR API authorization denied");
      const retryable = status === 429 || (typeof status === "number" && status >= 500 && status <= 599) || isTransportTimeout(error);
      if (!retryable || attempt === 2) throw new TriggerdError("SOURCE_UNAVAILABLE", "PR API unavailable");
      const delay = retryDelay(error, attempt, options);
      if (now() + delay >= deadlineAt) exhausted();
      await wait(delay);
    } finally { clearTimeout(timeout); }
  }
  throw new TriggerdError("SOURCE_UNAVAILABLE", "PR API unavailable");
}

export async function collectPrChanges(client: Client, expected: Metadata, deadlineMs = 120000, startedAt = Date.now(), options: RetryOptions = {}): Promise<readonly ChangeRecord[]> {
  const now = options.now ?? Date.now;
  const deadlineAt = startedAt + deadlineMs;
  if (!Number.isFinite(deadlineMs) || deadlineMs <= 0 || !Number.isFinite(startedAt) || !Number.isFinite(deadlineAt) || now() >= deadlineAt) exhausted();
  const before = await request((signal) => client.metadata(signal), options, deadlineAt);
  if (!same(before, expected)) stale();
  if (!Number.isInteger(before.changedFiles) || before.changedFiles < 0) invalid();
  if (before.changedFiles > 3000) incomplete();
  const rows: ApiRow[] = [];
  for (let page = 1; rows.length < before.changedFiles; page += 1) {
    const current = await request((signal) => client.files(page, signal), options, deadlineAt);
    if (!Array.isArray(current) || !current.length || current.length > 100) incomplete();
    rows.push(...current);
    if (current.length < 100) break;
  }
  const after = await request((signal) => client.metadata(signal), options, deadlineAt);
  if (!same(after, before)) stale();
  if (rows.length !== before.changedFiles || rows.length > 3000) incomplete();
  try {
    return normalizeChanges(rows.map((row) => ({ status: row.status === "renamed" ? "renamed" : row.status === "copied" ? "copied" : row.status === "changed" ? "type-changed" : row.status, path: row.filename, ...(row.previous_filename === undefined ? {} : { previousPath: row.previous_filename }) })) as ChangeRecord[]);
  } catch { return invalid(); }
}

export async function verifyPrMetadata(client: Client, expected: Metadata, deadlineMs: number, startedAt: number, options: RetryOptions = {}): Promise<void> {
  const now = options.now ?? Date.now;
  const deadlineAt = startedAt + deadlineMs;
  if (!Number.isFinite(deadlineMs) || deadlineMs <= 0 || !Number.isFinite(startedAt) || !Number.isFinite(deadlineAt) || now() >= deadlineAt) exhausted();
  const current = await request((signal) => client.metadata(signal), options, deadlineAt);
  if (!same(current, expected)) stale();
}
