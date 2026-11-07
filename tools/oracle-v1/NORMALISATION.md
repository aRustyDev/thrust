# Normalisation rules and the exception register (Q-0122.2)

Two machine-readable files, one purpose: let two runs of the pinned Plane (or Plane and the Rust v1
slice) be compared byte-for-byte **without** hiding a real behavioural difference.

| File | Role | Consumed by |
| --- | --- | --- |
| `oracle/normalise.yaml` | what is *erased* before comparing | `harness/normalise.py` (`--self-test` proves each rule) |
| `oracle/exceptions.yaml` | what is *accepted as different*, and why | `harness/run_conformance.py`, `harness/run_controls.py` |

## Normalisation (`normalise.yaml`)

The rule of thumb: erase what a correct implementation **cannot** reproduce, keep everything it **must**.

| Class | Rule | Why |
| --- | --- | --- |
| uuid | seeded ids become symbolic (`<project:alpha>`, `<issue:i3>`); any other uuid becomes `<uuid:N>` by first appearance within one recording. **Dashless 32-hex ids** (storage keys are `uuid4().hex-<filename>`) get the same treatment, canonicalised so one id written both ways is one id | ids made during a run are random; seeded ids are deterministic (`uuid5`) and carry meaning |
| random defaults | `logo_props.icon.name` / `.color` become `<random>` | the server picks a random default icon for every created project; the contract is "an icon with a name and a colour" |
| timestamps | a *value* is replaced (`<ts>`, `<date>`) only if its **key** is time-like **and** it parses; `null` is left alone | "unset vs set" is behaviour, the instant is not. `*_date` seeds are relative to today |
| `sequence_id` | **never normalised** (also `sort_order`, cursors, counts) | ADR-0120: sequencing is behaviour a schema cannot express. Scenario order is fixed, so the value is reproducible; a mismatch is a real parity failure |
| ordering | arrays stay in API order, compared strictly | default `order_by`/`sort_order` is behaviour. Genuinely unordered arrays go in the register as `kind: ordering`, not silently sorted |
| opaque | presigned-upload signature/policy/credential/key/url become `<opaque>`; field *names* are kept | per-request, unreproducible; the contract is the shape |
| headers | whitelist only (`content-type`, `allow`, `www-authenticate`, `location`, `retry-after`, `x-ratelimit-*`, `cache-control`); `x-ratelimit-reset` is `<epoch>`; counters are `<int>` | the rest is transport noise. `x-ratelimit-remaining` is kept **verbatim in the `rate_limit` resource**, where it must count down 4,3,2,1 |

The last two rows were **not in the first rule set**: the CHECK-mode diff (two recordings on independent fresh stacks) failed on them and
nothing else would have found them. Keep that stage in CI.

Proof that this is enough and not too much: recording the corpus on two independent fresh stacks gives
byte-identical files (`run-all.sh` CHECK mode: `corpus-reproducible`), while the same recordings
**un-normalised** differ (ids, timestamps): `harness/check_normalisation.py` counts the volatile values in the raw recordings (hundreds) and requires zero in the committed corpus.

## Exception register (`exceptions.yaml`, `format_version: 1`)

Three sections:

* `gate` — configuration of the conformance gate (tool version, checks, seed, vacuity floors), so a result is reproducible.
* `scope_exclusions[]` — operations the gate does **not** exercise (`id`, `method` and/or `path_regex`, `reason`, `covered_by`).
* `entries[]` — failures the gate **accepts**.

An entry:

```yaml
- id: EX-006                      # stable; cite it in DEVIATIONS.md
  kind: upstream-schema-defect    # upstream-schema-defect | upstream-bug | intentional-deviation | harness-limit | ordering
  applies_to: conformance         # conformance | corpus | differential
  status: active                  # active | retired
  optional: false                 # true only for findings that depend on fuzz luck; otherwise STALE if it matches nothing
  match:
    check: response_schema_conformance   # required (Schemathesis check name)
    failure_type: JsonSchemaError        # optional
    methods: [GET]                       # optional
    operations: ["GET /api/v1/..."]      # optional, exact operation labels   (or operation_regex)
    message_regex: 'null is not of type' # optional, searched in the failure message
  reason: why Plane differs from its own schema here
  evidence: how it was observed (file, command, date)
  rust_expectation: reproduce | fix | n/a    # what the Rust v1 slice must do
  deviation_ref: null                        # DEVIATIONS.md id once an intentional deviation is decided
```

The gate is deliberately strict in **both** directions:

* a failure that matches no entry **fails the gate** (a new difference must be reviewed, not ignored);
* an `active`, non-`optional` entry that matches **nothing** is **stale and fails the gate** (an exception that no longer fires silently widens the hole);
* a run that tests fewer than `min_operations_tested`, or reaches a 2xx on fewer than `min_operations_with_2xx`, fails as **vacuous**;
* a Schemathesis runtime error on an operation is a failure, never a pass.

For the Rust run the same register is reused: its failure set must equal the oracle's. Entries with
`rust_expectation: reproduce` must still fire; entries with `fix` must **stop** firing (and become
`intentional-deviation` rows in `DEVIATIONS.md`).
