# spike-0004

The harness for spike 0004 (topology and the `.woven` tunnel, public half): an OpenTofu root, three small JavaScript
Workers, a real-platform runner and the Python assertion suite that checks the Access, Worker and custom-domain
topology on Cloudflare. Method, results and the runbook:
[`research/0006-topology-public-half.md`](../../.agents/plans/contrail/research/0006-topology-public-half.md).

This is **throwaway spike tooling**, not product code. No crate imports it, and the three Workers are registered as
non-Rust spike pieces in the ADR-0006 register (round 29). It is retired when a Rust Worker re-run of the spike
replaces it.

## Provenance

| | |
|---|---|
| Source repository | `aRustyDev/plane` |
| Source branch | `chore/contrail-plan-scripts` (one signed commit, no PR) |
| Source commit | `dd575509c4a0227e7a4027ea79c7fa830a0281a5` |
| Source path | `.agents/plans/contrail/scripts/spike-0004/` (tree `d85476e4b284bec38d5239673bedf32734371f2d`) |
| Imported by | thrust issue #55 |

The files were extracted with `git archive` and are byte-identical to the source commit, including the executable bit
on `dev.sh` and `run_platform.sh`, and the nested `.gitignore` (it keeps `tofu/.terraform/`, `tofu/*.tfstate*`,
`terraform.tfvars`, `.wrangler/` and `tests/out/` out of git; the repository's root `.gitignore` covers none of them,
and the tofu state holds the service-token secret). No header was added to any file: `run_platform.sh` prints its own
usage with `sed -n '2,35p'`, so a new first line would shift it.

## What running it does

Read this before running anything here.

- `./dev.sh` is local only: `wrangler dev --local`, with Cloudflare credentials unset.
- `./run_platform.sh <step>` has local steps (`preflight`) and read-only ones (`token-verify`, `snapshot`, `compare`,
  `verify-present`, `verify-absent`), and **mutating** ones. `deploy` deploys three Workers named `spike-0004-*`,
  `apply` creates 10 Cloudflare resources (it refuses to apply unless the plan reads exactly
  `10 to add, 0 to change, 0 to destroy`), and `teardown` destroys them again. These touch a real account and zone.
- The runner refuses to start a step without its environment: `SPIKE_ACCOUNT_ID`, `SPIKE_ZONE_ID`, `SPIKE_ZONE_NAME`,
  `SPIKE_TEST_EMAIL`, `SPIKE_TEAM_NAME` and `SPIKE_OP_TOKEN_REF` (an `op://` reference, read with `op read`; the value
  is never printed or put on argv). None of those values is in this directory.
- `tests/delete_cert_packs.py` is a dry run unless given `--yes`. Its `DELETE` is the only non-`GET` call in the harness.
- Those guards live in `run_platform.sh`. A bare `wrangler deploy` or `tofu apply` run by hand from these directories
  is not guarded: `wrangler` uses whatever login is active.

## Running it

Python is run through `uv` only; no `pip`. Every entry script carries a PEP 723 header and is standard library only,
with one exception: `tests/jwt_verifier_test.py` depends on `cryptography`, which is **not version-pinned**, so `uv`
resolves the latest release when it runs. `tests/checks.py`, `latency.py`, `mockgate.py` and `rawhttp.py` have no
header because they are imported, not run.

```sh
# Local only, no Cloudflare credentials: generates throwaway keys, starts its own `dev.sh`, needs `wrangler` on PATH.
uv run tools/spike-0004/tests/jwt_verifier_test.py

# Local only too, but it needs the six variables above and `op`, `wrangler`, `tofu` and `uv` on PATH.
tools/spike-0004/run_platform.sh preflight
```

## Left as written

The scripts and `research/0006` still cite plan documents by their old location in a few comments (`run_platform.sh`
cites the runbook, and a local rule file, `rules/1password-op-cli.md`, that lives outside any repository). They were
not rewritten, to keep the copy byte-identical.

## Licence

AGPL-3.0-only, like the rest of this repository. The files carry no per-file header; they were written for the plan.
