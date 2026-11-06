// Shared "business logic" for the latency comparison (spike 0004, step 4).
//
// The edge Worker calls compute() in-process; the same function is exported by the
// second Worker (`spike-0004-b`) and reached through a service binding. Identical code
// on both sides means any latency delta between /lat/inproc and /lat/binding|rpc is the
// cost of the binding hop, not of different work.
//
// Deterministic, allocation-light, a few microseconds of CPU. NOT a benchmark of
// compute; it only has to be non-trivial enough that the optimiser cannot delete it.

export function compute(n) {
  const iterations = Math.max(1, Math.min(Number(n) || 20000, 2_000_000));
  let h = 0x811c9dc5; // FNV-1a 32-bit offset basis
  for (let i = 0; i < iterations; i++) {
    h ^= i & 0xff;
    h = Math.imul(h, 0x01000193) >>> 0;
  }
  return { iterations, hash: h };
}
