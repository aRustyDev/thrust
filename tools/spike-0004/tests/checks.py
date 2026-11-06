"""Spike 0004 assertions. Every positive assertion is paired with a control that MUST fail.

Result statuses
  PASS   positive held AND every paired control behaved as it must (the attack failed)
  FAIL   positive did not hold (or a leak was found)
  VOID   a control that must fail PASSED -> the positive result is meaningless; printed loudly
  WEAK   no control failed and nothing leaked, but the probes could not be shown to reach the target
  SKIP   cannot run here (needs a gate / credentials / the real platform)
  INFO   an observation that is a finding, not an assertion

`--expect-gate yes` (default): the target is supposed to sit behind Access. If the gate-liveness control
(unauthenticated request to a gated path) is NOT refused, every dependent result is VOID.
`--expect-gate no`: functional run against an ungated target (wrangler dev): gate-dependent checks SKIP.
"""

from __future__ import annotations

import json
import os
import time
from dataclasses import dataclass, field
from typing import Callable

from rawhttp import Resp, Target, WSClient, request

MARK_DYN = "GATED-DYNAMIC-MARKER-7f3a9c"
MARK_STATIC = "GATED-STATIC-MARKER-51be0d"
GATED_MARKERS = [MARK_DYN, MARK_STATIC, "ROOT-SHELL", "SPACES-SHELL"]
NAV = {"Sec-Fetch-Mode": "navigate", "Accept": "text/html"}

PASS, FAIL, VOID, WEAK, SKIP, INFO = "PASS", "FAIL", "VOID", "WEAK", "SKIP", "INFO"


@dataclass
class Result:
    id: str
    claim: str
    status: str
    detail: str = ""
    platform_only: bool = False
    evidence: list = field(default_factory=list)


def classify(r: Resp) -> str:
    """What kind of response is this? Never trusts status alone: Access and the Worker both can send 403/404."""
    if r.status == 0:
        return "error"
    body = r.text
    if any(m in body for m in GATED_MARKERS):
        return "gated-content"
    try:
        j = json.loads(body)
        if isinstance(j, dict) and j.get("worker") == "spike-0004-edge":
            return "worker-internal" if str(j.get("surface", "")).startswith("internal") else "worker-json"
    except ValueError:
        pass
    loc = r.h("location") or ""
    if r.status in (301, 302, 303, 307, 308):
        return "access-redirect" if ("cloudflareaccess.com" in loc or "/cdn-cgi/access" in loc) else "other-redirect"
    if r.status in (401, 403):
        return "access-denied"
    if r.status == 404:
        return "not-found"
    if r.status == 400:
        return "bad-request"
    return f"other-{r.status}"


LEAK_CLASSES = {"gated-content", "worker-internal"}
REFUSED_CLASSES = {"access-redirect", "access-denied"}


class Ctx:
    def __init__(self, t: Target, expect_gate: bool, cid: str | None, csec: str | None,
                 ws_soak: int = 0, workers_dev_host: str | None = None, quick: bool = False, overlap: bool = False,
                 space_target: Target | None = None):
        self.t = t
        self.expect_gate = expect_gate
        self.cid, self.csec = cid, csec
        self.ws_soak = ws_soak
        self.workers_dev_host = workers_dev_host
        self.quick = quick
        self.overlap = overlap
        self.space_target = space_target
        self.results: list[Result] = []
        self.gate_live: bool | None = None

    # -- request helpers ---------------------------------------------------------------------------------
    @property
    def have_creds(self) -> bool:
        return bool(self.cid and self.csec)

    def creds(self, good: bool = True) -> dict[str, str]:
        if not self.expect_gate:
            return {}
        if good:
            return {"CF-Access-Client-Id": self.cid or "", "CF-Access-Client-Secret": self.csec or ""}
        return {"CF-Access-Client-Id": self.cid or "x.access", "CF-Access-Client-Secret": "wrong-" + (self.csec or "x")[:8]}

    def req(self, raw: str, *, auth: bool | str = False, **kw) -> Resp:
        h = dict(kw.pop("headers", {}) or {})
        target = kw.pop("target", None) or self.t
        if auth is True:
            h.update(self.creds(True))
        elif auth == "wrong":
            h.update(self.creds(False))
        return request(target, raw, headers=h, **kw)

    def add(self, id: str, claim: str, status: str, detail: str = "", platform_only: bool = False, evidence=None) -> Result:
        r = Result(id, claim, status, detail, platform_only, evidence or [])
        self.results.append(r)
        return r


