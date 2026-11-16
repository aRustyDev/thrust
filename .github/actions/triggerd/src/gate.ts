import { TriggerdError } from "./errors.js";

type Bindings = { readonly planner: string; readonly bindings: Record<string, string>; readonly needs: Record<string, readonly string[]> };
type NeedResult = { readonly result: string };
const fail = (): never => { throw new TriggerdError("OUTPUT_INVALID", "Completion gate input invalid"); };
const record = (value: unknown): Record<string, unknown> => value && typeof value === "object" && !Array.isArray(value) ? value as Record<string, unknown> : fail();
const same = (left: readonly string[], right: readonly string[]): boolean => left.length === right.length && left.every((value, index) => value === right[index]);
const sort = (values: readonly string[]): string[] => [...values].sort((left, right) => left < right ? -1 : left > right ? 1 : 0);
const unique = (values: readonly string[]): boolean => new Set(values).size === values.length;

export function validateBindings(ruleIdsInput: readonly string[], bindingsInput: unknown): Bindings {
  if (!unique(ruleIdsInput)) fail();
  const data = record(bindingsInput);
  const planner = typeof data.planner === "string" && data.planner ? data.planner : fail();
  const bindings = record(data.bindings);
  const declaredNeeds = record(data.needs);
  const ruleIds = sort(ruleIdsInput);
  if (!same(sort(Object.keys(bindings)), ruleIds)) fail();
  const jobs = new Set<string>();
  for (const rule of ruleIds) {
    const job: string = typeof bindings[rule] === "string" ? bindings[rule] : fail();
    if (!job || jobs.has(job)) fail();
    jobs.add(job);
  }
  const jobIds = sort([...jobs]);
  if (!same(sort(Object.keys(declaredNeeds)), jobIds)) fail();
  for (const job of jobIds) {
    const dependencies = declaredNeeds[job];
    if (!Array.isArray(dependencies) || !dependencies.every((value) => typeof value === "string") || !unique(dependencies) || !dependencies.includes(planner) || dependencies.some((value) => value !== planner && !jobs.has(value))) fail();
  }
  const cleanBindings: Record<string, string> = Object.fromEntries(ruleIds.map((rule) => [rule, bindings[rule] as string]));
  const cleanNeeds: Record<string, readonly string[]> = Object.create(null);
  for (const job of jobIds) cleanNeeds[job] = Object.freeze([...(declaredNeeds[job] as string[])]);
  return Object.freeze({ planner, bindings: Object.freeze(cleanBindings), needs: Object.freeze(cleanNeeds) });
}

export function verifyGate(rulesInput: unknown, needsInput: unknown, bindingsInput: unknown): { readonly selected: readonly string[]; readonly overhead: readonly string[] } {
  const rules = record(rulesInput);
  const needs = record(needsInput);
  const bindingData = record(bindingsInput);
  const bindings = record(bindingData.bindings);
  const ruleIds = sort(Object.keys(bindings));
  const validated = validateBindings(ruleIds, bindingsInput);
  const { planner, bindings: declaredBindings, needs: declaredNeeds } = validated;
  if (!same(sort(Object.keys(rules)), ruleIds)) fail();
  const jobs = new Map<string, string>();
  for (const rule of ruleIds) {
    if (typeof rules[rule] !== "boolean") fail();
    jobs.set(declaredBindings[rule], rule);
  }
  const jobIds = sort([...jobs.keys()]);
  if (!same(sort(Object.keys(declaredNeeds)), jobIds) || !same(sort(Object.keys(needs)), sort([planner, ...jobIds]))) fail();
  const plannerResult = record(needs[planner]);
  if (plannerResult.result !== "success") fail();
  const selected: string[] = [];
  const overhead: string[] = [];
  for (const job of jobIds) {
    const rule = jobs.get(job)!;
    const required = declaredNeeds[job];
    const result = record(needs[job]) as NeedResult;
    if (!(["success", "failure", "cancelled", "skipped"] as const).includes(result.result as "success")) fail();
    if (rules[rule] === true) {
      for (const prerequisite of required) {
        const prerequisiteRule = jobs.get(prerequisite);
        if (prerequisiteRule && rules[prerequisiteRule] !== true) fail();
      }
      if (result.result !== "success") fail();
      selected.push(rule);
    } else if (result.result === "failure" || result.result === "cancelled") fail();
    else if (result.result === "success") overhead.push(rule);
  }
  return Object.freeze({ selected: Object.freeze(sort(selected)), overhead: Object.freeze(sort(overhead)) });
}
