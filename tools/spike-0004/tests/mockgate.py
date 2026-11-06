"""A local MODEL of path-scoped Cloudflare Access, used ONLY to prove the spike-0004 harness discriminates.

THIS IS NOT CLOUDFLARE. It encodes what the documentation says about application paths
(developers.cloudflare.com/cloudflare-one/access-controls/policies/app-paths/) plus deliberate defects.
Passing the harness against the good model proves the scripts can recognise a correct gate; failing
against the defective models proves they can recognise a broken one. Neither says anything about how
the real Access behaves: that is what the real run (runbook in research/0006) measures.

Modes
  good     match on the normalised path; strip trailing dot from Host; wildcard `/x/*` covers any depth
  leaky    match on the RAW path but forward the NORMALISED path (differential parsing: `/api/../internal/x`
           is bypass for the gate and `/internal/x` for the Worker)           -> traversal probes must FAIL
  dotleak  good, except a trailing-dot Host matches no app, so the request is ungated -> host probe must FAIL
  shallow  good, except `/x/*` covers exactly ONE segment                      -> deep bypass paths get gated
  open     gates nothing                                                       -> gate-liveness control must VOID
  spaceleak  good, except the public space hostname is wired to the PRODUCT Worker (wrong service on the
           custom domain)                                                      -> space-host probes must FAIL
"""

from __future__ import annotations

import asyncio
import base64
import json
import re
import threading
from dataclasses import dataclass, field

MOCK_HOST = "spike-0004.mock.test"
MOCK_SPACE_HOST = "spike-0004-space.mock.test"
MOCK_TEAM = "https://spike-team.cloudflareaccess.com"
MOCK_CLIENT_ID = "mockclientid.access"
MOCK_CLIENT_SECRET = "mock-client-secret-not-a-secret"

_UNRESERVED = re.compile(r"%(2[de]|3[0-9]|4[1-9a-f]|5[0-9a]|6[1-9a-f]|7[0-9a]|5f|7e)", re.I)


def normalise(path: str) -> str:
    """RFC 3986-ish: decode unreserved percent-escapes, then remove dot segments. `;` is NOT special."""
    path = _UNRESERVED.sub(lambda m: chr(int(m.group(1), 16)), path)
    out: list[str] = []
    segs = path.split("/")
    for i, seg in enumerate(segs):
        if seg == ".":
            if i == len(segs) - 1:
                out.append("")
            continue
        if seg == "..":
            if len(out) > 1:
                out.pop()
            if i == len(segs) - 1:
                out.append("")
            continue
        out.append(seg)
    res = "/".join(out)
    return res if res.startswith("/") else "/" + res


@dataclass
class App:
    name: str
    destinations: list[tuple[str, str]]  # (host, path) ; path "" = whole host
    policy: str                          # "service_auth" | "bypass"


def default_apps(host: str = MOCK_HOST, space_host: str = MOCK_SPACE_HOST) -> list[App]:
    bypass = [(host, p) for s in ("/api", "/auth", "/live") for p in (s, s + "/*")]
    return [
        App("ui", [(host, "")], "service_auth"),
        App("bypass", bypass, "bypass"),
        # overlap experiment: a gated app nested UNDER the bypass app's /api/*
        App("overlap", [(host, "/api/gated"), (host, "/api/gated/*")], "service_auth"),
        # the public space hostname: host-wide bypass, nothing else
        App("space-bypass", [(space_host, "")], "bypass"),
    ]