def pair(ctx: Ctx, id: str, claim: str,
         positive: Callable[[], tuple[bool, str]],
         controls: list[tuple[str, Callable[[], tuple[bool, str]]]],
         platform_only: bool = False, needs_gate: bool = False, needs_creds: bool = False,
         always: list[tuple[str, Callable[[], tuple[bool, str]]]] | None = None) -> Result:
    """controls: gate-dependent (name, fn) -> (held, detail); skipped on an ungated target.
    always: controls that do not depend on a gate and always run.
    held=False means a control that MUST fail PASSED -> result VOID."""
    if needs_gate and not ctx.expect_gate:
        return ctx.add(id, claim, SKIP, "target is ungated by design (--expect-gate no)", platform_only)
    if needs_creds and ctx.expect_gate and not ctx.have_creds:
        return ctx.add(id, claim, SKIP, "needs SPIKE_ACCESS_CLIENT_ID / SPIKE_ACCESS_CLIENT_SECRET", platform_only)
    pos_ok, pos_detail = positive()
    voided = []
    run_controls = (controls if ctx.expect_gate else []) + (always or [])
    for name, fn in run_controls:
        held, d = fn()
        if not held:
            voided.append(f"{name}: {d}")
    if not ctx.expect_gate and controls:
        pos_detail += "  [ungated functional run: gate controls not applicable]"
    if voided:
        return ctx.add(id, claim, VOID,
                       "*** CONTROL PASSED: RESULT VOID *** " + " | ".join(voided) + f"  (positive was {pos_ok}: {pos_detail})",
                       platform_only)
    return ctx.add(id, claim, PASS if pos_ok else FAIL, pos_detail, platform_only)


# ---------------------------------------------------------------------------------------------------------
# G: the gate itself
# ---------------------------------------------------------------------------------------------------------
def control_refused(ctx: Ctx, raw: str, name: str, auth: bool | str = False, **kw) -> Callable[[], tuple[bool, str]]:
    """A control: this request MUST be refused by the gate. held=True iff refused and no gated content."""
    def run() -> tuple[bool, str]:
        r = ctx.req(raw, auth=auth, **kw)
        c = classify(r)
        return (c in REFUSED_CLASSES), f"{name} -> {r.status} class={c}"
    return run


def check_gate(ctx: Ctx) -> None:
    if not ctx.expect_gate:
        ctx.gate_live = None
        ctx.add("G0", "gate liveness: unauthenticated request to a gated path is refused", SKIP,
                "ungated target (--expect-gate no)")
        return
    r1 = ctx.req("/internal/worker-secret")
    r2 = ctx.req("/gated/asset-secret.txt")
    c1, c2 = classify(r1), classify(r2)
    ctx.gate_live = c1 in REFUSED_CLASSES and c2 in REFUSED_CLASSES
    if ctx.gate_live:
        ctx.add("G0", "gate liveness: unauthenticated request to a gated path is refused", PASS,
                f"dynamic -> {r1.status} {c1}; static -> {r2.status} {c2}", evidence=[r1.h("location")])
    else:
        ctx.add("G0", "gate liveness: unauthenticated request to a gated path is refused", VOID,
                f"*** CONTROL PASSED: RESULT VOID *** dynamic -> {r1.status} {c1}; static -> {r2.status} {c2}. "
                "The gate is absent or not enforced; EVERY bypass/traversal/WebSocket result below is meaningless.")


def gate_live_control(ctx: Ctx) -> tuple[str, Callable[[], tuple[bool, str]]]:
    return "G0 gate liveness", lambda: (bool(ctx.gate_live), "gate live" if ctx.gate_live else "gate NOT live")


def check_gated_paths(ctx: Ctx) -> None:
    def gated_positive(raw: str, want: str, headers=None, id_extra: str = ""):
        def p() -> tuple[bool, str]:
            r = ctx.req(raw, auth=True, headers=headers)
            ok = r.status == 200 and want in r.text
            return ok, f"{raw} with credentials -> {r.status} class={classify(r)} has[{want}]={want in r.text}"
        return p

    cases = [
        ("G1", "gated dynamic Worker route accepts a service token", "/internal/worker-secret", MARK_DYN, None),
        ("G2", "gated static asset (served by the assets service, Worker not invoked) accepts a service token",
         "/gated/asset-secret.txt", MARK_STATIC, None),
        ("G3", "gated root app shell accepts a service token", "/", "ROOT-SHELL", NAV),
    ]
    for id, claim, raw, want, hdr in cases:
        pair(ctx, id, claim, gated_positive(raw, want, hdr),
             [("unauthenticated is refused", control_refused(ctx, raw, f"GET {raw} no credentials", headers=hdr)),
              ("wrong secret is refused", control_refused(ctx, raw, f"GET {raw} wrong secret", auth="wrong", headers=hdr))],
             needs_creds=True)

    # whoami: JWT delivered on a gated path, verified by the Worker (only when POLICY_AUD/TEAM_DOMAIN configured)
    def whoami() -> tuple[bool, str]:
        r = ctx.req("/internal/whoami", auth=True)
        try:
            j = r.json()
        except ValueError:
            return False, f"non-JSON {r.status}"
        acc, verdict = j.get("access", {}), j.get("verdict", {})
        if not acc.get("jwt_present"):
            return False, "gated path delivered NO Cf-Access-Jwt-Assertion to the Worker"
        if verdict.get("reason") == "verifier-not-configured":
            return True, f"JWT present (type={acc.get('jwt_type')}, aud={acc.get('jwt_aud')}); verifier not configured (set TEAM_DOMAIN/POLICY_AUD to verify)"
        return bool(verdict.get("valid")), f"JWT present; verdict={verdict}"
    pair(ctx, "G4", "gated path delivers an Access JWT to the Worker (and it verifies when configured)", whoami,
         [("unauthenticated is refused", control_refused(ctx, "/internal/whoami", "GET /internal/whoami no credentials"))],
         platform_only=True, needs_gate=True, needs_creds=True)


