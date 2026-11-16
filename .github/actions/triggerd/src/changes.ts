import { TriggerdError } from "./errors.js";
import { LIMITS } from "./limits.js";
import { compilePattern } from "./patterns.js";

export type ChangeStatus = "added" | "modified" | "removed" | "renamed" | "copied" | "type-changed";
export type ChangeRecord = { readonly status: ChangeStatus; readonly path: string; readonly previousPath?: string };
export type Classification = {
  readonly groups: Readonly<Record<string, boolean>>;
  readonly unmatched: readonly string[];
  readonly invalidated: boolean;
  readonly coverageWarnings: readonly string[];
};

type Policy = Record<string, any>;
type UnknownRecord = Record<string, unknown>;

const statuses = new Set<ChangeStatus>(["added", "modified", "removed", "renamed", "copied", "type-changed"]);
const compare = (left: string, right: string): number => left < right ? -1 : left > right ? 1 : 0;
const inputFail = (): never => { throw new TriggerdError("INPUT_INVALID", "Change validation failed"); };
const limitFail = (): never => { throw new TriggerdError("LIMIT_EXCEEDED", "Change limit exceeded"); };
const isRecord = (value: unknown): value is UnknownRecord => Boolean(value) && !Array.isArray(value) && typeof value === "object";

function validPath(value: unknown): value is string {
  if (typeof value !== "string" || !value || Buffer.byteLength(value, "utf8") > LIMITS.pathBytes) return false;
  if (Buffer.from(value, "utf8").toString("utf8") !== value || value.includes("\0") || value.includes("\\") || value.startsWith("/")) return false;
  const segments = value.split("/");
  return segments.length <= LIMITS.pathSegments && !segments.some((part) => !part || part === "." || part === "..");
}

function normalizeRecord(value: unknown): ChangeRecord {
  if (!isRecord(value)) return inputFail();
  const keys = Object.keys(value);
  if (keys.some((key) => key !== "status" && key !== "path" && key !== "previousPath")) inputFail();
  const status = value.status;
  const path = value.path;
  if (typeof status !== "string" || !statuses.has(status as ChangeStatus)) return inputFail();
  if (!validPath(path)) return inputFail();
  const hasPrevious = Object.hasOwn(value, "previousPath") && value.previousPath !== undefined;
  const previousPath = value.previousPath;
  if (status === "renamed") {
    if (!hasPrevious || !validPath(previousPath) || previousPath === path) inputFail();
  } else if (status === "copied") {
    if (hasPrevious && (!validPath(previousPath) || previousPath === path)) inputFail();
  } else if (hasPrevious) inputFail();
  return hasPrevious
    ? Object.freeze({ status: status as ChangeStatus, path, previousPath: previousPath as string })
    : Object.freeze({ status: status as ChangeStatus, path });
}

export function normalizeChanges(records: readonly ChangeRecord[]): readonly ChangeRecord[] {
  if (!Array.isArray(records)) inputFail();
  if (records.length > LIMITS.rawChanges) limitFail();
  const seen = new Set<string>();
  const renameSources = new Set<string>();
  const renameDestinations = new Set<string>();
  const normalized = records.map((record) => {
    const change = normalizeRecord(record);
    const key = `${change.status}\0${change.previousPath ?? ""}\0${change.path}`;
    if (seen.has(key)) inputFail();
    seen.add(key);
    if (change.status === "renamed") {
      if (renameSources.has(change.previousPath!) || renameDestinations.has(change.path)) inputFail();
      renameSources.add(change.previousPath!);
      renameDestinations.add(change.path);
    }
    return change;
  });
  return Object.freeze(normalized);
}

function referencedGroups(policy: Policy): Set<string> {
  const referenced = new Set<string>();
  const walk = (expression: unknown): void => {
    if (!isRecord(expression)) return;
    if (typeof expression.changed === "string") referenced.add(expression.changed);
    for (const child of [expression.all, expression.any]) if (Array.isArray(child)) child.forEach(walk);
    walk(expression.not);
  };
  for (const rule of Object.values(policy.rules) as any[]) walk(rule.when);
  return referenced;
}

function countPatterns(policy: Policy): number {
  return policy.invalidate.length + Object.values(policy.changes).reduce(
    (count: number, group: any) => count + group.paths.length + (group.exclude?.length ?? 0),
    0
  );
}

export function classifyChanges(policy: Policy, records: readonly ChangeRecord[]): Classification {
  const paths = [...new Set(records.flatMap((record) => record.previousPath ? [record.path, record.previousPath] : [record.path]))].sort(compare);
  const totalPatterns = countPatterns(policy);
  if (!Number.isSafeInteger(totalPatterns) || totalPatterns < 1 || paths.length > Math.floor(LIMITS.comparisons / totalPatterns)) limitFail();

  const referenced = referencedGroups(policy);
  const groups: Record<string, boolean> = Object.create(null);
  const consumed = new Set<string>();
  const coverageWarnings: string[] = [];
  for (const id of Object.keys(policy.changes).sort(compare)) {
    const group = policy.changes[id];
    const include = group.paths.map(compilePattern);
    const exclude = (group.exclude ?? []).map(compilePattern);
    const matched = paths.filter((path) => include.some((match: (path: string) => boolean) => match(path)) && !exclude.some((match: (path: string) => boolean) => match(path)));
    groups[id] = matched.length > 0;
    if (referenced.has(id) || Object.hasOwn(policy.coverage ?? Object.create(null), id)) matched.forEach((path) => consumed.add(path));
    else coverageWarnings.push(id);
  }
  const invalidation = policy.invalidate.map(compilePattern);
  const invalidatedPaths = new Set(paths.filter((path) => invalidation.some((match: (path: string) => boolean) => match(path))));
  return Object.freeze({
    groups: Object.freeze(groups),
    unmatched: Object.freeze(paths.filter((path) => !consumed.has(path) && !invalidatedPaths.has(path))),
    invalidated: invalidatedPaths.size > 0,
    coverageWarnings: Object.freeze(coverageWarnings)
  });
}
