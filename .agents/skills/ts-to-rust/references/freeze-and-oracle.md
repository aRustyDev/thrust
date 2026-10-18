# Freeze, controls, and the compatibility oracle

Stage 2 (Freeze), the controls that make stages 4 and 5 trustworthy, and the cross-cutting oracle for HTTP
surfaces. Each section carries its own provenance tag.

## 1. Freeze: golden fixtures from the TypeScript side `[verified: node v26.9.0]`

Before any Rust exists, run the TS module over a corpus and check in what it did. The fixtures are the contract.

**Corpus design** (edge cases first, then the happy path):

- every branch and every thrown error, with the message (callers may display it);
- garbage for `unknown` inputs: `null`, arrays, numbers where strings are expected, empty strings;
- keys inherited from `Object.prototype` (`"constructor"`): the worked example's TS accepted one as a valid
  priority, and the freeze caught it;
- empty, single-element and **large interleaved** collections: a stability bug only appeared with 60 items and
  duplicate keys, not with 50 identical ones (an all-equal input looks already sorted to a sort routine);
- Unicode: BOM (U+FEFF), U+0085, non-breaking and em spaces, astral characters, composed vs decomposed accents;
- numbers: `0`, `-0`, values above 2^53, non-integers where integers are expected;
- ties under `localeCompare`, accents, and case.

**Record the environment** in the fixture header (runtime version, locale, timezone) and freeze under a fixed
timezone and locale. Date-time strings without an offset are local time in JS.

