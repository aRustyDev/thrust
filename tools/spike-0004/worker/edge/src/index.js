// Spike 0004 throwaway edge Worker (JS). Resource prefix: spike-0004-
//
// Surfaces (mirrors ADR-0090 round-25 layout: one hostname, same-origin UI + /api + /auth + /live):
//   /                      static assets (root app shell)               -> GATED by Access in the real run
//   /gated/*               static assets, served by the assets service
//                          WITHOUT invoking this Worker                 -> GATED
//   /spaces/*              second app under a prefix (Q-0011.1)         -> GATED (in the real design `space`
//                          has its own hostname; here it tests prefixes)
//   /internal/*            dynamic Worker routes holding a secret marker -> GATED (traversal target)
//   /api, /api/*           echo JSON incl. which Access headers we saw   -> BYPASS
//   /auth, /auth/*         same                                         -> BYPASS
//   /live, /live/*         WebSocket echo                               -> BYPASS
//   /api/lat/*             latency probes (bypass path, so no Access evaluation in the timing): null,
//                          in-process, service binding (fetch), service binding (RPC)
//   /internal/ws           WebSocket echo on a GATED path (Access + WebSocket upgrade through a gated app)
//
// Nothing here is a secret. Markers are fixed strings the test harness greps for.
// The Worker NEVER echoes cookie / authorization / Cf-Access-Jwt-Assertion VALUES, only presence and,
// for the JWT, non-sensitive claim metadata (iss, aud, type, alg) so the harness can tell which
// Access application authenticated a request.

import { compute } from "../../shared/logic.js";

const MARKERS = {
  dynamic: "GATED-DYNAMIC-MARKER-7f3a9c",
  static: "GATED-STATIC-MARKER-51be0d",
};

const SAFE_HEADERS = [
  "host", "user-agent", "accept", "sec-fetch-mode", "sec-fetch-dest", "sec-fetch-site",
  "upgrade", "connection", "x-forwarded-proto", "cf-visitor", "cf-ray", "cf-ipcountry",
  "content-type", "content-length", "origin",
];

function b64urlToBytes(s) {
  const pad = "=".repeat((4 - (s.length % 4)) % 4);
  const bin = atob(s.replace(/-/g, "+").replace(/_/g, "/") + pad);
  return Uint8Array.from(bin, (c) => c.charCodeAt(0));
}
function b64urlToString(s) {
  return new TextDecoder().decode(b64urlToBytes(s));
}

function jwtMeta(token) {
  try {
    const [h, p] = token.split(".");
    const header = JSON.parse(b64urlToString(h));
    const payload = JSON.parse(b64urlToString(p));
    return {
      jwt_alg: header.alg,
      jwt_kid_present: !!header.kid,
      jwt_iss: payload.iss,
      jwt_aud: payload.aud,
      jwt_type: payload.type,
      jwt_has_email: !!payload.email,
      jwt_common_name_present: !!payload.common_name,
    };
  } catch (e) {
    return { jwt_decode_error: String(e && e.message ? e.message : e) };
  }
}

// What Access did to this request, as seen by the Worker. Presence only, never values.
function accessView(request) {
  const h = request.headers;
  const jwt = h.get("cf-access-jwt-assertion");
  return {
    jwt_present: !!jwt,
    jwt_len: jwt ? jwt.length : 0,
    ...(jwt ? jwtMeta(jwt) : {}),
    client_id_header_present: h.has("cf-access-client-id"),
    client_secret_header_present: h.has("cf-access-client-secret"),
    cf_authorization_cookie_present: /(?:^|;\s*)CF_Authorization=/.test(h.get("cookie") || ""),
    authenticated_user_email_header_present: h.has("cf-access-authenticated-user-email"),
  };
}

function echo(surface, request, status = 200) {
  const url = new URL(request.url);
  const seen = {};
  for (const name of SAFE_HEADERS) {
    const v = request.headers.get(name);
    if (v !== null) seen[name] = v;
  }
  return Response.json(
    {
      worker: "spike-0004-edge",
      surface,
      method: request.method,
      raw_url: request.url,
      pathname: url.pathname,
      search: url.search,
      host: url.host,
      headers_seen: seen,
      header_names: [...request.headers.keys()].sort(),
      access: accessView(request),
      colo: request.cf && request.cf.colo,
    },
    { status, headers: { "cache-control": "no-store" } },
  );
}

