import { normalizeChanges, type ChangeRecord } from "./changes.js";
import { evaluatePolicy } from "./evaluate.js";
import { TriggerdError } from "./errors.js";

type Policy = Record<string, any>;
type Case = { readonly name: string; readonly changes: readonly ChangeRecord[]; readonly required: readonly string[]; readonly forbidden: readonly string[]; readonly context?: { readonly event?: string; readonly scope?: string } };
type Mutation = { readonly name: string; readonly operation: Record<string, unknown>; readonly mustFail: readonly string[] };
type Oracle = { readonly version: number; readonly defaults: { readonly event: string; readonly scope: string }; readonly cases: readonly Case[]; readonly mutations?: readonly Mutation[] };
const fail = (): never => { throw new TriggerdError("INPUT_INVALID", "Coverage oracle invalid"); };
const record = (value: unknown): Record<string, unknown> => value && typeof value === "object" && !Array.isArray(value) ? value as Record<string, unknown> : fail();
const strings = (value: unknown): readonly string[] => Array.isArray(value) && value.every((item) => typeof item === "string") ? value : fail();
const unique = (values: readonly string[]): boolean => new Set(values).size === values.length;

function parseCase(value: unknown, knownRules: ReadonlySet<string>): Case {
  const item = record(value);
  const name = typeof item.name === "string" && item.name ? item.name : fail();
  const required = strings(item.required);
  const forbidden = strings(item.forbidden);
  if (!unique(required) || !unique(forbidden) || required.some((id) => forbidden.includes(id)) || [...required, ...forbidden].some((id) => !knownRules.has(id))) fail();
  const context = item.context === undefined ? undefined : record(item.context);
  if (context && Object.keys(context).some((key) => key !== "event" && key !== "scope") || context?.event !== undefined && typeof context.event !== "string" || context?.scope !== undefined && typeof context.scope !== "string") fail();
  const changes = Array.isArray(item.changes) ? item.changes : fail();
  return Object.freeze({ name, changes: normalizeChanges(changes as readonly ChangeRecord[]), required, forbidden, ...(context ? { context } : {}) });
}

export function parseOracle(input: unknown, policy: Policy): Oracle {
  const raw = record(input);
  const knownRules = new Set(Object.keys(record(policy.rules)));
  const defaults = record(raw.defaults);
  const event: string = typeof defaults.event === "string" ? defaults.event : fail();
  const scope: string = typeof defaults.scope === "string" ? defaults.scope : fail();
  const caseInput: unknown[] = Array.isArray(raw.cases) ? raw.cases : fail();
  if (raw.version !== 1) fail();
  const cases = caseInput.map((item: unknown) => parseCase(item, knownRules));
  if (!cases.length || !unique(cases.map((item) => item.name))) fail();
  const mutations = raw.mutations === undefined ? [] : Array.isArray(raw.mutations) ? raw.mutations.map((value) => {
    const mutation = record(value); const name = typeof mutation.name === "string" ? mutation.name : fail(); const operation = record(mutation.operation); const mustFail = strings(mutation.mustFail);
    const knownFailure = (item: string): boolean => cases.some((candidate: Case) => candidate.name === item) || operation.kind === "remove-binding" && item === "static bindings completeness";
    if (!name || !unique(mustFail) || mustFail.some((item) => !knownFailure(item))) fail();
    if (operation.kind === "remove-binding" ? typeof operation.rule !== "string" : operation.kind === "remove-any-child" ? typeof operation.rule !== "string" || typeof operation.group !== "string" : operation.kind === "replace-paths" ? typeof operation.group !== "string" || !Array.isArray(operation.paths) : true) fail();
    return Object.freeze({ name, operation, mustFail });
  }) : fail();
  return Object.freeze({ version: 1, defaults: { event, scope }, cases, mutations });
}

export function verifyCoverage(policy: Policy, oracle: Oracle): { readonly overhead: Readonly<Record<string, readonly string[]>> } {
  const overhead: Record<string, readonly string[]> = Object.create(null);
  for (const item of oracle.cases) {
    const context = { event: item.context?.event ?? oracle.defaults.event, scope: item.context?.scope ?? oracle.defaults.scope, fullCheck: false };
    const decisions = evaluatePolicy(policy, context, item.changes).decisions;
    if (item.required.some((id) => decisions[id]?.selected !== true) || item.forbidden.some((id) => decisions[id]?.selected !== false)) fail();
    overhead[item.name] = Object.freeze(Object.keys(decisions).filter((id) => decisions[id].selected && !item.required.includes(id)).sort());
  }
  return Object.freeze({ overhead: Object.freeze(overhead) });
}

export function applyMutation(policy: Policy, mutation: Mutation): Policy {
  const copy = JSON.parse(JSON.stringify(policy)) as Policy;
  const operation = mutation.operation;
  if (operation.kind === "remove-any-child" && typeof operation.rule === "string" && typeof operation.group === "string") {
    const children = copy.rules?.[operation.rule]?.when?.any;
    if (!Array.isArray(children)) fail();
    const index = children.findIndex((child: unknown) => record(child).changed === operation.group);
    if (index < 0) fail();
    children.splice(index, 1);
  } else if (operation.kind === "replace-paths" && typeof operation.group === "string" && Array.isArray(operation.paths) && operation.paths.every((path) => typeof path === "string")) {
    if (!copy.changes?.[operation.group]) fail();
    copy.changes[operation.group].paths = [...operation.paths];
  } else fail();
  return copy;
}

export function verifyMutations(policy: Policy, oracle: Oracle): void {
  for (const mutation of oracle.mutations ?? []) {
    if (mutation.operation.kind === "remove-binding") continue;
    const mutated = applyMutation(policy, mutation);
    let failed = false;
    try { verifyCoverage(mutated, oracle); } catch { failed = true; }
    if (!failed) fail();
  }
}