# ---------------------------------------------------------------------------------------------------------
# B: bypass surfaces
# ---------------------------------------------------------------------------------------------------------
BYPASS_CASES = [
    ("/api/ping", "api"), ("/api", "api"), ("/api/a/b/c", "api"), ("/api/v1/work-items/123/comments?x=1", "api"),
    ("/auth/callback", "auth"), ("/auth", "auth"), ("/auth/a/b/c", "auth"),
    ("/live", "live-no-upgrade"), ("/live/echo", "live-no-upgrade"),
    # trailing-slash forms: is `host/api` the same destination as `/api/`?
    ("/api/", "api"), ("/auth/", "auth"), ("/live/", "live-no-upgrade"),
]
LOOKALIKES = ["/apix/ping", "/api-v2/ping", "/authz", "/authx/y", "/liveness", "/apiary/x"]


def check_bypass(ctx: Ctx) -> None:
    for i, (raw, surface) in enumerate(BYPASS_CASES, 1):
        def positive(raw=raw, surface=surface) -> tuple[bool, str]:
            r = ctx.req(raw, headers=NAV if raw.startswith("/auth") else None)
            c = classify(r)
            try:
                j = r.json()
            except ValueError:
                return False, f"{raw} no creds -> {r.status} class={c} (not Worker JSON)"
            ok = (j.get("surface") == surface) and (j.get("access", {}).get("jwt_present") is False)
            return ok, f"{raw} no creds -> {r.status} surface={j.get('surface')} jwt_present={j.get('access', {}).get('jwt_present')} saw_path={j.get('pathname')}"
        controls = [gate_live_control(ctx)]
        stem = "/" + raw.split("/")[1].split("?")[0]
        look = {"/api": "/apix/ping", "/auth": "/authz", "/live": "/liveness"}[stem]
        controls.append(("string-prefix lookalike is still gated", control_refused(ctx, look, f"GET {look} no creds")))
        pair(ctx, f"B{i}", f"bypass path reaches the Worker with no Access credentials and without an Access JWT: {raw}",
             positive, controls, platform_only=False, needs_gate=False)

    # lookalikes alone, each its own assertion (they are the over-matching probe)
    def lookalike_all() -> tuple[bool, str]:
        out, ok = [], True
        for raw in LOOKALIKES:
            c = classify(ctx.req(raw))
            out.append(f"{raw}={c}")
            ok &= c in REFUSED_CLASSES
        return ok, "; ".join(out)
    pair(ctx, "BL1", "bypass destinations do not over-match by string prefix (/apix, /api-v2, /authz, /liveness, /apiary stay gated)",
         lookalike_all, [gate_live_control(ctx)], needs_gate=True)

    # findings (no assertion)
    r = ctx.req("/api/ping", headers={"Cf-Access-Jwt-Assertion": "forged.header.value", "Cf-Access-Authenticated-User-Email": "forged@example.com"})
    try:
        a = r.json().get("access", {})
        forged = a.get("jwt_present")
        ctx.add("BF1", "FINDING: client-supplied Cf-Access-Jwt-Assertion on a bypass path",
                INFO, f"Worker saw jwt_present={forged}, email_header_present={a.get('authenticated_user_email_header_present')}. "
                + ("The edge does NOT strip it: the API Worker must never trust Cf-Access-* headers on /api,/auth,/live; verify or ignore."
                   if forged else "Stripped/overwritten before reaching the Worker."))
    except ValueError:
        ctx.add("BF1", "FINDING: client-supplied Cf-Access-Jwt-Assertion on a bypass path", INFO, f"no JSON ({r.status})")
    if ctx.expect_gate and ctx.have_creds:
        r = ctx.req("/api/ping", auth=True)
        try:
            a = r.json().get("access", {})
            ctx.add("BF2", "FINDING: service-token headers sent to a bypass path", INFO,
                    f"jwt_present={a.get('jwt_present')} client_id_header_present={a.get('client_id_header_present')} "
                    f"client_secret_header_present={a.get('client_secret_header_present')} "
                    f"(True/True = Access does NOT strip service-token credentials on a Bypass app: they reach the origin)")
        except ValueError:
            ctx.add("BF2", "FINDING: service-token headers sent to a bypass path", INFO, f"no JSON ({r.status})")
    if ctx.expect_gate:
        # Is the Access path match for the bypass destinations case-sensitive? The Worker's own routing is case-sensitive, so a
        # case-variant that Access treats as bypass falls through to the static-asset SPA shell, i.e. gated content served unauthenticated.
        rows, shell_open = [], []
        for raw in ("/API/ping", "/Api/ping", "/API", "/AUTH/callback", "/Auth/x", "/LIVE/echo", "/Live"):
            c = classify(ctx.req(raw, headers=NAV))
            rows.append(f"{raw}={c}")
            if c in LEAK_CLASSES:
                shell_open.append(raw)
        ctx.add("BF3", "FINDING: case variants of the BYPASS prefixes, unauthenticated", INFO,
                ("; ".join(rows)) + (
                    f" -> Access treats these as BYPASS (match is case-INSENSITIVE) and the Worker's case-sensitive routing falls through to the gated app shell: {shell_open}"
                    if shell_open else " -> all refused by Access: the destination match is case-sensitive, or the edge lower-cases nothing"))