@dataclass
class MockGate:
    mode: str = "good"
    upstream: tuple[str, int] = ("127.0.0.1", 8787)
    space_upstream: tuple[str, int] | None = None
    apps: list[App] = field(default_factory=default_apps)
    log: list[str] = field(default_factory=list)
    port: int = 0
    _loop: asyncio.AbstractEventLoop | None = None
    _server: asyncio.base_events.Server | None = None
    _thread: threading.Thread | None = None

    # ---- matching -----------------------------------------------------------------------------------
    def _host_key(self, host_header: str) -> str:
        h = host_header.split(":")[0].lower()
        if self.mode != "dotleak":
            h = h.rstrip(".")
        return h

    def _path_matches(self, dest: str, path: str) -> tuple[bool, tuple]:
        path = path.split("?")[0]
        if dest == "":
            return True, (0, 0, 0)
        d = dest.rstrip("/")
        p = path.rstrip("/") or "/"
        if d.endswith("/*"):
            prefix = d[:-2]
            if not p.startswith(prefix + "/") or p == prefix + "/":
                return False, ()
            rest = p[len(prefix) + 1:]
            if self.mode == "shallow" and "/" in rest:
                return False, ()
            return True, (1, len(prefix), 0)
        return (p == d), (1, len(d), 1)

    def decide(self, host_header: str, raw_path: str) -> tuple[str, str, str]:
        """-> (policy, forwarded_path, matched_app). policy in {'service_auth','bypass','none'}"""
        qs = ""
        base = raw_path
        if "?" in raw_path:
            base, qs = raw_path.split("?", 1)
            qs = "?" + qs
        norm = normalise(base)
        match_path = base if self.mode == "leaky" else norm
        host = self._host_key(host_header)
        best, best_key = None, None
        if self.mode != "open":
            for app in self.apps:
                for dh, dp in app.destinations:
                    if dh.lower() != host:
                        continue
                    ok, key = self._path_matches(dp, match_path)
                    if ok and (best_key is None or key > best_key):
                        best, best_key = app, key
        if best is None:
            return "none", norm + qs, "-"
        return best.policy, norm + qs, best.name

    def _upstream_for(self, host_header: str) -> tuple[str, int]:
        if self._host_key(host_header) == MOCK_SPACE_HOST and self.space_upstream and self.mode != "spaceleak":
            return self.space_upstream
        return self.upstream

    # ---- serving ------------------------------------------------------------------------------------
    @staticmethod
    def _fake_jwt() -> str:
        def b(d):
            return base64.urlsafe_b64encode(json.dumps(d).encode()).rstrip(b"=").decode()
        return ".".join([b({"alg": "RS256", "kid": "mock"}),
                         b({"type": "app", "aud": ["mockaud"], "iss": MOCK_TEAM, "common_name": MOCK_CLIENT_ID}), "c2ln"])

    async def _handle(self, reader: asyncio.StreamReader, writer: asyncio.StreamWriter) -> None:
        try:
            head = await reader.readuntil(b"\r\n\r\n")
        except Exception:
            writer.close()
            return
        lines = head.decode("latin-1").split("\r\n")[:-2]
        method, target, ver = lines[0].split(" ", 2)
        hdrs = [tuple(ln.split(":", 1)) for ln in lines[1:] if ":" in ln]
        hdrs = [(k.strip(), v.strip()) for k, v in hdrs]
        get = lambda n: next((v for k, v in hdrs if k.lower() == n), None)  # noqa: E731
        host = get("host") or ""
        policy, fwd_path, app = self.decide(host, target)
        cid, csec = get("cf-access-client-id"), get("cf-access-client-secret")
        self.log.append(f"{method} {target} host={host} -> {policy}/{app} fwd={fwd_path}")

        async def reply(code: int, reason: str, extra: str = "", body: bytes = b"") -> None:
            writer.write(f"HTTP/1.1 {code} {reason}\r\n{extra}Content-Length: {len(body)}\r\nConnection: close\r\n\r\n".encode() + body)
            await writer.drain()
            writer.close()

        inject_jwt = False
        if policy == "service_auth":
            if cid == MOCK_CLIENT_ID and csec == MOCK_CLIENT_SECRET:
                inject_jwt = True
            elif cid or csec:
                await reply(403, "Forbidden", body=b"forbidden")
                return
            else:
                loc = f"{MOCK_TEAM}/cdn-cgi/access/login/{host}?kid=mock&redirect_url={target}"
                await reply(302, "Found", f"Location: {loc}\r\n")
                return

        try:
            ur, uw = await asyncio.open_connection(*self._upstream_for(host))
        except OSError:
            await reply(502, "Bad Gateway", body=b"upstream down")
            return
        is_upgrade = (get("upgrade") or "").lower() == "websocket"
        out_h = []
        for k, v in hdrs:
            if k.lower() == "connection" and not is_upgrade:
                continue
            if k.lower() == "cf-access-jwt-assertion" and inject_jwt:
                continue
            out_h.append((k, v))
        if not is_upgrade:
            out_h.append(("Connection", "close"))
        if inject_jwt:
            out_h.append(("Cf-Access-Jwt-Assertion", self._fake_jwt()))
        req = f"{method} {fwd_path} {ver}\r\n" + "".join(f"{k}: {v}\r\n" for k, v in out_h) + "\r\n"
        uw.write(req.encode("latin-1"))
        await uw.drain()

        async def pipe(src: asyncio.StreamReader, dst: asyncio.StreamWriter) -> None:
            try:
                while True:
                    data = await src.read(65536)
                    if not data:
                        break
                    dst.write(data)
                    await dst.drain()
            except Exception:
                pass
            finally:
                try:
                    dst.close()
                except Exception:
                    pass

        t1 = asyncio.ensure_future(pipe(reader, uw))
        t2 = asyncio.ensure_future(pipe(ur, writer))
        await asyncio.wait({t1, t2}, return_when=asyncio.FIRST_COMPLETED)
        for t in (t1, t2):
            t.cancel()
        for w in (uw, writer):
            try:
                w.close()
            except Exception:
                pass

    # ---- lifecycle ----------------------------------------------------------------------------------
    def start(self) -> int:
        ready = threading.Event()

        def run() -> None:
            self._loop = asyncio.new_event_loop()
            asyncio.set_event_loop(self._loop)
            self._server = self._loop.run_until_complete(asyncio.start_server(self._handle, "127.0.0.1", 0))
            self.port = self._server.sockets[0].getsockname()[1]
            ready.set()
            self._loop.run_forever()

        self._thread = threading.Thread(target=run, daemon=True)
        self._thread.start()
        ready.wait(10)
        return self.port

    def stop(self) -> None:
        loop = self._loop
        if loop and self._server:
            async def shutdown() -> None:
                self._server.close()
                pending = [t for t in asyncio.all_tasks(loop) if t is not asyncio.current_task()]
                for t in pending:
                    t.cancel()
                await asyncio.gather(*pending, return_exceptions=True)
                loop.stop()

            asyncio.run_coroutine_threadsafe(shutdown(), loop)
        if self._thread:
            self._thread.join(timeout=5)
