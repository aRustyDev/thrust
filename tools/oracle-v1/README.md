# oracle-v1

The Plane v1 oracle: the pinned OpenAPI schema, an exception register, a golden corpus recorded from Plane, and
the Schemathesis conformance harness built by spike 0008 (bead `plane-b6a`). Thrust runs it against its own
`/api/v1` routes (issue #58). Method and numbers:
[`research/0005-oracle-harness.md`](../../.agents/plans/contrail/research/0005-oracle-harness.md); decision record:
ADR-0122.

This is tooling, not product code. No crate imports it (ADR-0006 register).

## Provenance

| | |
|---|---|
| Source repository | `aRustyDev/plane` |
| Source branch | `feat/plane-b6a-upstream-sync-oracle` (local, unpushed) |
| Source commit | `72b2e014972cc30b38d56290da501c56af866c65` |
| Source path | `oracle/` (tree `dda199cc2a796937a5964d36cb949e5123e44e74`, 50 files) |
| Pinned Plane | tag `oracle/v1-pin-20261018` -> `ba85d4d3d3` (`PIN`) |
| Imported by | thrust issue #58 (resolves Q-0122.4) |

`SOURCE` is the machine-readable form. The files were extracted with `git archive`, so they are byte-identical to
the source commit apart from the edits below; the import commit's `tools/oracle-v1/` tree hashes to the source tree
above except for `SOURCE` and this file. Check with `git ls-tree` against the fork:

```sh
git -C <fork clone> rev-parse 72b2e01497:oracle   # dda199cc2a796937a5964d36cb949e5123e44e74
```

## What a Thrust run does not need, and what it does

The harness runs against any HTTP target. Running **Plane** (the baseline and the corpus recordings) needs the
fork checkout, because `compose.oracle.yml` builds from `../apps/api` and `scripts/verify-pin.sh` reads the pin's
git ancestry. Point at it with `ORACLE_DIR=<fork checkout>/oracle`, as `tools/compat-record/fresh-oracle.sh` does.
Running against Thrust needs only this directory.

## Running it

Python is run through `uv` only; every script is a PEP 723 `uv run --script` file. Schemathesis is fetched at run
time with `uvx --from schemathesis==4.31.1`. `oasdiff` runs from an image pinned by digest; never pass `--open`
(it uploads the specs).

Against Thrust, through `scripts/oracle-v1.sh` (repository root):

```sh
scripts/oracle-v1.sh native-memory            # also: native-pg, worker-memory, worker-cf
scripts/oracle-v1.sh native-memory controls   # and the must-fail controls C0-C4
scripts/oracle-v1.sh plane                    # the pin, for the numbers the scope's floors are measured on
```

It starts the target as `scripts/contract.sh` does, then runs the scoped gate (`run_conformance.py --scope m1`), the
corpus comparison (`record_corpus.py --only`, then `compare_corpus.py --scope m1`) and the comparison's own controls
(`compare_corpus.py --self-test`), each from the oracle's seed, and ends in `VERDICT: PASS` or `VERDICT: FAIL`. The
results are committed under `results/m1/`.

Two things about the pin that a recording of it depends on. Plane soft-deletes what a deleted project holds in a
Celery task, so a second `DELETE` of a project answers 404 until its worker has run and 403 after: the corpus records
the settled 403 (`record_corpus.py` waits for it, see the `settle` of `projects/delete-again`), which needs the
worker running; the gate runs with it stopped (`scripts/db-state.sh restore` leaves it stopped). And a gate run on a
database that is not the seeded one fails for that reason, so `restore` checks the row counts.

## Edits after the import

Each edit to a vendored file is its own commit and is listed here with the reason.

1. `exceptions.yaml`, the round-27 corrections that the source commit never received (ADR-0031 round 27,
   `research/0005` "Corrections"). EX-001 and EX-002 change from `rust_expectation: fix` to `reproduce` (owner
   decision Q-0033.2: reproduce every Plane quirk, including the 500s, as a handled error, never a crash). The `reason`
   of EX-011 said search results omit `id`; they do not, the body is the `issues` wrapper. EX-003, EX-004 and EX-010
   gain the fixture and component-collision notes. Only `reason` and `rust_expectation` change; `match`, `gate` and
   `scope_exclusions` are untouched, so no failure is newly accepted or rejected.
2. `harness/run_conformance.py` and `harness/st_hooks.py`: the scoped gate (issue #58). `--scope <name>` runs only the
   operations of the register's `scopes.<name>` (Schemathesis `--include-tag`, JSON request bodies only through the
   `before_load_schema` hook of `st_hooks.py`), restricts the register to them, counts staleness per (entry, operation)
   and measures the floors on the scope. `--target plane|thrust` selects the entries that apply to the target
   (`targets:`, `absent_on_thrust`). Without `--scope` the run is the whole-schema run it was.
3. `exceptions.yaml`: `scopes.m1` (tags, floors measured on the pin, `entry_operations`, the corpus resources and their
   floor), EX-013 (an upstream schema defect the scoped run found: `expand` is not modelled), EX-014 (Thrust-only: slug
   folding, D-031) and EX-C01 to EX-C04 (`applies_to: corpus`: the differences a recording of Thrust may have from the
   committed corpus). No existing entry changes.
4. `harness/run_controls.py`: `--scope` and `--target` (the controls' gate is the scoped one), and a control C4 through the
   corpus comparison (a State without its `id` is recorded through the proxy and the comparison must fail on it).
5. `harness/record_corpus.py`, `corpus/work_items.json`, `corpus/INDEX.json`: a step may carry a `settle` probe (repeated,
   not recorded, until it answers) for work that the pin does in a Celery worker; `projects/delete-again` uses it, so that
   its answer (403) is the settled one and not a race. Three exchanges are added at the end of `work_items`, recorded on the
   pin: create, `DELETE .../issues/{pk}/` (204) and the detail after (404). It is the one operation of the scope the corpus
   had no exchange for. The 46 exchanges before them are byte-identical to the committed ones.
6. `scripts/db-state.sh`: paths of this layout (it was written for the fork's `oracle/`), and `restore` checks the row
   counts of the seed.
New: `harness/compare_corpus.py` (the corpus comparison and its controls).