# ---------------------------------------------------------------------------------------------------------
# T/H/M: traversal, encoding, host and method probes from a bypass path toward gated targets
# ---------------------------------------------------------------------------------------------------------
TARGETS = ["internal/worker-secret", "gated/asset-secret.txt", ""]
FORMS = [
    "/api/../{t}", "/api/..;/{t}", "/api/%2e%2e/{t}", "/api/%2E%2E/{t}", "/api/.%2e/{t}", "/api/%2e./{t}",
    "/api/..%2f{t}", "/api/%2e%2e%2f{t}", "/api/..%5c{t}", "/api/%252e%252e/{t}", "/api/%252e%252e%252f{t}",
    "/api//../{t}", "/api/./../{t}", "/api/x/../../{t}", "/api/..%00/{t}", "/api/%2e%2e;/{t}",
    "/auth/../{t}", "/auth/..;/{t}", "/live/../{t}", "/live/%2e%2e/{t}",
    # case-variant bypass prefix + traversal (added after BF3 showed Access's bypass match is case-insensitive)
    "/API/../{t}", "/Api/%2e%2e/{t}", "/AUTH/..;/{t}", "/LIVE/%2e%2e%2f{t}", "/Api/{t}", "/API//../{t}",
    "//{t}", "/./{t}", "/%2e%2e/{t}", "/api%2f..%2f{t}", "/api%2F..%2F{t}",
    "/{t};/api/x", "/{t}%3b/api", "/{t}/", "/{t}%20", "/{t}.", "/{t}/.", "/{t}%2e", "/{t}?/api/x", "/{t}?x=/api/",
    "/{T}", "/{Ti}",
]


def probe_forms() -> list[str]:
    seen, out = set(), []
    for t in TARGETS:
        for f in FORMS:
            raw = f.replace("{T}", t.upper()).replace("{Ti}", t.title()).replace("{t}", t)
            if raw == "/" or raw in seen:
                continue
            seen.add(raw)
            out.append(raw)
    return out


def probe_pair(ctx: Ctx, raw: str, **kw) -> tuple[str, str, Resp, Resp | None]:
    """Send the probe unauthenticated, and (if possible) with credentials to learn if it is inert."""
    u = ctx.req(raw, **kw)
    a = ctx.req(raw, auth=True, **kw) if ctx.have_creds else None
    return classify(u), (classify(a) if a is not None else "n/a"), u, a


def summarise_probes(ctx: Ctx, id: str, claim: str, probes: list[tuple[str, dict]]) -> None:
    if not ctx.expect_gate:
        ctx.add(id, claim, SKIP, "ungated target")
        return
    leaks, active, inert, rows = [], 0, 0, []
    for raw, kw in probes:
        uc, ac, u, a = probe_pair(ctx, raw, **{k: v for k, v in kw.items() if k != "label"})
        label = kw.get("label", raw)
        rows.append((label, uc, ac, u.status))
        if uc in LEAK_CLASSES:
            leaks.append(f"{label} -> unauth {u.status} {uc}")
        elif ac in LEAK_CLASSES or ac == "worker-internal":
            active += 1
        else:
            inert += 1
    if not ctx.gate_live:
        ctx.add(id, claim, VOID, f"*** CONTROL PASSED: RESULT VOID *** gate-liveness control G0 did not hold; {len(leaks)} 'leaks' seen but meaningless",
                evidence=rows)
    elif leaks:
        ctx.add(id, claim, FAIL, f"LEAK: {len(leaks)} probe(s) returned gated content with NO credentials: " + "; ".join(leaks[:6]), evidence=rows)
    elif not ctx.have_creds:
        ctx.add(id, claim, WEAK, f"{len(probes)} probes, no leak, but without credentials cannot tell whether any probe reaches the target (inert vs refused)", evidence=rows)
    elif active == 0:
        ctx.add(id, claim, WEAK, f"{len(probes)} probes, no leak, but NONE resolved to the gated target even with credentials (all inert): the edge rejects or rewrites them; assertion is weak", evidence=rows)
    else:
        ctx.add(id, claim, PASS, f"{len(probes)} probes, 0 leaks; {active} resolved to the gated target and were refused without credentials (non-vacuous), {inert} inert", evidence=rows)


def check_traversal(ctx: Ctx) -> None:
    forms = probe_forms()
    if ctx.quick:
        forms = forms[:12]
    summarise_probes(ctx, "T1", "path traversal / encoding from a bypass prefix toward gated targets never returns gated content unauthenticated",
                     [(raw, {}) for raw in forms])


