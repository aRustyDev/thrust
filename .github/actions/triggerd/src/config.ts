import { parseAllDocuments } from "yaml";
import { TriggerdError } from "./errors.js";
import { LIMITS } from "./limits.js";
import { compilePattern } from "./patterns.js";

type Policy = Record<string, unknown>;
const RESERVED = new Set(["__proto__", "constructor", "prototype"]);
const GROUP = /^[a-z][a-z0-9-]*$/;
const RULE = /^[a-z][a-z0-9-]*\.[a-z][a-z0-9-]*$/;
const EVENTS = new Set(["pull_request", "push", "workflow_dispatch", "schedule", "merge_group"]);
const fail = (): never => { throw new TriggerdError("CONFIG_INVALID", "Policy validation failed"); };
const isRecord = (value: unknown): value is Policy => Boolean(value) && !Array.isArray(value) && typeof value === "object";
const record = (value: unknown): Policy => isRecord(value) ? value : fail();
const array = (value: unknown): unknown[] => Array.isArray(value) ? value : fail();
const nonempty = (value: unknown): string => typeof value === "string" && value ? value : fail();
const allowed = (value: Policy, names: string[]): void => { for (const key of Object.keys(value)) if (RESERVED.has(key) || !names.includes(key)) fail(); };

function toSafeValue(value: unknown): unknown {
  if (Array.isArray(value)) return value.map(toSafeValue);
  if (!isRecord(value)) return value;
  const safe: Policy = Object.create(null) as Policy;
  for (const [key, child] of Object.entries(value)) {
    if (RESERVED.has(key)) fail();
    safe[key] = toSafeValue(child);
  }
  return safe;
}

function checkExpression(value: unknown, groups: Set<string>, depth = 1, count = { value: 0 }): void {
  if (depth > LIMITS.expressionDepth || ++count.value > LIMITS.expressionNodes) fail();
  if (typeof value === "boolean") return;
  const expression = record(value); const names = Object.keys(expression); if (names.length !== 1) fail();
  const name = names[0];
  if (name === "changed") { if (!groups.has(nonempty(expression.changed))) fail(); return; }
  if (name === "activity") { if (!new Set(["edited"]).has(nonempty(expression.activity))) fail(); return; }
  if (name === "not") return checkExpression(expression.not, groups, depth + 1, count);
  if (name !== "all" && name !== "any" || !Array.isArray(expression[name]) || expression[name].length === 0) fail();
  for (const child of expression[name] as unknown[]) checkExpression(child, groups, depth + 1, count);
}

export function parsePolicy(bytes: Uint8Array, logicalRulesPath: string): Policy {
  if (bytes.byteLength > LIMITS.policyBytes || !logicalRulesPath) fail();
  let text: string; try { text = new TextDecoder("utf-8", { fatal: true }).decode(bytes); } catch { return fail(); }
  if (/(^|\s)(?:<<:|[&*][A-Za-z_]|!)/.test(text)) fail();
  const documents = parseAllDocuments(text, { uniqueKeys: true, merge: false, schema: "core" });
  if (documents.length !== 1 || documents[0].errors.length || documents[0].warnings.length) fail();
  const policy = record(toSafeValue(documents[0].toJS({ mapAsMap: false })));
  allowed(policy, ["version", "invalidate", "changes", "rules", "coverage"]);
  if (policy.version !== 1) fail();
  const invalidate = array(policy.invalidate);
  if (!invalidate.includes(logicalRulesPath)) fail();
  if (!invalidate.includes(".github/actions/triggerd/**")) fail();
  const changes = record(policy.changes); const groups = new Set(Object.keys(changes));
  if (!groups.size || groups.size > LIMITS.groups) fail();
  let patterns = invalidate.length;
  for (const invalidation of invalidate) compilePattern(nonempty(invalidation));
  for (const [id, item] of Object.entries(changes)) { if (!GROUP.test(id)) fail(); const group = record(item); allowed(group, ["paths", "exclude"]); const paths = array(group.paths); if (!paths.length) fail(); for (const path of paths) compilePattern(nonempty(path)); const exclude = group.exclude === undefined ? undefined : array(group.exclude); if (exclude !== undefined) { if (!exclude.length) fail(); for (const path of exclude) compilePattern(nonempty(path)); patterns += exclude.length; } patterns += paths.length; }
  if (patterns > LIMITS.patterns) fail();
  const rules = record(policy.rules); if (!Object.keys(rules).length || Object.keys(rules).length > LIMITS.rules) fail();
  const consumed = new Set<string>();
  const collect = (value: unknown): void => { if (!value || typeof value !== "object") return; const expression = value as Policy; if (typeof expression.changed === "string") consumed.add(expression.changed); for (const child of [expression.all, expression.any]) if (Array.isArray(child)) child.forEach(collect); collect(expression.not); };
  for (const [id, item] of Object.entries(rules)) { if (!RULE.test(id)) fail(); const rule = record(item); allowed(rule, ["description", "events", "when"]); nonempty(rule.description); if (!Array.isArray(rule.events) || !rule.events.length || new Set(rule.events).size !== rule.events.length || !rule.events.every((event) => typeof event === "string" && EVENTS.has(event))) fail(); checkExpression(rule.when, groups); collect(rule.when); }
  if (policy.coverage !== undefined) for (const [id, item] of Object.entries(record(policy.coverage))) {
    if (!groups.has(id) || consumed.has(id)) fail();
    const disposition = record(item);
    if (disposition.kind === "external") {
      allowed(disposition, ["kind", "owner", "reason", "workflow", "job"]);
      nonempty(disposition.owner); nonempty(disposition.reason);
      if (typeof disposition.workflow !== "string" || !/^\.github\/workflows\/[^/]+\.ya?ml$/.test(disposition.workflow)) fail();
      nonempty(disposition.job);
    } else if (disposition.kind === "exempt") {
      allowed(disposition, ["kind", "owner", "reason"]);
      nonempty(disposition.owner); nonempty(disposition.reason);
    } else fail();
  }
  return policy;
}
