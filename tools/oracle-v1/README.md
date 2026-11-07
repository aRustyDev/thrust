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

## Edits after the import

Each edit to a vendored file is its own commit and is listed here with the reason.

1. `exceptions.yaml`, the round-27 corrections that the source commit never received (ADR-0031 round 27,
   `research/0005` "Corrections"). EX-001 and EX-002 change from `rust_expectation: fix` to `reproduce` (owner
   decision Q-0033.2: reproduce every Plane quirk, including the 500s, as a handled error, never a crash). The `reason`
   of EX-011 said search results omit `id`; they do not, the body is the `issues` wrapper. EX-003, EX-004 and EX-010
   gain the fixture and component-collision notes. Only `reason` and `rust_expectation` change; `match`, `gate` and
   `scope_exclusions` are untouched, so no failure is newly accepted or rejected.
