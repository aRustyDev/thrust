# Research 0016: one hostname for every target, the live spike (#195, ADR-0111)

Run on 2026-11-16 against the Cloudflare account that holds `sow.foo`, with a short-lived account token, wrangler 4.151.2 (CI's
pin) and workers-rs 0.8.7 (the workspace's). It answers the questions ADR-0111 could not settle from the documentation.
`[measured]` marks what was observed on the live account; anything else is the documentation's word.

## What was built, and that it is gone

- `thrust-spike-target`: a JavaScript stand-in for `thrust`. It has a Durable Object class `Room` that counts and answers
  with the bundle that runs it, static assets, a WebSocket echo, a redirect and a `__Host-` cookie. Its bundles differ by
  one constant, `A` (production), `B` and `C`, so every answer says which code produced it.
- Two Previews of it. `p-shared` (bundle B) binds `Room` with `script_name = "thrust-spike-target"`. `p-iso` (bundle C)
  does not.
- `thrust-spike-edge`: a Rust Worker on the Custom Domain `thrust-spike-edge.sow.foo`. It has a service binding to the
  target, a Flagship binding, and an Access service token in two secrets.
- One Access app on the target's Previews, one on the edge's `/_edge` path, one service token, and a Flagship app with
  three flags. The target also had a Preview-only custom domain, `spike-previews.sow.foo`.

**Everything was deleted the same day** `[measured]`. The listings of Workers, Durable Object namespaces, Access apps,
service tokens, Flagship apps, `sow.foo` DNS records and Workers custom domains name nothing `spike`. The wildcard edge
certificate that a Preview custom domain gets could not be listed with the spike's token (no SSL read).

## Results

| Question | Result `[measured]` |
|---|---|
| Can the edge reach a Preview? | Over its **workers.dev** Preview URL (`<preview>-<worker>.<sub>.workers.dev`), yes. Over its **custom-domain** Preview URL on the same zone, no: Cloudflare answers the subrequest `400` "Workers custom domain preview routing is not supported for subrequests". |
| What does the extra hop cost? | 12 calls each from the edge: the service binding to production 2-54 ms (median about 16); a fetch to a Preview 11-188 ms (median about 58). |
| Can a Preview be reachable only through the edge? | An Access app with `destinations: [{type: "preview_worker", worker_id: <the Worker's tag>}]` and one `non_identity` policy that includes only the edge's service token covers that Worker's Previews only, within 10 s. The direct URL, a made-up token, the edge without its token, and a client's own `cf-access-*` headers sent through the edge (the edge drops them) all get `403`; the edge with its token gets `200`; production through the binding is not affected. |
| Do rooms in a Preview reach production's? | With `script_name` naming the Worker, the Preview's `Room` binding has **production's `namespace_id`**, and the room runs **production's code and environment** (it answered `A`, `ROLE=production`, and its counter continued production's). Without `script_name` the Preview gets its own rooms and runs its own code (`C`, count 1). Cloudflare also creates a per-Preview namespace `<worker>_<preview>_<Class>` either way. |
| Can workers-rs use the Flagship binding? | Only through `#[wasm_bindgen] extern` method imports on an extern type (`#[wasm_bindgen(method, js_name = getStringValue)]`), with the binding taken by `Reflect::get(&env, "FLAGS")` and `unchecked_into`. `dyn_into::<Function>()` fails, because the methods are RPC proxies, and so does `Function::call3` ("The RPC receiver does not implement the method \"call\""). The first evaluation took 110 ms, the next ones 0-6 ms. A rule `kind in [human, ci]` served its JSON variant for `kind=ci`; `kind=stranger`, no context and a missing flag all returned the default. |
| How fast does a flag change arrive? | Disabling a flag (`PUT` of its whole definition) reached the edge in 6 s; enabling it again, in 6 s. The documentation says up to 30 s. |
| What does Access tell the edge? | On an app that covers only `host/_edge`, the Worker gets `Cf-Access-Jwt-Assertion`; other paths are untouched. A service token's claims are `type: app`, `common_name: <client id>.access` and an empty `sub`, with no email. Group names are not in the token. A human login was not tried. |
| Does `wrangler preview` work from CI? | `wrangler preview --name <n> --config <rendered> --json --ignore-base-config [--tag]` deploys a prebuilt bundle with `[assets]`. With `--json`, stdout still begins with the asset-upload progress lines; the JSON starts at the first line that starts with `{`. It reports `preview.urls`, `deployment.id`, `deployment.env` (the bindings) and `deployment.annotations["workers/tag"]`. `wrangler preview delete --name <n> --worker-name <w> --skip-confirmation` removes the Preview and its per-Preview Durable Object namespaces. |
| Do WebSockets pass? | Yes, through the service binding and through the edge's fetch to a Preview. workers-rs keeps the `webSocket` of a fetched `101` (`Response::websocket()` then `Response::from_websocket`). Without the edge's token the upgrade is refused with `403`. |
| Version metadata in a Preview | Absent unless the `previews` block declares `version_metadata`. With it, `.id` is the Preview deployment's id and **`.tag` is empty**, even with `--tag`; the tag shows only as the deployment annotation `workers/tag`. |
| Clean-up order | A Flagship app that a Worker binds cannot be deleted ("referenced by Worker script(s)"). `wrangler delete` deleted the Workers, then failed listing KV namespaces (legacy Workers Sites clean-up) for want of KV read; the API (`DELETE /accounts/<a>/workers/scripts/<name>?force=true`) needs no such permission. |

## What the edge and the pipeline must do because of it

1. Reach Previews over workers.dev only; plan no Preview custom domain.
2. Remove `Set-Cookie: CF_Authorization=…` from every Preview response. Access sets it with no `Domain`, so passed on it
   would be stored for the shared hostname.
3. Fetch Previews with `redirect: manual` and rewrite a `Location` that names the Preview host. By default `fetch()`
   followed the Preview's redirect itself, and the followed URL answered `404`.
4. Call Flagship through extern method imports, as above.
5. Learn a person's audience from the Access path that admitted them, one path per audience. Their groups are not in
   the token. Identify CI by the token's `common_name`.
6. Declare `version_metadata` in the `previews` block. The target smoke compares the deployment id that
   `wrangler preview --json` reports, because a Preview's tag is empty.
7. Delete a target's Worker-side pieces before its Flagship flags, the Flagship app last, and delete Workers through the
   API.

**Also found in thrust itself:** `/api/instances/` builds the app's origin from `Host`
(`crates/thrust-compat-plane/src/instance.rs`, `fn origin`). Through the edge, a Preview sees its own workers.dev host, so
the app would be pointed at a URL that Access refuses. Production, reached by the service binding, sees the shared host.

## Not answered here

- thrust itself behind the edge (the app, the OIDC callback, CSRF). It needs a database, so it is tested with the first
  real release target.
- Whether `preview_urls = true`, which Previews need for their workers.dev URLs, also opens production's Version URLs
  past the edge, and whether an Access app with destination `worker` (rather than `preview_worker`) closes them without
  touching the service binding (Q-0111.1).
- Creating a Hyperdrive config through the API: the spike's token had no Hyperdrive scope.
- Verifying the Access token's signature in workers-rs. It is unit-testable with a fixed key.
