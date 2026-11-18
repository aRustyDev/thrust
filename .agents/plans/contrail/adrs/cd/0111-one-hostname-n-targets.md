---
status: accepted
date: 2026-11-16
decision-makers: asmith
consulted: []
informed: []
---

# ADR-0111: One hostname for every deployed target, routed by an edge Worker over Worker Previews, decided by Flagship

## Context and Problem Statement

Until now each environment had its own Worker, data and hostname. Only staging existed (`thrust-staging` at
`thrust-staging.sow.foo`, its own schema on `thrust-pg-dev`, its own bucket and queue). Production had no Worker,
hostname or resources yet. The owner asked for one hostname that serves production **and** any number of other live
targets: a release `vX.Y.Z`, the alias `staging` for the open release candidate, and `dev-*` builds of pull requests.
Cloudflare Flagship decides which target a request reaches, and every flag is made and changed by CI/CD (#195).

That puts several versions of the code in front of users at once, some of them on production's data. So it is also a
decision about data, migrations and collaboration rooms, not only about routing.

## Decision Drivers

* One hostname, `thrust.sow.foo`, for every target (the owner, 2026-11-15).
* Any number of live targets, each with its own code. Some use production's data; some must not.
* Flagship chooses, and only CI/CD writes flags. The repository is public, so no person's identity may be written into
  it.
* Ordinary users must never depend on the routing machinery: production keeps working if Flagship, or the routing to
  other targets, fails.
* Built-ins first (the owner's standing rule): hand-write only what Cloudflare does not provide.
* The pipeline of ADR-0110 (expand migration, attested bundle, smoke with a control, rollback, manual gates) keeps
  applying.

## Considered Options

* Option 1: one Worker, with a candidate version at 0 % reached by version overrides
* Option 2: Workers for Platforms (a dispatch namespace of target Workers behind a dispatch Worker)
* Option 3: Worker Previews of `thrust`, reached by an edge Worker that Flagship steers
* Option 4: a fixed pool of ordinary Workers bound to an edge Worker
* Option 5: one hostname per environment (what existed)

## Decision Outcome

**Chosen option: Option 3** (the owner, 2026-11-15, over Options 2 and 4 when they were put side by side; Option 1
was ruled out because a deployment holds at most two versions; Option 5 is what the request replaces). The live spike
of 2026-11-16 (`research/0016`) proved the parts the documentation did not settle.

The owner's decisions, all 2026-11-15:
* one hostname, `thrust.sow.foo`; `thrust-staging.sow.foo` is removed and CI also goes through the edge;
* N targets named `prod`, `vX.Y.Z`, `staging` (an alias) and `dev-*`;
* a request reaches a non-production target only by **explicit opt-in**, and the identity behind the opt-in comes from
  Cloudflare Access on the opt-in path only;
* a target that shares production's data and brings a new migration waits for a reviewer;
* every Flagship flag is created and changed by CI/CD.

### The request path

`thrust.sow.foo` is a Custom Domain on `thrust-edge`, a small Rust Worker (workers-rs) that holds no application code
and no data binding.
* A request without a valid pin cookie, or one the rules do not allow, goes through the service binding `THRUST` to
  production `thrust`. Production has no route of its own and `workers_dev = false`.
* A pinned and allowed request is fetched from that target's Preview URL **on workers.dev**, with an Access service
  token. Cloudflare refuses subrequests to custom-domain Preview URLs, and the Access app on thrust's Previews admits
  only that token, so the edge cannot be bypassed and Flagship cannot be skipped.

For every request the edge:
1. drops a client's `cf-access-*`, `Cf-Access-Jwt-Assertion` and `Cloudflare-Workers-Version-*` headers and its
   `CF_Authorization` cookie;
2. checks the pin cookie `__Host-thrust_edge`, an HMAC over `{principal, kind, audience, requested, exp}` under the
   secret `EDGE_PIN_KEY`; without a valid one it sends the request to production and calls nothing else;
3. evaluates Flagship. The global switch `routing` must be on; `alias-<requested>`, when it exists, names the target;
   `target-<name>`, evaluated with `{kind, audience}`, returns the target only when its rules allow it. Anything else
   goes to production;
4. fetches the target with `redirect: manual`, sets `X-Forwarded-Host`, rewrites a `Location` that names the Preview
   host, removes the `Set-Cookie: CF_Authorization` that Access adds, passes WebSocket upgrades through, and adds
   `x-thrust-route`.

The opt-in endpoints are the only paths behind Access on the shared hostname: `/_edge/opt-in/<audience>` (one Access
path per audience, for example `rc` and `dev`), `/_edge/opt-out` and `/_edge/whoami`. Access decides who may opt in,
and its policies live in the infrastructure repository. The edge takes the audience from the path that admitted the
person, because group names are not in the Access token, and recognises CI by the service token's `common_name`.

### Flagship

The app is made by OpenTofu (`cloudflare_flagship_app`) with an app-scoped token. Everything inside it is made by
thrust's CI, from definitions in this repository:
* `routing` (boolean, default off): the kill switch for every non-production target;
* `alias-staging`, `alias-prod` (strings naming targets), written by the release pipeline;
* `target-<name>` (JSON), one per live target, created by its deploy job and deleted by its teardown. Its value is
  `{url, deployment, release, kind, data, expires}`. Its default variation is empty (denied), and its rules come from a
  policy per kind, which names kinds and audiences, never people. Disabling one flag removes that one target.

A changed flag reached the edge in 6 s in the spike; the documentation promises 30 s. The edge calls Flagship through
`#[wasm_bindgen] extern` method imports, the only form that works on its RPC methods from workers-rs (`research/0016`).

### Targets and their data

A target is a Preview of `thrust` deployed from an attested bundle with
`wrangler preview --name <target> --json --ignore-base-config` and a rendered configuration whose `previews` block holds
its bindings. That block declares `version_metadata`. A Preview's version tag is empty, so a target's smoke compares the
deployment id that `wrangler preview` reports.

Each target has one of two data profiles:
* **`shared`** (release targets and the `staging` alias): production's Hyperdrive config, bucket, rate-limit namespaces
  and queue producer. Its events are consumed by production, as any production request's are. Its collaboration rooms
  are production's, bound with `script_name = "thrust"`. The spike showed that such a room runs production's code and
  environment, so a shared target cannot exercise a change to `RoomDo`, and it never writes a second copy of a page over
  production's (ADR-0061).
* **`isolated`** (`dev-*` targets by default): its own role and schema on `thrust-pg-dev`, with a role-level
  `search_path` of that schema only (the `pr-schema.sh` design that migration `0006` relies on: `pg_trgm` stays in
  `public` and is named by its qualified name). It also has its own Hyperdrive config, its own bucket, the dev
  rate-limit ids, and its own rooms, which Cloudflare creates per Preview and deletes with it. At most 50 live, inside
  the 100-config Hyperdrive limit.

**No Preview runs Cron Triggers or consumes a queue.** Shared targets therefore never sweep production's outbox twice,
and isolated targets deliver no asynchronous events.

### Migrations on shared data

The ledger of ADR-0042's 2026-11-16 update (PR #201) records each file once. The pipeline adds three rules:
* Only the newest shared target migrates. Its files must be a superset of the ledger; a target with fewer files deploys
  without migrating (`Older`); a diverged set is refused.
* While more than one shared target is live, only expand migrations run. A **contract** migration
  (`NNNN_contract_*.sql`) runs only in the production deploy, and only when no older shared target is live; a lint
  refuses a destructive statement in any other new file.
* Release targets are deleted 7 days after a newer release goes live, so a contract migration is never blocked for
  long.

### Consequences

* Good, because production's path is the one it would have had anyway: one in-account service binding (median about
  16 ms in the spike), with no Flagship call and no dependency on the routing.
* Good, because any number of targets exist at once, each with its own code, configuration and, when isolated, data,
  using a built-in (Previews: 500 per Worker, deleted with their Durable Object storage).
* Good, because no target is reachable except through the edge, and the edge forwards only what a rule allows.
* Good, because a rule or a target can be withdrawn by CI in seconds, without a deploy.
* Bad, because a pinned request takes one more hop (median about 58 ms against about 16 in the spike) and counts as two
  billed requests.
* Bad, because `thrust-edge` is a new single point of failure in front of production. It deploys only with a release,
  with its own smoke and rollback.
* Bad, because people on a shared target write production's data with candidate code. A contract migration has to wait
  for older shared targets. A shared target cannot test a `RoomDo` change, and a change to `[[migrations]]` still ships
  only through production's manual gate.
* Bad, because isolated targets deliver no asynchronous events until Cloudflare provisions queues per Preview.
* Neutral: Flagship is used only by the edge for routing. Product flags remain the database table of ADR-0008.

### Confirmation

* The spike (`research/0016`): reachability, the extra hop, Access on Previews only, rooms by `script_name`, Flagship
  from workers-rs, flag propagation, the Access identity, `wrangler preview` from CI, WebSockets, version metadata and
  clean-up.
* In CI, with must-fail controls:
  * a forged pin cookie, client-sent `cf-access-*` or override headers, a denied kind or audience, and `routing` off
    each reach production;
  * a direct call to a Preview URL is refused by Access;
  * a contract migration while an older shared target is live, and a diverged migration set, are refused; an older
    shared target deploys without migrating;
  * an isolated target cannot see production's data;
  * `flagship-sync --check` fails after a dashboard edit.
* Live, at the cut-over:
  * no cookie gives production's commit, and a CI pin gives the target's deployment;
  * the kill switches act within 30 s;
  * a teardown leaves no Preview, flag, Hyperdrive config, schema or bucket.

## Pros and Cons of the Options

### Option 1: one Worker, a candidate at 0 %, version overrides

* Good, because it is entirely built in, with one Worker, one configuration and shared rooms.
* Bad, because a deployment holds at most **two** versions, so it gives `prod` and one candidate, never N. It also
  cannot give a target its own data.

### Option 2: Workers for Platforms

* Good, because a dispatch Worker reaches any number of target Workers through a binding, with no public URL and no
  extra hop.
* Bad, because it is a paid add-on ($25 a month) built for customer code (untrusted mode by default). Whether a target
  Worker could bind production's rooms, and run cron or a queue consumer, was unverified.

### Option 3: Worker Previews behind an edge (chosen)

* Good, because Previews are the built-in for "N isolated, production-like copies of one Worker": per-Preview
  configuration and rooms, deletion with storage, and an Access destination for Previews only.
* Bad, because the edge must fetch each Preview over a public workers.dev URL (custom-domain Preview URLs refuse
  subrequests), so Access and the edge's token are load-bearing. The extra hop costs about 40 ms at the median.

### Option 4: a fixed pool of ordinary Workers

* Good, because every Worker feature works, including `script_name` rooms.
* Bad, because N is capped by the pool, and the slot bookkeeping, reuse and clean-up would all be written by hand.

### Option 5: one hostname per environment

* Good, because it is what exists, with no edge and no extra hop.
* Bad, because it is not what was asked: it gives no N targets, no Flagship choice and no shared hostname.

## Open Questions

* Q-0111.1 — `preview_urls = true` gives Previews their workers.dev URLs. Does it also open production's Version URLs
  past the edge, and does an Access app with destination `worker` (all of the Worker's traffic) close them without
  affecting the service binding? To be measured before production is deployed.
* Q-0111.2 — Does thrust itself work behind the edge in a Preview (the React app, the OIDC callback, CSRF)? The spike
  found that `/api/instances/` builds the app's origin from `Host` (a Preview sees its own workers.dev host), so it must
  prefer `THRUST_PUBLIC_BASE_URL`. To be shown with the first release target.

## More Information

* #195 (tracking), #201 (the migration ledger), #57 (PR previews; the `dev-*` targets answer it).
* `research/0016-one-hostname-spike.md`: the spike, its measurements and what it left untested.
* Cloudflare documentation: Worker Previews (configuration, resources and isolation, custom domains), Version overrides,
  Cloudflare Access for Workers, Flagship (binding, targeting, API tokens, limits), the Terraform provider ≥ 5.20.0
  (`cloudflare_flagship_app`, `cloudflare_flagship_flag`), Custom Domains (Worker-to-Worker communication).
* Related: [[ADR-0110]], [[ADR-0092]], [[ADR-0090]], [[ADR-0008]], [[ADR-0042]], [[ADR-0061]], [[ADR-0006]].

## Update — Flagship as code (2026-11-16)

Built in #195's stage D.
- **The writer.** `scripts/flagship.py` makes every flag from `.github/deploy/flagship/policy.json`, which also holds
  `routing`'s value. Its commands are `validate`, `check`, `sync`, and `put`, `alias` and `delete` for the deploys and
  teardowns. The plan's `flagship-sync.sh` and `flagship-target.sh` are these commands.
- **The live check runs in `flagship.yml`, not on pull requests:** after every `sync` and every night, as
  `check --evaluate`. A pull request has no secrets, and a policy change is meant to differ from what is live, so it gets
  `flagship.py validate` and the tests against a fake instead. `check --evaluate` also asks Flagship what each flag serves
  to every principal kind and audience, so a rule the service reads differently from the policy is caught there.
- **A dashboard edit is reported by the nightly `check`, not overwritten.** `sync` restores the policy when a person runs
  it.
- **The rules.** A target flag's rules are one per policy entry, each with the conditions `kind in [...]` and
  `audience in [...]`, all of which must match.
- **The policy's audiences** must be exactly the edge's `ACCESS_AUD_<AUDIENCE>` variables.
- **Not yet verified against the real service:** `docs/src/testing/status.md` G-43.

## Update — The edge's build and its version (2026-11-17)

The first part of #195's stage E3 (deploying the edge).
- **The edge says which build of it answered.** Every answer it gives carries `x-thrust-edge-version`: the tag of its
  version (the release's commit), else the version id, by thrust's `x-thrust-version` rule. The exception is a passed-through
  WebSocket upgrade. A value an upstream sends under that name is dropped. Its template declares `[version_metadata]`, and
  `scripts/check-edge-config.sh` requires exactly one. Without the header, a deploy of the edge could not be told from the
  version it replaced. `x-thrust-version` stays production's or the target's.
- **The edge is built into the deploy bundle,** under `edge/build/`, beside thrust. `release.yml` therefore builds and
  attests it with the release, and a release candidate's bundle proves it builds. One bundle and one attestation cover both
  Workers, so the edge needs no second trust chain. A pull request's deploy dry run renders the edge's template and lets
  `wrangler deploy --dry-run` check it against that build.
- **Its secrets go up with each version** (`wrangler deploy --secrets-file`), since `secrets.required` makes a deploy that
  lacks one fail. The dry run does not check secrets.

## Update — The edge's deploy (2026-11-17)

The rest of #195's stage E3: `scripts/deploy-edge.sh`, `scripts/edge-controls.sh`, and `deploy.yml`'s `deploy-edge`.
- **Every release deploys the edge,** after production. This deviates from the plan's "when `crates/edge` changed". Every
  release bumps the workspace version and the lock the edge is built from, so a rule on paths could not tell an edge change
  from a release. A rule that missed a dependency's fix would be worse than a redeploy. The edge's tag is therefore always
  the release's commit.
- **The proof** comes in three parts, all on the shared hostname:
  - the smoke through the edge: this edge answers, the request reaches production, production is at the release, and a
    real read works;
  - the smoke's control: an edge nobody deployed must fail;
  - the edge's own controls: a forged pin goes to production, and the opt-in gives no pin to a request Access did not
    admit.

  Any failure rolls back to the edge that was live, and a probe must show that edge answering again.
- **The first deploy is not smoked.** A Worker that did not exist has no hostname, so the first edge deploy cannot reach
  `thrust.sow.foo` until the infrastructure moves it. That deploy stops green, and the next release proves it.
- **A split or unreadable live edge stops the deploy:** nobody plans a gradual rollout of the edge, and there would be no
  one version to roll back to.
- **The job runs in its own environment, `edge`,** while the repository variable `THRUST_EDGE_ENABLED` is `true`. That
  environment holds what the edge needs, and none of production's database route or Flagship's write token. It requires no
  reviewers of its own: the same run's production deploy was already approved.

