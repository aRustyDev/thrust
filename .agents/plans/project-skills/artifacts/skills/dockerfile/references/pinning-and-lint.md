# Pinning, reproducibility, .dockerignore, linting

Behaviour is for Docker 29.8.2 and buildx v0.37.2 (2026-10-18). Tags: `[derived]` = from documentation, not
executed; `[unverified: not run]` = not executed.

## Pin images by tag AND digest

Docker's guidance ("Building best practices > Pin base image versions"): tags are mutable, so the same
`FROM alpine:3.21` can resolve to different images months apart; pin a digest to get the same image every time,
at the price of opting out of automatic security fixes unless something bumps it.

```dockerfile
FROM gcr.io/distroless/cc-debian13:nonroot@sha256:<index digest> AS runtime
```

- **Never copy a digest from `examples/Dockerfile` into another file and never invent one.** The example's
  digests are a dated illustration. The agent cannot resolve one itself: ask the user to run

```sh
docker buildx imagetools inspect gcr.io/distroless/cc-debian13:nonroot --format '{{.Manifest.Digest}}'
```

  and paste the output (registry-only; no daemon needed). Keep a visible placeholder until they do. The value
  is the multi-arch **index** digest: one pin covers amd64 and arm64.
- Keep the human-readable tag next to the digest; update bots need both to propose a bump.
- Pin the Dockerfile frontend too: `# syntax=docker/dockerfile:1@sha256:<digest of docker/dockerfile:1>`
  (resolved the same way). An unpinned `# syntax=docker/dockerfile:1` pulls a moving image at build time.
- Automate bumps: Dependabot with `package-ecosystem: "docker"` opens PRs that update tag and digest together
  (same Docker page).

```yaml
# .github/dependabot.yml (fragment)
- package-ecosystem: "docker"
  directory: "/"
  schedule:
    interval: "weekly"
```

- Write every `FROM` literally. Images hidden behind `ARG` defaults may not be seen by update bots
  `[unverified: not run]`.
- Verify distroless signatures before first adoption (command in `runtime-and-health.md`).
- Pin tools too: `cargo chef` comes from the pinned chef image; do not `cargo install` unpinned tools in a
  build stage; run linters from a digest-pinned image (below).

## Reproducibility - what is and is not claimed

Docker docs ("Reproducible builds with GitHub Actions"): `SOURCE_DATE_EPOCH` makes "the timestamps in the image
index, config, and file metadata reflect the specified Unix time"; set it from the commit:

```sh
SOURCE_DATE_EPOCH="$(git log -1 --pretty=%ct)" docker buildx build ...
```

In a workflow, put it in the build step's `env:`. Levers that matter for this image:

1. Digest-pinned bases (above); without them nothing else holds.
2. `cargo ... --locked` everywhere, so dependency versions come from `Cargo.lock`.
3. No `apt-get` in the runtime stage (distroless), so there is no moving package index.
4. Builder paths: both cargo-chef stages use `/app`; an embedded path stays stable while it does not change.

**Not claimed:** bit-identical image digests across two builds. Verify by building twice and comparing the
digests of an OCI-exported result `[unverified: not run]`. The exporter option that rewrites layer timestamps
(`rewrite-timestamp=true`) is described by BuildKit documentation that is not cited here `[unverified: not run]`.

## .dockerignore

Docker's best-practices page: use a `.dockerignore` to exclude files from the build context. For this
workspace (`examples/dockerignore.example`; copy to `.dockerignore` at the repo root):

| Entry | Why |
|---|---|
| `.git`, `.claude`, `.agents` | history and agent state are not build inputs |
| `target`, `**/target` | a host `target/` is gigabytes, is sent every build, and busts `COPY . .` |
| `crates/thrust-worker/build`, `.wrangler`, `node_modules` | Worker build output (listed in the workspace `.gitignore`) |
| `.env`, `.env.*`, `.dev.vars`, `.dev.vars.*` | may hold secrets; `COPY . .` would bake them into a layer |
| `docs/book` | generated site output |

BuildKit prints the transferred context size at the start of a build; hundreds of MB means an exclusion is
missing. Do **not** ignore `Cargo.lock`, `Cargo.toml`, or `crates/**/migrations` (embedded via `include_str!`).

## Linting: `docker build --check` first, hadolint second

`docker build --check` (Dockerfile frontend 1.8+, buildx 0.15+; "Checking your build configuration") validates
the Dockerfile and options and reports violations **without running the build steps**:

```sh
docker build --check -f Dockerfile .
```

With `--check`, **any violation exits non-zero**; in a regular `docker build` violations are warnings with
exit 0 unless the Dockerfile carries `# check=error=true` (the example does). The check still loads base-image
metadata for each `FROM`, so it needs registry access and a running builder. Rules it reports include
`JSONArgsRecommended` (exec form, for signal handling), `SecretsUsedInArgOrEnv`, `UndefinedVar`,
`FromAsCasing`, `LegacyKeyValueFormat`. `# check=skip=<RuleName>` silences a rule deliberately.
`examples/Dockerfile` itself was not checked `[unverified: not run]`.

hadolint (its README) adds shell-level and style rules. Run it from a **digest-pinned** image or binary, like
every other tool in a pinning guide (resolve the digest as above; do not paste one from this page):

```sh
hadolint Dockerfile                                           # [unverified: not run]
hadolint --failure-threshold warning Dockerfile               # [unverified: not run]
docker run --rm -i hadolint/hadolint@sha256:<digest> < Dockerfile   # [unverified: not run]
```

Rules most relevant to this Dockerfile: `DL3000` use an absolute `WORKDIR`; `DL3002` last `USER` should not be
root; `DL3006` tag the image version explicitly; `DL3007` do not use `latest`. The apt rules (`DL3008`,
`DL3009`, `DL3015`) do not apply because no stage runs `apt-get`. Suppress with `# hadolint ignore=DLxxxx` on
the line above, with a reason. hadolint's config search includes `$PWD/.hadolint.yaml` and files in the home
directory; keep the repo-local one authoritative.

Linting is static: it cannot catch the failures that only appear at run time (`GLIBC_2.xx not found`, ignored
SIGTERM, a missing `migrate`). Those need a build and a run.

## CI shape (generic)

Run `docker build --check` on every PR that touches `Dockerfile*`, `.dockerignore`, `Cargo.lock` or `docker/**`;
build without pushing on PRs, push on tags. Digest-pinned `FROM`s mean the PR that bumps an image is the PR
that proves the new image builds. Workflow design for that job (`docker/build-push-action`, caches, required
checks) belongs to `gha-workflow-design`.