def check_host_variants(ctx: Ctx) -> None:
    base = ctx.t.hostname_only
    port = ctx.t.host_header.split(":")[1] if ":" in ctx.t.host_header else None
    suffix = f":{port}" if port else ""
    tgt = "/internal/worker-secret"
    probes = [
        (tgt, {"host_header": f"{base}.{suffix}", "label": "Host: trailing dot"}),
        (tgt, {"host_header": f"{base.upper()}{suffix}", "label": "Host: upper case"}),
        (tgt, {"host_header": f"{base}:443", "label": "Host: explicit :443"}),
        (tgt, {"host_header": f"{base}:8443", "label": "Host: :8443"}),
    ]
    if ctx.t.tls:
        probes.append((tgt, {"host_header": f"{base}.", "sni": f"{base}.", "verify_hostname": False, "label": "Host+SNI: trailing dot"}))
        probes.append((tgt, {"sni": base.upper(), "label": "SNI: upper case"}))
    summarise_probes(ctx, "H1", "Host/SNI variants (trailing dot, case, port) of a gated path do not bypass the gate", probes)


def check_methods(ctx: Ctx) -> None:
    if not ctx.expect_gate:
        ctx.add("M1", "non-GET methods to a gated path are refused", SKIP, "ungated target")
        return
    def positive() -> tuple[bool, str]:
        out, ok = [], True
        for m in ("OPTIONS", "HEAD", "POST", "PUT", "DELETE"):
            r = ctx.req("/internal/worker-secret", method=m, body=b"{}" if m in ("POST", "PUT") else b"",
                        headers={"Origin": "https://evil.example", "Access-Control-Request-Method": "GET"} if m == "OPTIONS" else None)
            c = classify(r)
            out.append(f"{m}={r.status}/{c}")
            ok &= c in REFUSED_CLASSES
        return ok, "; ".join(out)
    pair(ctx, "M1", "OPTIONS/HEAD/POST/PUT/DELETE to a gated path are refused (no CORS-preflight bypass by default)", positive,
         [gate_live_control(ctx)], needs_gate=True)


# ---------------------------------------------------------------------------------------------------------
# W: WebSocket through bypassed and gated paths
# ---------------------------------------------------------------------------------------------------------
def ws_session(ctx: Ctx, raw: str, auth: bool, big: bool = True) -> tuple[bool, str]:
    ws, resp = WSClient.connect(ctx.t, raw, headers=ctx.creds(True) if auth else None)
    if ws is None:
        return False, f"no upgrade: {resp.status} class={classify(resp)} {resp.error or ''}"
    try:
        op, hello = ws.recv()
        h = json.loads(hello.decode())
        ws.send("ping-1")
        op1, p1 = ws.recv()
        ws.send(bytes(range(256)))
        op2, p2 = ws.recv()
        n_ok = 0
        for i in range(50):
            ws.send(f"m{i}")
            _, e = ws.recv()
            n_ok += e == f"echo:m{i}".encode()
        bigok = True
        if big:
            blob = os.urandom(1024 * 1024)
            ws.send(blob)
            _, e = ws.recv()
            bigok = e == blob
            txt = "x" * 65536
            ws.send(txt)
            _, e = ws.recv()
            bigok &= e == f"echo:{txt}".encode()
        ok = (op1 == 1 and p1 == b"echo:ping-1" and op2 == 2 and p2 == bytes(range(256)) and n_ok == 50 and bigok)
        return ok, (f"101, hello.jwt_present={h.get('access', {}).get('jwt_present')}, text echo={p1 == b'echo:ping-1'}, "
                    f"binary echo={p2 == bytes(range(256))}, 50 sequential={n_ok}/50, 64KiB+1MiB={bigok}")
    except Exception as e:  # noqa: BLE001
        return False, f"session error {type(e).__name__}: {e}"
    finally:
        ws.close()