**Lossless encoder.** JSON erases `undefined`, `NaN`, `-0`, `Date`, `Map`, `Set` and `BigInt`. Encode them so each
difference becomes a visible decision (it runs under Node 26.9.0 as a `.ts` file with no flags; or use the
project's own runner):

```ts
type Enc = null | boolean | number | string | Enc[] | { [k: string]: Enc };
function enc(v: unknown): Enc {
  if (v === undefined) return { $: "undefined" };
  if (v === null || typeof v === "boolean" || typeof v === "string") return v;
  if (typeof v === "number") {
    if (Number.isNaN(v)) return { $: "nan" };
    if (v === Infinity) return { $: "inf" };
    if (v === -Infinity) return { $: "-inf" };
    if (Object.is(v, -0)) return { $: "-0" };
    return v;
  }
  if (typeof v === "bigint") return { $: "bigint", v: v.toString() };
  if (typeof v === "function" || typeof v === "symbol") return { $: typeof v };
  if (v instanceof Date) return { $: "date", v: Number.isNaN(v.getTime()) ? null : v.toISOString() };
  if (v instanceof Map) return { $: "map", v: [...v].map(([k, x]) => [enc(k), enc(x)]) };
  if (v instanceof Set) return { $: "set", v: [...v].map(enc) };
  if (v instanceof Error) return { $: "error", name: v.name, message: v.message };
  if (Array.isArray(v)) return Array.from(v, enc);
  const o = v as Record<string, unknown>;
  const body = Object.fromEntries(Object.keys(o).map((k) => [k, enc(o[k])]));
  return "$" in o ? { $: "object", v: body } : body;
}
function capture(name: string, fn: (...a: any[]) => unknown, args: unknown[]) {
  try { return { name, args: enc(args), ok: enc(fn(...args)) }; }
  catch (e) { const err = e as Error; return { name, args: enc(args), throws: { name: err.name, message: err.message } }; }
}
```

Fixture layout: `fixtures/<module>.json` = `{ meta, cases: [{ name, args, ok | throws }] }`. For async functions
`await` inside `capture`. The freeze script is TypeScript and `Cargo.toml` is a manifest: both fall outside the
skill's edit grants, so writing them prompts the user (or the user writes them). Running the script through Bash
(which prompts) writes the fixtures; they are never hand-edited.

**Replay in Rust.** Deserialize into `serde_json::Value`, decode the tags, and compare **by value** (numbers via
`f64`, objects without regard to key order). Decide the mapping once and write it down: in the worked example a
property encoded `{"$":"undefined"}` means "key absent on the wire".

## 2. Controls: see the suite fail before you believe it passes `[verified: rustc 1.98.1, cargo 1.98.1]`

A frozen suite that cannot fail is decoration. Build all four into the replay itself (`worked-example.md` does):

1. **Mutated fixture.** In memory, change one expected value; the replay must report a failure. (Test:
   `control_a_mutated_fixture_must_fail`.)
2. **Error text is compared.** Change one expected error message; the replay must fail.
3. **Pinned size.** Assert the case count. A harness that silently loads fewer cases proves less. An empty
   corpus must fail.
4. **Stale exceptions fail.** A case listed in the exception register must keep differing; if it begins to match,
   the register entry is stale and the replay fails.

**Mutate the port as well.** Make one textual change at a time, run the suite, and require a failure at the case
you expected. In the worked example all 7 hand-made changes are caught: plain `trim()` instead of the JS trim set
(`trims_bom_zwnbsp`, `keeps_nel_u0085`), case-sensitive name order (`rank_case_ties`), rejecting `null` priority,
accepting an empty id, dropping `assignee: null`, rejecting an array as an object, and `sort_unstable_by` (only the
60-item interleaved case catches it).

Give every mutant its own build directory: with one shared `CARGO_TARGET_DIR`, stale mutant artifacts can leak into
later builds and fail the unmutated suite, which reads as a real regression until the target directory is cleared.
`cargo-mutants` automates this loop `[unverified: cargo-mutants not installed]`.

## 3. Compatibility oracle (HTTP surfaces) `[verified: python3 3.14.7, uv 0.12.18, grep (BSD grep, GNU compatible) 2.6.0-FreeBSD]`

Use when the port must behave like a running reference implementation, whether it is written in TypeScript,
Python or anything else (for example an API the port has to stay compatible with). The oracle is **recorded real
responses**, not hand-written expectations.

**Home.** `tests/oracle/` (relative to the project) holds `rules.json`, `exceptions.json`, `recorded/` and
`replay/`. The skill's edit grants cover `rules.json` and `exceptions.json` only. `recorded/` and `replay/` are
written by the recording and replay scripts or `curl` through Bash, which prompts, and are never hand-edited: a
hand-editable replay directory would let the agent forge the evidence the differ compares. Anything else outside
the grants (the TS freeze script, manifests) prompts the user.

**Credential and personal-data hygiene.** `scripts/oracle_diff.py` performs **no redaction**: normalisation
happens in memory, and the files on disk are exactly what was recorded.

- Record only from a throwaway, seeded instance of the reference implementation. Never record production.
- Store the request without headers (method, path, query, body). Supply authentication to the recording tool
  out of band, and never write `Authorization`, `Cookie` or `Set-Cookie` values to a file.
- Treat `tests/oracle/recorded/` as sensitive until it has been scrubbed. Before checking it in, run this scan,
  which prints **file names only** (so a matched secret is not echoed), review each file by hand, and read a
  sample of the rest. It looks for credentials and email addresses; other personal data needs review by eye. On a
  hit, fix the recorder (strip the field at the source) and re-record; the user may scrub the file by hand; the
  agent does not edit a recording in place.

```sh
grep -rliE 'authorization|cookie|bearer |token|secret|passw(or)?d|api[_-]?key|session|jwt|eyJ[A-Za-z0-9_-]{10,}|BEGIN [A-Z ]*PRIVATE KEY|[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}' tests/oracle/recorded
```

  A clean directory exits 1 with no output; planting a file for each category (token, email, JWT-like string,
  private-key header, cookie) lists exactly those files (control).

**Record.** Run one scripted request sequence against the reference implementation. Store `{request, status,
body}` per case, including error cases (4xx), empty lists, pagination edges and unusual payloads.

**Replay.** Run the same request script against the port with equivalent seed data (a Bash call, which prompts); one
file per case with the same name under `tests/oracle/replay/`, shaped `{"status": 200, "body": ...}`, never
hand-edited. `status` must be an integer and `body` must be present, or the differ exits 2.

**Normalise volatile fields** with reasoned rules: generated ids, timestamps, tokens, sequence numbers,
pagination cursors, anything ordered by an unordered query. Every rule needs a reason; a rule that erases most of
the data is itself a failure.

**Compare** with the bundled differ (offline, stdlib-only, deterministic; needs `uv` with a local Python 3.10+, or
plain `python3`; `scripts/` is in the skill's directory, `tests/oracle/` in the project):

```
uv run --offline --script scripts/oracle_diff.py compare tests/oracle/recorded tests/oracle/replay \
  --rules tests/oracle/rules.json [--exceptions tests/oracle/exceptions.json] [--max-normalised-pct 50]
```

- `rules.json`: `[{"path": "*/id", "as": "<id>", "reason": "server-generated id"}]`. Paths are JSON-pointer-like
  and `*` is the **only** wildcard (any run of characters), so `*/id` covers an `id` at any depth while `/id` is
  only the top level (a `/id` rule leaves `/results/0/id` unmasked, and the differ then correctly fails). `?`, `[`
  and `]` are rejected, not treated as glob syntax. Matching has no backtracking, so a pattern with many `*` stays
  cheap. A rule matches scalar values only; to mask an object, match its fields. A bare-scalar body sits at the path
  `/`, in rules and in reports alike, so a rule at `/` can mask it (with one value masked, raise
  `--max-normalised-pct` to 100 or the over-normalisation check fires). A masked value keeps its JSON type, so a
  rule can hide a value but not the difference between `null` and a string. A rule that matches no value is
  reported as a `WARN` line.
- `exceptions.json`: `[{"case": "issue_create", "path": "/priority", "reason": "...", "revisit": "..."}]`, the
  **exception register** of deliberate differences. `case`, `path` and `reason` are all required, and a
  wildcard-only `case` or `path` is rejected so that one entry cannot hide every difference. The exact path `/` is
  concrete: it registers a top-level difference (a bare-scalar body, a top-level array length). A difference in
  the HTTP status has the reserved path `#status`; a body field called `status` is `/status`.
- It ignores key order, treats `1` and `1.0` as equal, and treats `null` and a missing key as different.
- It exits 1 on any unexcepted difference, a recorded case that was not replayed, a replayed case that was never
  recorded, **zero cases**, a **stale exception**, or normalisation above `--max-normalised-pct` (default 50).
- It exits 2 on malformed input: unreadable or invalid JSON, a case without an integer `status` or a `body`, a
  rule or exception missing a field, a wildcard-only or glob-syntax exception, or `--max-normalised-pct` outside
  0 to 100.

**The harness needs its own must-fail control.**

- `uv run --offline --script scripts/oracle_diff.py selftest` runs in-memory cases and must end `SELFTEST PASS`.
  Pass cases: identical responses, volatile-only differences, `1` vs `1.0`, a reasoned exception, an exception at
  `#status`, an exception at `/` for a top-level difference, a literal `.` in a rule path, a 41-star pattern over
  a 200-character key. Fail cases: one changed value, missing and extra fields, null vs missing, a masked field
  that is `null` on one side and a string on the other, array element and length, status, `true` vs `1`, an
  exception for a different path or case, an exception for `/status` against an HTTP status difference, a stale
  exception, zero cases, an unreplayed case, a replayed case never recorded, and an erase-everything rule.
  Usage-error cases: a rule without a reason, an exception without a case, with a wildcard-only value (`*`, `//`)
  or with `?` or a bracket class, malformed case files, and an out-of-range `--max-normalised-pct`. Direct checks
  of the matcher cover prefix, suffix, ordered middles and non-overlap, including a 40-star pattern with a missing
  middle over 199 characters. Rules at `/` over a bare-scalar body are covered too (masked pass, no-rule fail, type
  kept).
- End to end, alter one replayed value and expect exit 1 with a `FAIL <case>` line naming the path; the sample
  output is in `SKILL.md`.

**Limits.** The oracle proves equivalence only on the recorded requests; coverage is the corpus. Re-record when
the reference implementation changes, and review the diff of the recordings as you would review code.