// ---- Access JWT verification (ADR-0050 "Worker additionally verifies the Access JWT") ----------------
let jwksCache = { at: 0, url: "", keys: null };
async function getJwks(env) {
  const url = env.JWKS_URL || `${env.TEAM_DOMAIN}/cdn-cgi/access/certs`;
  const now = Date.now();
  if (jwksCache.keys && jwksCache.url === url && now - jwksCache.at < 5 * 60_000) return jwksCache.keys;
  const res = await fetch(url);
  if (!res.ok) throw new Error(`jwks fetch ${res.status}`);
  const body = await res.json();
  jwksCache = { at: now, url, keys: body.keys || [] };
  return jwksCache.keys;
}

async function verifyAccessJwt(token, env) {
  if (!env.POLICY_AUD || !(env.TEAM_DOMAIN || env.JWKS_URL)) {
    return { valid: false, reason: "verifier-not-configured" };
  }
  const parts = token.split(".");
  if (parts.length !== 3) return { valid: false, reason: "malformed" };
  const [h, p, s] = parts;
  let header, payload;
  try {
    header = JSON.parse(b64urlToString(h));
    payload = JSON.parse(b64urlToString(p));
  } catch {
    return { valid: false, reason: "malformed" };
  }
  if (header.alg !== "RS256") return { valid: false, reason: "alg" };
  const keys = await getJwks(env);
  const jwk = keys.find((k) => k.kid === header.kid);
  if (!jwk) return { valid: false, reason: "kid" };
  const key = await crypto.subtle.importKey(
    "jwk", jwk, { name: "RSASSA-PKCS1-v1_5", hash: "SHA-256" }, false, ["verify"],
  );
  const ok = await crypto.subtle.verify(
    "RSASSA-PKCS1-v1_5", key, b64urlToBytes(s), new TextEncoder().encode(`${h}.${p}`),
  );
  if (!ok) return { valid: false, reason: "signature" };
  const now = Math.floor(Date.now() / 1000);
  if (typeof payload.exp === "number" && payload.exp < now) return { valid: false, reason: "expired" };
  if (env.TEAM_DOMAIN && payload.iss !== env.TEAM_DOMAIN) return { valid: false, reason: "iss" };
  const aud = Array.isArray(payload.aud) ? payload.aud : [payload.aud];
  if (!aud.includes(env.POLICY_AUD)) return { valid: false, reason: "aud" };
  return { valid: true, reason: "ok", type: payload.type };
}

// ---- WebSocket echo -----------------------------------------------------------------------------------
function live(request) {
  const upgrade = (request.headers.get("upgrade") || "").toLowerCase();
  if (upgrade !== "websocket") {
    // Distinguishable from an Access redirect/login: 426 + JSON naming this Worker.
    return echo("live-no-upgrade", request, 426);
  }
  const pair = new WebSocketPair();
  const [client, server] = Object.values(pair);
  // MEASURED (wrangler dev, compat date 2026-08-18) and documented (compatibility-flags,
  // websocket_standard_binary_type, default since 2026-03-17): without this line binary frames arrive as Blob,
  // `typeof ev.data !== "string"` is true, and echoing `${ev.data}` silently sent the text "[object Blob]".
  // Yjs sync frames are binary, so spikes 0001/0005 must set binaryType (DO hibernation handlers always get ArrayBuffer).
  server.binaryType = "arraybuffer";
  server.accept();
  server.send(JSON.stringify({ hello: "spike-0004-edge", surface: "live", access: accessView(request) }));
  server.addEventListener("message", (ev) => {
    server.send(typeof ev.data === "string" ? `echo:${ev.data}` : ev.data);
  });
  server.addEventListener("close", (ev) => {
    try { server.close(ev.code && ev.code !== 1005 ? ev.code : 1000, "bye"); } catch { /* already closed */ }
  });
  return new Response(null, { status: 101, webSocket: client });
}

