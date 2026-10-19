# Spike 0004 — Worker topology and the private `.woven` path

**Gates:** ADR-0090, ADR-0091, ADR-0011, ADR-0022.
**Questions:** Q-0090, Q-0090.2, Q-0090.3, Q-0091.1, Q-0030.2, Q-0011.1.
**Timebox:** 3 working days. **Owner:** TBD. Bead `plane-byd.2`.
**Depends on:** none for the public half (round 26): it runs on a throwaway JS Worker and the same checks are re-run against the
Rust Worker once spike 0001 passes. The `.woven` half is a **desk recipe only** in this wave (no change to the shared tunnel,
CoreDNS or external-dns); building it needs an estate decision first, because a Worker is not a K8s Service and the
external-dns annotation route does not apply.

## Question
How is the Cloudflare set laid out (Workers, routes, custom domains), and how does a Worker become reachable on a private
`.woven` name via a tunnel origin to a Worker custom domain (owner direction, round 13)?

## Method
1. Deploy a hello Worker on a public custom domain with **path-scoped Access applications** (UI paths gated; `/api`, `/auth`, `/live` bypass) and a WebSocket echo route; verify the WebSocket upgrade works through Access-gated and bypassed paths and that a bypassed path cannot reach an Access-gated one (round 25).
2. Reach it from a `.woven` name: tunnel origin → Worker custom domain; check Access interaction, WARP path, TLS chain,
   hostnames and ports (probe the actual port; a wrong-port probe looks like an outage).
3. Verify with the real operation from a WARP-enrolled client; assert it **resolves and serves** — never pin an IP.
   Note the woven-o11y rule that new names are created by annotating a Service; Workers need a different mechanism.
4. Topology comparison: one Worker with routes vs Worker-per-surface via service bindings (API, consumers, DO collab,
   `space` public Worker, static assets). Measure latency overhead of service bindings.
5. Do internal RPC and `/api/v1` share one Worker and auth stack (Q-0030.2)?
6. Public `space` hostname bypasses Access narrowly (ADR-0022); confirm it cannot reach authenticated routes.
7. Static assets: serve the React apps (and later Dioxus web build) under prefixes (`/`, `/spaces`) (Q-0011.1).

## Exit criteria
A documented, reproducible recipe (OpenTofu-able) that serves a Worker on a `.woven` name and on a public name; topology
chosen with measured overhead; Access/bypass boundaries tested with must-fail controls.

## Deliverables
ADR-0090/0091 decisions; OpenTofu snippet; Q-0090.2/.3 closed.

## Result (round 26)

**Public half: done.** Docs, local runs and two real runs (the second with the Access gate on) are in `research/0006-topology-public-half.md`; ADR-0050 and ADR-0090 *Update* have the findings (case-insensitive bypass; +2.3 to +2.8 ms per service-binding hop). **`.woven` half:** a desk recipe only (ADR-0090 *Update*), pending four owner decisions. Q-0030.2, Q-0011.1 and Q-0090.2 are answered.
