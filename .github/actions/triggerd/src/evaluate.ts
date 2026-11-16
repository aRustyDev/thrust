import { classifyChanges, type ChangeRecord } from "./changes.js";
import { TriggerdError } from "./errors.js";
import type { ChangeSet, NotCollectedChangeSet } from "./types.js";

type Policy = Record<string, any>;
type Reason = "event-ineligible" | "full-check" | "global-invalidation" | "unmatched-path" | "matched" | "not-matched";
type Decision = { readonly selected: boolean; readonly reason: Reason; readonly groups: Readonly<Record<string, boolean>> };
export type Evaluation = {
  readonly decisions: Readonly<Record<string, Decision>>;
  readonly changeSet: ChangeSet;
  readonly coverageWarnings: readonly string[];
};
export type Context = {
  readonly event: string;
  readonly activity?: string;
  readonly scope: string;
  readonly fullCheck: boolean;
  readonly notCollectedReason?: NotCollectedChangeSet["reason"];
};

const EVENTS = new Set(["pull_request", "push", "workflow_dispatch", "schedule", "merge_group"]);
const NOT_COLLECTED_REASONS = new Set<NotCollectedChangeSet["reason"]>(["explicit-full-check", "manual", "schedule", "new-branch"]);
const compare = (left: string, right: string): number => left < right ? -1 : left > right ? 1 : 0;
const fail = (): never => { throw new TriggerdError("INPUT_INVALID", "Evaluation input invalid"); };
const isRecord = (value: unknown): value is Record<string, unknown> => Boolean(value) && !Array.isArray(value) && typeof value === "object";

function referencedGroups(expression: unknown, groups = new Set<string>()): Set<string> {
  if (!isRecord(expression)) return groups;
  if (typeof expression.changed === "string") groups.add(expression.changed);
  for (const child of [expression.all, expression.any]) if (Array.isArray(child)) child.forEach((item) => referencedGroups(item, groups));
  referencedGroups(expression.not, groups);
  return groups;
}

function evaluateExpression(expression: unknown, groups: Readonly<Record<string, boolean>>, activity = ""): boolean {
  if (typeof expression === "boolean") return expression;
  if (!isRecord(expression)) return false;
  if (typeof expression.changed === "string") return groups[expression.changed] === true;
  if (typeof expression.activity === "string") return activity === expression.activity;
  if (Array.isArray(expression.all)) return expression.all.every((child) => evaluateExpression(child, groups, activity));
  if (Array.isArray(expression.any)) return expression.any.some((child) => evaluateExpression(child, groups, activity));
  return !evaluateExpression(expression.not, groups, activity);
}

function groupValues(expression: unknown, values: Readonly<Record<string, boolean>>): Readonly<Record<string, boolean>> {
  const selected: Record<string, boolean> = Object.create(null);
  for (const id of [...referencedGroups(expression)].sort(compare)) selected[id] = values[id] === true;
  return Object.freeze(selected);
}

function scopeRules(policy: Policy, scope: string): readonly string[] {
  if (typeof scope !== "string") fail();
  const ids = Object.keys(policy.rules).sort(compare);
  if (!scope) return ids;
  const scoped = ids.filter((id) => id.startsWith(`${scope}.`));
  if (!scoped.length) fail();
  return scoped;
}

function changeSet(context: Context, changes: readonly ChangeRecord[] | null): ChangeSet {
  if (typeof context.fullCheck !== "boolean" || !EVENTS.has(context.event)) fail();
  if (context.fullCheck) {
    if (changes !== null) fail();
    const reason = context.notCollectedReason ?? "explicit-full-check";
    if (!NOT_COLLECTED_REASONS.has(reason)) return fail();
    return Object.freeze({ kind: "not-collected", reason });
  }
  if (changes === null || context.notCollectedReason !== undefined) return fail();
  return Object.freeze({ kind: "complete", count: changes.length });
}

export function evaluatePolicy(policy: Policy, context: Context, changes: readonly ChangeRecord[] | null): Evaluation {
  const collected = changeSet(context, changes);
  const classification = changes === null ? undefined : classifyChanges(policy, changes);
  const decisions: Record<string, Decision> = Object.create(null);
  for (const id of scopeRules(policy, context.scope)) {
    const rule = policy.rules[id];
    const groups = classification ? groupValues(rule.when, classification.groups) : Object.freeze(Object.create(null));
    if (!rule.events.includes(context.event)) {
      decisions[id] = Object.freeze({ selected: false, reason: "event-ineligible", groups });
    } else if (context.fullCheck) {
      decisions[id] = Object.freeze({ selected: true, reason: "full-check", groups });
    } else if (classification!.invalidated) {
      decisions[id] = Object.freeze({ selected: true, reason: "global-invalidation", groups });
    } else if (classification!.unmatched.length) {
      decisions[id] = Object.freeze({ selected: true, reason: "unmatched-path", groups });
    } else {
      const selected = evaluateExpression(rule.when, classification!.groups, context.activity);
      decisions[id] = Object.freeze({ selected, reason: selected ? "matched" : "not-matched", groups });
    }
  }
  return Object.freeze({
    decisions: Object.freeze(decisions),
    changeSet: collected,
    coverageWarnings: classification?.coverageWarnings ?? Object.freeze([])
  });
}
