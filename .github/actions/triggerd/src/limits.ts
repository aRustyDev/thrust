export const LIMITS = Object.freeze({
  policyBytes: 64 * 1024,
  groups: 256,
  rules: 512,
  patterns: 2048,
  expressionDepth: 16,
  expressionNodes: 8192,
  patternBytes: 1024,
  patternSegments: 64,
  globstars: 8,
  pathBytes: 8192,
  pathSegments: 512,
  rawChanges: 100000,
  comparisons: 5000000,
  machineOutputBytes: 128 * 1024,
  summaryBytes: 32 * 1024,
  workerMs: 10000,
  totalMs: 120000
});