def check_websocket(ctx: Ctx) -> None:
    def ws_refused(raw: str, auth: bool | str = False):
        def run() -> tuple[bool, str]:
            ws, resp = WSClient.connect(ctx.t, raw, headers=ctx.creds(auth is True) if auth else (ctx.creds(False) if auth == "wrong" else None))
            if ws is not None:
                ws.close()
                return False, f"UPGRADED (101) on {raw}"
            c = classify(resp)
            return (c in REFUSED_CLASSES), f"upgrade on {raw} -> {resp.status} class={c}"
        return run

    def pos_bypass() -> tuple[bool, str]:
        ok, d = ws_session(ctx, "/live/echo", auth=False)
        return ok, d
    # Note: on a bypass path the hello must report jwt_present False; ws_session returns raw detail, check here
    def pos_bypass_strict() -> tuple[bool, str]:
        ok, d = pos_bypass()
        return ok and "jwt_present=False" in d, d
    pair(ctx, "W1", "WebSocket upgrade + text/binary/64KiB/1MiB echo works on a BYPASS path (/live/echo) with no credentials",
         pos_bypass_strict,
         [gate_live_control(ctx), ("WS upgrade on a gated path without credentials is refused", ws_refused("/internal/ws"))])

    pair(ctx, "W2", "WebSocket upgrade + echo works through a GATED path (/internal/ws) with a service token",
         lambda: ws_session(ctx, "/internal/ws", auth=True),
         [("WS upgrade on gated path, no credentials, is refused", ws_refused("/internal/ws")),
          ("WS upgrade on gated path, wrong secret, is refused", ws_refused("/internal/ws", "wrong"))],
         platform_only=True, needs_creds=True)

    # W4: the upgrade survives a service-binding hop (edge -> Worker B), the shape of edge -> collab Worker.
    def via_binding() -> tuple[bool, str]:
        ws, resp = WSClient.connect(ctx.t, "/live/via-b")
        if ws is None:
            return False, f"no upgrade through the binding: {resp.status} class={classify(resp)} {resp.error or ''}"
        try:
            _, hello = ws.recv()
            ws.send("through-binding")
            _, e1 = ws.recv()
            blob = os.urandom(256 * 1024)
            ws.send(blob)
            _, e2 = ws.recv()
            ok = b"spike-0004-b" in hello and e1 == b"b-echo:through-binding" and e2 == blob
            return ok, f"101 via binding; hello from B={b'spike-0004-b' in hello}; text echo prefixed by B={e1 == b'b-echo:through-binding'}; 256KiB binary intact={e2 == blob}"
        except Exception as e:  # noqa: BLE001
            return False, f"session error {type(e).__name__}: {e}"
        finally:
            ws.close()

    def edge_echo_is_distinguishable() -> tuple[bool, str]:
        # control: the edge Worker's own /live/echo answers with the "echo:" prefix, so "b-echo:" can only come from B
        ws, resp = WSClient.connect(ctx.t, "/live/echo")
        if ws is None:
            return False, f"no upgrade {resp.status}"
        try:
            ws.recv()
            ws.send("x")
            _, e = ws.recv()
            return e == b"echo:x", f"edge echo reply={e!r}"
        finally:
            ws.close()
    pair(ctx, "W4", "WebSocket upgrade passes through a SERVICE BINDING (edge Worker -> Worker B) and Worker B answers",
         via_binding, [gate_live_control(ctx)], always=[("edge's own echo is distinguishable from B's", edge_echo_is_distinguishable)])

    if ctx.ws_soak > 0:
        def soak() -> tuple[bool, str]:
            ws, resp = WSClient.connect(ctx.t, "/live/echo")
            if ws is None:
                return False, f"no upgrade {resp.status}"
            t0, sent = time.time(), 0
            try:
                ws.recv()
                while time.time() - t0 < ctx.ws_soak:
                    time.sleep(min(20, ctx.ws_soak))
                    ws.send(f"soak{sent}")
                    _, e = ws.recv()
                    if e != f"echo:soak{sent}".encode():
                        return False, f"bad echo at {time.time() - t0:.0f}s"
                    sent += 1
                return True, f"survived {time.time() - t0:.0f}s with {sent} messages"
            except Exception as e:  # noqa: BLE001
                return False, f"dropped after {time.time() - t0:.0f}s: {type(e).__name__}: {e}"
            finally:
                ws.close()
        pair(ctx, "W3", f"idle-ish WebSocket on a bypass path survives {ctx.ws_soak}s (message every 20s)", soak,
             [gate_live_control(ctx)], platform_only=True)
    else:
        ctx.add("W3", "WebSocket soak (--ws-soak N seconds)", SKIP, "not requested", platform_only=True)


# ---------------------------------------------------------------------------------------------------------
# S: static assets, prefixes, SPA fallback, run_worker_first (Q-0011.1)
# ---------------------------------------------------------------------------------------------------------
def body_has(ctx: Ctx, raw: str, want: str, avoid: str | None = None, headers=None) -> tuple[bool, str]:
    r = ctx.req(raw, auth=True, headers=headers)
    ok = r.status == 200 and want in r.text and (avoid is None or avoid not in r.text)
    return ok, f"{raw} -> {r.status} has[{want}]={want in r.text}" + (f" has[{avoid}]={avoid in r.text}" if avoid else "")