// ---- latency probes -----------------------------------------------------------------------------------
async function viaBindingFetch(env, n) {
  const res = await env.B.fetch(new Request(`https://b.internal/compute?n=${n}`));
  return res.json();
}
async function lat(request, env) {
  const url = new URL(request.url);
  const kind = url.pathname.replace(/^\/api\/lat\//, "");
  const n = url.searchParams.get("n");
  if (kind === "null") return Response.json({ served_by: "edge", via: "null" });
  if (kind === "inproc") return Response.json({ served_by: "edge", via: "inproc", ...compute(n) });
  if (kind === "binding") return Response.json({ served_by: "edge+b", ...(await viaBindingFetch(env, n)) });
  if (kind === "rpc") return Response.json({ served_by: "edge+b", ...(await env.B.compute(n)) });
  if (kind === "loop") {
    // m sequential calls inside ONE request. Date.now() only advances on I/O in production
    // (docs: workers/reference/security-model), so the in-process loop reports ~0 ms there;
    // the binding loops contain real awaits and so get a usable (ms-resolution / m) number.
    const mode = url.searchParams.get("mode") || "binding";
    const m = Math.max(1, Math.min(Number(url.searchParams.get("m")) || 200, 2000));
    const t0 = Date.now();
    for (let i = 0; i < m; i++) {
      if (mode === "inproc") compute(n);
      else if (mode === "rpc") await env.B.compute(n);
      else await viaBindingFetch(env, n);
    }
    const elapsed_ms = Date.now() - t0;
    return Response.json({ mode, m, elapsed_ms, per_call_ms: elapsed_ms / m });
  }
  return new Response("unknown /api/lat/ probe", { status: 404 });
}

// ---- /spaces/* : prefix-aware SPA fallback (Q-0011.1) ----------------------------------------------------
// Workers' built-in SPA fallback serves the ROOT /index.html for any unmatched path (docs), so a deep link
// under /spaces/ would get the WRONG app shell. Intercept /spaces/* (run_worker_first) and map
// extension-less misses to /spaces/index.html ourselves. Hashed files under /spaces/assets/* are excluded
// from run_worker_first (negative pattern) so they stay free/unbilled static-asset hits.
async function spaces(request, env) {
  const url = new URL(request.url);
  const last = url.pathname.split("/").pop() || "";
  const looksLikeFile = last.includes(".");
  if (!looksLikeFile) {
    const shell = new URL("/spaces/", url);
    const res = await env.ASSETS.fetch(new Request(shell, request));
    return new Response(res.body, { status: 200, headers: res.headers });
  }
  return env.ASSETS.fetch(request);
}

export default {
  async fetch(request, env) {
    const url = new URL(request.url);
    const p = url.pathname;

    if (p.startsWith("/api/lat/")) return lat(request, env);
    if (p === "/api" || p.startsWith("/api/")) return echo("api", request);
    if (p === "/auth" || p.startsWith("/auth/")) return echo("auth", request);
    // WebSocket through a service binding: the edge forwards the upgrade request untouched to Worker B
    if (p === "/live/via-b") return env.B.fetch(request);
    if (p === "/live" || p.startsWith("/live/")) return live(request);
    if (p === "/spaces" || p.startsWith("/spaces/")) return spaces(request, env);

    if (p === "/internal/ws") return live(request);
    if (p === "/internal/worker-secret") {
      return Response.json(
        { worker: "spike-0004-edge", surface: "internal", marker: MARKERS.dynamic, access: accessView(request) },
        { headers: { "cache-control": "no-store" } },
      );
    }
    if (p === "/internal/whoami") {
      const token = request.headers.get("cf-access-jwt-assertion");
      const verdict = token ? await verifyAccessJwt(token, env) : { valid: false, reason: "no-jwt" };
      return Response.json(
        { worker: "spike-0004-edge", surface: "internal-whoami", marker: MARKERS.dynamic, verdict, access: accessView(request) },
        { headers: { "cache-control": "no-store" } },
      );
    }
    if (p.startsWith("/internal/")) return echo("internal-other", request, 404);

    // Everything else is a static asset (root shell, SPA fallback, /gated/*).
    return env.ASSETS.fetch(request);
  },
};
