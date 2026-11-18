# `infra/edge`

The Cloudflare resources the edge Worker needs (ADR-0111, issue #195): the Flagship app it evaluates, and the Access
applications that say who may opt in and that only the edge reaches the production Worker's Previews. Applied by
[edge-infrastructure.yml](../../.github/workflows/edge-infrastructure.yml), from the protected environment `edge-infrastructure`.

## Its own state, on purpose

State is on R2, bucket `tofu`, key `products/thrust/edge/646535719915/terraform.tfstate`. **No other root may use that key**:
the workflow's `test` job fails when another workflow names it.

That is a rule, not a style point. `infra/cloudflare` and the infrastructure repository's `products/thrust/cloudflare` share one
key while declaring different resources. Only the second declares production's and staging's buckets, queues and Custom Domains,
so a plan of the first proposes destroying them, and an apply of either would destroy what only the other declares. Nothing
here touches that state.

## Owns

| Resource | What |
|---|---|
| `cloudflare_flagship_app.thrust` | The Flagship app `thrust`. The app only: every flag in it is thrust CI's (`scripts/flagship.py`, from `.github/deploy/flagship/policy.json`), and the workflow refuses a `cloudflare_flagship_flag` here |
| `cloudflare_zero_trust_access_application.opt_in["<audience>"]` | One per audience of `policy.json` (`dev`, `rc`), on `<shared_hostname>/_edge/opt-in/<audience>` only, a public path: the rest of the hostname stays open |
| `cloudflare_zero_trust_access_policy.opt_in_people["<audience>"]` | `allow` for the members of that audience's Access groups, named by id (`opt_in_group_ids`); made only for an audience with groups |
| `cloudflare_zero_trust_access_policy.opt_in_ci` | `non_identity` for the service token `thrust-ci-opt-in` (`ci_opt_in_token_id`), on every opt-in path, after the people |
| `cloudflare_zero_trust_access_application.previews` | Destination `preview_worker` = the production Worker (`production_worker_id`): its Previews, which the edge forwards opted-in requests to |
| `cloudflare_zero_trust_access_policy.edge_previews` | `non_identity` for the service token `thrust-edge-previews` (`edge_previews_token_id`), the only credential the Previews admit |

An opt-in path with no group and no CI token admits no one. The Previews application is made only once both its token and the
Worker are given.

**Not here:**
- the service tokens' secrets: the provider keeps a token's `client_secret` in state, and this state is unencrypted. `service-token.py` makes them;
- who is in the groups: thrust is public, so the groups and their members are kept in Cloudflare (or the infrastructure repository), and only their ids come here;
- the move of `thrust.sow.foo` to the edge: the infrastructure repository owns that Custom Domain.

## Inputs

All of them are identifiers. They reach the plan as `TF_VAR_*` from the environment's variables, never from a file in this
tree.

| Variable of `edge-infrastructure` | Terraform variable | What |
|---|---|---|
| `CLOUDFLARE_ACCOUNT_ID` | `cloudflare_account_id` | the account |
| `EDGE_OPT_IN_GROUP_IDS` | `opt_in_group_ids` | JSON, `{"rc": ["<group id>"], "dev": ["<group id>"]}`; group ids (UUIDs) only, never an email, which the plan refuses |
| `EDGE_CI_OPT_IN_TOKEN_ID` | `ci_opt_in_token_id` | the `id=` that `service-token.py` prints for `thrust-ci-opt-in` |
| `EDGE_PREVIEWS_TOKEN_ID` | `edge_previews_token_id` | the `id=` it prints for `thrust-edge-previews` |
| `EDGE_PRODUCTION_WORKER_ID` | `production_worker_id` | the production Worker's script tag: the `tag` of `thrust-production` in `GET /accounts/<id>/workers/scripts` |

`shared_hostname` (`thrust.sow.foo`) and `flagship_app_name` (`thrust`) have defaults.

**The environment's secrets:**
- `CLOUDFLARE_API_TOKEN`, with Flagship app Edit and Access: Apps and Policies Edit;
- `TOFU_R2_ACCESS_KEY_ID` and `TOFU_R2_SECRET_ACCESS_KEY`, with the variable `TOFU_R2_S3_ENDPOINT` (the R2 backend).

**The environment's protection:** deployment branches `main`, and required reviewers. The dispatch refuses to run without them.

## Switching the edge on

1. **The two service tokens.** Run with `CLOUDFLARE_ACCOUNT_ID` and a token holding Access: Service Tokens Edit:

   ```sh
   python3 infra/edge/service-token.py ensure --name thrust-ci-opt-in     --credentials-file <private path>/thrust-ci-opt-in.json
   python3 infra/edge/service-token.py ensure --name thrust-edge-previews --credentials-file <private path>/thrust-edge-previews.json
   ```

   Each prints its `id` and `client_id`, never the secret, which stays in the file. Keep the files where secrets are kept: a
   token's secret cannot be read back. `check` verifies a file against Cloudflare and says when the token expires (one year
   by default, `--duration`). `ensure` refuses to make a token whose name exists without its file.
2. **The environment `edge-infrastructure`,** as above, with the two token ids, the groups and the Worker's tag.
3. **Run the workflow** with `plan`, read the plan, then run it with `apply`. On the first apply the plan adds the app, the two
   opt-in applications and their policies, and the Previews application and its policy.
4. **Put the outputs and the credentials where thrust reads them:**

   | From | To |
   |---|---|
   | output `flagship_app_id` | repository variable `FLAGSHIP_APP_ID`, which turns `flagship.yml` on |
   | output `opt_in_aud["rc"]`, `opt_in_aud["dev"]` | environment `edge`: `THRUST_ACCESS_AUD_RC`, `THRUST_ACCESS_AUD_DEV` |
   | `thrust-edge-previews.json`: `client_id`, `client_secret` | environment `edge`: `PREVIEW_ACCESS_CLIENT_ID`, `PREVIEW_ACCESS_CLIENT_SECRET` |
   | `thrust-ci-opt-in.json`: `client_id`, `client_secret` | environments `target`, `target-migrate`: `EDGE_CI_CLIENT_ID`, `EDGE_CI_CLIENT_SECRET` |

5. **The Flagship tokens,** made in the dashboard, scoped to this app:
   - one with Write and Read, as `FLAGSHIP_WRITE_TOKEN` in the environments `flagship`, `target` and `target-migrate`;
   - one with Read, as `FLAGSHIP_READ_TOKEN` in `flagship`.

The rest of the switch is thrust's: [the edge's deploy](../../docs/src/architecture/deploy.md#the-edges-deploy-off-until-the-cut-over).

## Checking it without credentials

```sh
tofu -chdir=infra/edge init -backend=false && tofu -chdir=infra/edge validate && tofu -chdir=infra/edge test
python3 -I infra/edge/test-service-token.py
```

`contract.tftest.hcl` runs on a mocked provider. It pins:
- the opt-in paths: exactly `policy.json`'s audiences, each on its own path;
- their policies: groups first, then the CI token;
- the Previews: they admit only the edge's token;
- the inputs: an email, a client id, a Worker name, a hostname with a path, and an audience the edge has no path for are each
  refused.

Each of 20 hand mutants of the configuration fails it, and each of 16 of `service-token.py` fails its test.

**Not checked here:** the real API's reading of these resources. The live spike (`research/0016`) made the same shapes through
the API: a path application, and a `preview_worker` destination with a `non_identity` policy. It did not make them through this
provider. The first `plan` and `apply` are that check.