def check_assets(ctx: Ctx) -> None:
    ctl_unauth = lambda raw, hdr=None: ("unauthenticated is refused", control_refused(ctx, raw, f"GET {raw} no creds", headers=hdr))  # noqa: E731
    pair(ctx, "S1", "root app: `/` and a deep link serve the ROOT shell (SPA fallback)",
         lambda: _all([body_has(ctx, "/", "ROOT-SHELL", headers=NAV), body_has(ctx, "/some/deep/link", "ROOT-SHELL", headers=NAV)]),
         [ctl_unauth("/some/deep/link", NAV)], needs_creds=True)
    pair(ctx, "S2", "prefixed app: `/spaces/`, `/spaces` and a DEEP LINK under it serve the SPACES shell, not the root shell",
         lambda: _all([body_has(ctx, "/spaces/", "SPACES-SHELL", "ROOT-SHELL", NAV),
                       body_has(ctx, "/spaces", "SPACES-SHELL", "ROOT-SHELL", NAV),
                       body_has(ctx, "/spaces/deep/link/x", "SPACES-SHELL", "ROOT-SHELL", NAV)]),
         [ctl_unauth("/spaces/deep/link/x", NAV)], needs_creds=True)
    pair(ctx, "S3", "hashed asset under the prefix is served as the asset (not an app shell)",
         lambda: body_has(ctx, "/spaces/assets/app.abc123.js", "SPACES-APP-JS", "SPACES-SHELL"),
         [ctl_unauth("/spaces/assets/app.abc123.js")], needs_creds=True)

    # documented caveat: the built-in SPA fallback is ONE per Worker and serves the ROOT index.html
    def fallback_is_root() -> tuple[bool, str]:
        return body_has(ctx, "/spaces-not-the-prefix/deep", "ROOT-SHELL", "SPACES-SHELL", NAV)
    pair(ctx, "S4", "control for S2: a path OUTSIDE the prefix gets the ROOT shell (shells are distinguishable, S2 is not vacuous)",
         fallback_is_root, [ctl_unauth("/spaces-not-the-prefix/deep", NAV)], needs_creds=True)

    # navigation requests must still reach the Worker for /auth/* and /api (run_worker_first)
    def nav_reaches_worker() -> tuple[bool, str]:
        out, ok = [], True
        for raw, surface in (("/auth/callback", "auth"), ("/auth", "auth"), ("/api", "api"), ("/api/x", "api")):
            r = ctx.req(raw, headers=NAV)
            try:
                s = r.json().get("surface")
            except ValueError:
                s = f"HTML? {r.status}"
            out.append(f"{raw}->{s}")
            ok &= s == surface
        return ok, "; ".join(out)
    def nav_outside_is_shell() -> tuple[bool, str]:
        # control: a navigation to a path NOT listed in run_worker_first is answered with the SPA shell.
        # held=True when that is what happens (proves S5's positive is not vacuous).
        if ctx.expect_gate and not ctx.have_creds:
            return True, "no credentials: control not evaluated"
        r = ctx.req("/authx/callback", auth=True, headers=NAV)
        is_shell = "ROOT-SHELL" in r.text
        return is_shell, f"/authx/callback (navigation, outside run_worker_first) -> {r.status} ROOT-SHELL={is_shell}"
    pair(ctx, "S5", "browser NAVIGATION to /auth/callback, /auth, /api reaches the Worker (run_worker_first), not index.html",
         nav_reaches_worker, [gate_live_control(ctx)],
         always=[("navigation outside run_worker_first gets the SPA shell (so S5 is not vacuous)", nav_outside_is_shell)])


def _all(parts: list[tuple[bool, str]]) -> tuple[bool, str]:
    return all(p[0] for p in parts), "; ".join(p[1] for p in parts)


# ---------------------------------------------------------------------------------------------------------
# X: service binding behaviour and workers.dev exposure
# ---------------------------------------------------------------------------------------------------------
def check_binding(ctx: Ctx) -> None:
    def get(kind: str, n: int) -> dict:
        r = ctx.req(f"/api/lat/{kind}?n={n}")
        try:
            return r.json()
        except ValueError:
            return {"_status": r.status}
    def same_logic() -> tuple[bool, str]:
        a, b, c = get("inproc", 1000), get("binding", 1000), get("rpc", 1000)
        ok = a.get("hash") == b.get("hash") == c.get("hash") and a.get("hash") is not None
        return ok, f"hash inproc={a.get('hash')} binding={b.get('hash')} rpc={c.get('hash')}; served_by {a.get('served_by')}/{b.get('served_by')}/{c.get('served_by')}"
    def different_input_differs() -> tuple[bool, str]:
        a, b = get("binding", 1000), get("binding", 1001)
        if a.get("hash") is None or b.get("hash") is None:
            return True, f"not evaluable (probe path unreachable: {a.get('_status')}/{b.get('_status')}); positive already fails"
        return a.get("hash") != b.get("hash"), f"binding n=1000 hash={a.get('hash')} n=1001 hash={b.get('hash')}"
    def x1_positive() -> tuple[bool, str]:
        ok, d = same_logic()
        ok = ok and get("binding", 1).get("worker") == "spike-0004-b" and get("rpc", 1).get("via") == "rpc"
        return ok, d
    pair(ctx, "X1", "service binding (fetch) and binding RPC execute the same logic as in-process (identical result) in a second Worker",
         x1_positive, [],
         always=[("different input yields a different hash (the equality check can fail)", different_input_differs)])

    if ctx.workers_dev_host:
        def wd() -> tuple[bool, str]:
            wt = Target.parse(f"https://{ctx.workers_dev_host}")
            r = request(wt, "/internal/worker-secret")
            c = classify(r)
            return (c not in LEAK_CLASSES), f"workers.dev {ctx.workers_dev_host} /internal/worker-secret -> {r.status} class={c} {r.error or ''}"
        def custom_domain_serves() -> tuple[bool, str]:
            if not ctx.have_creds:
                return True, "no credentials: not evaluated"
            r = ctx.req("/internal/worker-secret", auth=True)
            return MARK_DYN in r.text, f"custom domain with credentials -> {r.status} has marker={MARK_DYN in r.text}"
        pair(ctx, "X2", "workers.dev hostname of the edge Worker does NOT serve the gated route (workers_dev=false / preview_urls=false)",
             wd, [gate_live_control(ctx)], platform_only=True,
             always=[("the custom domain with credentials DOES serve the route (the target exists)", custom_domain_serves)])
    else:
        ctx.add("X2", "workers.dev does not serve the gated route", SKIP,
                "set --workers-dev-host spike-0004-edge.<account-subdomain>.workers.dev (platform only)", platform_only=True)


