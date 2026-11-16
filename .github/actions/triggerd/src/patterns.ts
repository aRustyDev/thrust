import { minimatch } from "minimatch";
import { TriggerdError } from "./errors.js";
import { LIMITS } from "./limits.js";

const fail = (): never => { throw new TriggerdError("CONFIG_INVALID", "Policy validation failed"); };

export function compilePattern(pattern: string): (path: string) => boolean {
  if (!pattern || Buffer.byteLength(pattern, "utf8") > LIMITS.patternBytes || pattern.includes("\0") || pattern.startsWith("/") || pattern.startsWith("./") || pattern.includes("\\") || /[{}[\]()!]/.test(pattern)) fail();
  const segments = pattern.split("/");
  if (segments.length > LIMITS.patternSegments || segments.some((segment) => !segment || segment === "." || segment === ".." || (segment.includes("**") && segment !== "**"))) fail();
  if (segments.filter((segment) => segment === "**").length > LIMITS.globstars) fail();
  return (path: string) => minimatch(path, pattern, {
    dot: true,
    nocase: false,
    nobrace: true,
    noext: true,
    nonegate: true,
    optimizationLevel: 0,
    platform: "linux"
  });
}