def check_overlap(ctx: Ctx) -> None:
    """Overlapping Access apps: a GATED app at /api/gated/* nested under the bypass app's /api/*."""
    if not ctx.overlap:
        ctx.add("O1", "overlapping apps: gated /api/gated/* nested under bypass /api/* (--overlap, needs enable_overlap_experiment)", SKIP,
                "not requested", platform_only=True)
        return
    nested = ["/api/gated", "/api/gated/x", "/api/gated/x/y/z"]

    def positive() -> tuple[bool, str]:
        out, ok = [], True
        for raw in nested:
            u = ctx.req(raw)
            a = ctx.req(raw, auth=True)
            cu, ca = classify(u), classify(a)
            try:
                reached = a.json().get("surface") == "api"
            except ValueError:
                reached = False
            out.append(f"{raw}: unauth={u.status}/{cu} with-token={a.status}/{ca} reaches-worker={reached}")
            ok &= (cu in REFUSED_CLASSES) and reached
        return ok, "; ".join(out)

    def sibling_still_bypass() -> tuple[bool, str]:
        r = ctx.req("/api/ping")
        c = classify(r)
        return (c == "worker-json"), f"GET /api/ping no creds -> {r.status}/{c} (bypass must still work, else the overlap result proves nothing)"

    pair(ctx, "O1", "overlap: the MORE SPECIFIC gated app at /api/gated/* wins over the bypass app at /api/* (and nothing is inherited)",
         positive,
         [gate_live_control(ctx),
          ("sibling /api/ping is still bypassed (control: not everything under /api is gated)", sibling_still_bypass)],
         platform_only=True, needs_gate=True, needs_creds=True)


def check_space(ctx: Ctx) -> None:
    """Spike step 6 / ADR-0022: the public `space` hostname bypasses Access narrowly and cannot reach authenticated routes."""
    st = ctx.space_target
    if st is None:
        ctx.add("SP1", "public space hostname: host-wide bypass, no path to the product hostname's gated routes", SKIP,
                "needs --space-host <spike-0004-space.zone> (tofu creates it)", platform_only=True)
        ctx.add("SP2", "SNI/Host confusion between the bypass hostname and the gated hostname does not expose gated content", SKIP,
                "needs --space-host", platform_only=True)
        return
    product_host = ctx.t.host_header
    space_host = st.host_header

    def space_open() -> tuple[bool, str]:
        out, ok = [], True
        for raw in ("/", "/boards/abc/x"):
            r = request(st, raw)
            try:
                j = r.json()
            except ValueError:
                j = {}
            good = r.status == 200 and j.get("worker") == "spike-0004-space" and j.get("access", {}).get("jwt_present") is False
            out.append(f"{raw} no creds -> {r.status} worker={j.get('worker')}")
            ok &= good
        return ok, "; ".join(out)

    def space_has_no_gated_routes() -> tuple[bool, str]:
        out, held = [], True
        for raw in ("/internal/worker-secret", "/gated/asset-secret.txt", "/internal/ws"):
            c = classify(request(st, raw))
            out.append(f"{raw}={c}")
            held &= c not in LEAK_CLASSES
        return held, "space host serves no gated content: " + "; ".join(out)

    def sp1_positive() -> tuple[bool, str]:
        open_ok, d1 = space_open()
        none_gated, d2 = space_has_no_gated_routes()
        return open_ok and none_gated, f"{d1} | {d2}"
    # A leak is the thing under test (FAIL), not a validity control; the control is that the product gate is live at all.
    pair(ctx, "SP1", "public space hostname answers with no credentials (host-wide bypass) and serves NONE of the product host's gated content",
         sp1_positive, [gate_live_control(ctx)], platform_only=True)

    # SNI/Host confusion: TLS says one hostname, the Host header says the other. If Access keys on one and
    # the Worker routing on the other, a bypass on the space hostname could expose the gated hostname.
    t_prod, t_space = ctx.t, st
    probes = []
    for raw in ("/internal/worker-secret", "/gated/asset-secret.txt", "/"):
        probes.append((raw, {"target": t_space, "host_header": product_host, "label": f"connect to space, Host: product  {raw}"}))
        if t_prod.tls:
            probes.append((raw, {"target": t_prod, "host_header": product_host, "sni": space_host.split(":")[0],
                                 "verify_hostname": False, "label": f"connect to product, SNI: space, Host: product  {raw}"}))
            probes.append((raw, {"target": t_space, "host_header": product_host, "sni": product_host.split(":")[0],
                                 "verify_hostname": False, "label": f"connect to space, SNI: product, Host: product  {raw}"}))
            probes.append((raw, {"target": t_prod, "host_header": space_host, "sni": product_host.split(":")[0],
                                 "verify_hostname": False, "label": f"connect to product, SNI: product, Host: space  {raw}"}))
        else:
            probes.append((raw, {"target": t_space, "host_header": f"{product_host}.", "label": f"connect to space, Host: product-dot  {raw}"}))
    summarise_probes(ctx, "SP2", "SNI/Host confusion between the bypass hostname and the gated hostname does not expose gated content", probes)


def run_all(ctx: Ctx) -> list[Result]:
    check_gate(ctx)
    check_gated_paths(ctx)
    check_bypass(ctx)
    check_traversal(ctx)
    check_host_variants(ctx)
    check_methods(ctx)
    check_websocket(ctx)
    check_assets(ctx)
    check_binding(ctx)
    check_overlap(ctx)
    check_space(ctx)
    return ctx.results
