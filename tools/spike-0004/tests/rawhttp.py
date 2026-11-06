"""Raw HTTP/1.1 + minimal WebSocket client for spike 0004. Standard library only.

Why not requests/fetch/curl: the traversal and encoding probes must put the path on the wire VERBATIM
(`/api/..;/x`, `/api/%2e%2e/x`, `//x`). Every common client normalises dot-segments or re-encodes before
sending, which would silently turn a probe into an ordinary request and make the test vacuous.
This client builds the request line itself, never follows redirects and never decodes anything.
"""

from __future__ import annotations

import base64
import hashlib
import os
import socket
import ssl
import struct
from dataclasses import dataclass, field
from urllib.parse import urlsplit


@dataclass
class Target:
    """Where to connect and what to claim in Host/SNI. Mock/local targets override host_header."""

    scheme: str
    host: str          # TCP connect host
    port: int
    host_header: str   # value of the Host header (and default SNI)
    verify_tls: bool = True

    @staticmethod
    def parse(base: str, host_header: str | None = None) -> "Target":
        u = urlsplit(base)
        scheme = u.scheme or "http"
        port = u.port or (443 if scheme == "https" else 80)
        default_hh = u.hostname if (u.port in (None, 80, 443)) else f"{u.hostname}:{u.port}"
        return Target(scheme, u.hostname, port, host_header or default_hh)

    @property
    def tls(self) -> bool:
        return self.scheme == "https"

    @property
    def hostname_only(self) -> str:
        return self.host_header.split(":")[0]


@dataclass
class Resp:
    status: int
    reason: str
    headers: list[tuple[str, str]] = field(default_factory=list)
    body: bytes = b""
    head: str = ""
    error: str | None = None

    def h(self, name: str) -> str | None:
        n = name.lower()
        for k, v in self.headers:
            if k.lower() == n:
                return v
        return None

    @property
    def text(self) -> str:
        return self.body.decode("utf-8", "replace")

    def json(self):
        import json
        return json.loads(self.body.decode("utf-8"))


def _connect(t: Target, sni: str | None, timeout: float, verify_hostname: bool = True) -> socket.socket:
    sock = socket.create_connection((t.host, t.port), timeout=timeout)
    if t.tls:
        ctx = ssl.create_default_context()
        if not t.verify_tls:
            ctx.check_hostname = False
            ctx.verify_mode = ssl.CERT_NONE
        elif not verify_hostname:
            # chain still verified; only the name match is relaxed (needed for trailing-dot SNI probes)
            ctx.check_hostname = False
        sock = ctx.wrap_socket(sock, server_hostname=sni or t.hostname_only)
    sock.settimeout(timeout)
    return sock


def _dechunk(data: bytes) -> bytes:
    out, i = b"", 0
    while True:
        j = data.find(b"\r\n", i)
        if j < 0:
            break
        size = int(data[i:j].split(b";")[0] or b"0", 16)
        i = j + 2
        if size == 0:
            break
        out += data[i:i + size]
        i += size + 2
    return out


def _parse_head(head: str) -> tuple[int, str, list[tuple[str, str]]]:
    lines = head.split("\r\n")
    parts = lines[0].split(" ", 2)
    status = int(parts[1]) if len(parts) > 1 and parts[1].isdigit() else 0
    reason = parts[2] if len(parts) > 2 else ""
    hdrs = []
    for ln in lines[1:]:
        if ":" in ln:
            k, v = ln.split(":", 1)
            hdrs.append((k.strip(), v.strip()))
    return status, reason, hdrs


def request(
    t: Target,
    raw_target: str,
    *,
    method: str = "GET",
    headers: dict[str, str] | None = None,
    host_header: str | None = None,
    sni: str | None = None,
    verify_hostname: bool = True,
    body: bytes = b"",
    timeout: float = 15.0,
    head_only: bool = False,
) -> Resp:
    """Send one request with `raw_target` exactly as given; never follow redirects; Connection: close."""
    hdrs = {"User-Agent": "spike-0004-harness", "Accept": "*/*", "Connection": "close"}
    for k, v in (headers or {}).items():
        hdrs = {kk: vv for kk, vv in hdrs.items() if kk.lower() != k.lower()}
        hdrs[k] = v
    if body:
        hdrs["Content-Length"] = str(len(body))
    hh = host_header if host_header is not None else t.host_header
    msg = f"{method} {raw_target} HTTP/1.1\r\nHost: {hh}\r\n" + "".join(f"{k}: {v}\r\n" for k, v in hdrs.items()) + "\r\n"
    try:
        sock = _connect(t, sni, timeout, verify_hostname)
    except (OSError, ssl.SSLError) as e:
        return Resp(0, "", error=f"connect/tls: {type(e).__name__}: {e}")
    try:
        sock.sendall(msg.encode("latin-1") + body)
        buf = b""
        while b"\r\n\r\n" not in buf:
            chunk = sock.recv(65536)
            if not chunk:
                break
            buf += chunk
        if b"\r\n\r\n" not in buf:
            return Resp(0, "", error="no complete response head", head=buf.decode("latin-1", "replace"))
        head_b, rest = buf.split(b"\r\n\r\n", 1)
        head = head_b.decode("latin-1")
        status, reason, rh = _parse_head(head)
        if head_only or method == "HEAD" or status in (101, 204, 304):
            return Resp(status, reason, rh, rest, head)
        te = next((v for k, v in rh if k.lower() == "transfer-encoding"), "").lower()
        cl = next((v for k, v in rh if k.lower() == "content-length"), None)
        if cl is not None and "chunked" not in te:
            want = int(cl)
            while len(rest) < want:
                chunk = sock.recv(65536)
                if not chunk:
                    break
                rest += chunk
            rest = rest[:want]
        else:
            chunked = "chunked" in te
            while True:
                # a chunked body ends at the zero-length chunk; do not wait for the peer to close
                # (workerd keeps the connection open even when the request said Connection: close)
                if chunked and (rest.endswith(b"\r\n0\r\n\r\n") or rest == b"0\r\n\r\n"):
                    break
                try:
                    chunk = sock.recv(65536)
                except (socket.timeout, ConnectionResetError):
                    break
                if not chunk:
                    break
                rest += chunk
            if chunked:
                rest = _dechunk(rest)
        return Resp(status, reason, rh, rest, head)
    except (OSError, ssl.SSLError, ValueError) as e:
        return Resp(0, "", error=f"io: {type(e).__name__}: {e}")
    finally:
        try:
            sock.close()
        except OSError:
            pass


# ---------------------------------------------------------------------------------------------------------
# Minimal RFC 6455 client (text/binary/ping/close). Enough to prove upgrade + echo through a gate.
# ---------------------------------------------------------------------------------------------------------
class WSError(Exception):
    pass


class WSClient:
    def __init__(self, sock: socket.socket, leftover: bytes = b""):
        self.sock = sock
        self.buf = leftover

    @staticmethod
    def connect(t: Target, raw_target: str, headers: dict[str, str] | None = None, timeout: float = 15.0):
        """Returns (WSClient|None, Resp). WSClient is None unless the server answered 101."""
        key = base64.b64encode(os.urandom(16)).decode()
        hdrs = {
            "User-Agent": "spike-0004-harness",
            "Upgrade": "websocket",
            "Connection": "Upgrade",
            "Sec-WebSocket-Key": key,
            "Sec-WebSocket-Version": "13",
        }
        hdrs.update(headers or {})
        msg = f"GET {raw_target} HTTP/1.1\r\nHost: {t.host_header}\r\n" + "".join(f"{k}: {v}\r\n" for k, v in hdrs.items()) + "\r\n"
        try:
            sock = _connect(t, None, timeout)
        except (OSError, ssl.SSLError) as e:
            return None, Resp(0, "", error=f"connect/tls: {type(e).__name__}: {e}")
        sock.sendall(msg.encode("latin-1"))
        buf = b""
        while b"\r\n\r\n" not in buf:
            chunk = sock.recv(65536)
            if not chunk:
                break
            buf += chunk
        if b"\r\n\r\n" not in buf:
            sock.close()
            return None, Resp(0, "", error="no response head", head=buf.decode("latin-1", "replace"))
        head_b, rest = buf.split(b"\r\n\r\n", 1)
        head = head_b.decode("latin-1")
        status, reason, rh = _parse_head(head)
        resp = Resp(status, reason, rh, b"", head)
        if status != 101:
            # read any small body so callers can grep it for markers
            try:
                sock.settimeout(3)
                while True:
                    chunk = sock.recv(65536)
                    if not chunk:
                        break
                    rest += chunk
            except (socket.timeout, OSError):
                pass
            te = (resp.h("transfer-encoding") or "").lower()
            resp.body = _dechunk(rest) if "chunked" in te else rest
            sock.close()
            return None, resp
        accept = base64.b64encode(hashlib.sha1((key + "258EAFA5-E914-47DA-95CA-C5AB0DC85B11").encode()).digest()).decode()
        if resp.h("sec-websocket-accept") != accept:
            sock.close()
            resp.error = "bad Sec-WebSocket-Accept"
            return None, resp
        return WSClient(sock, rest), resp

    def _read(self, n: int) -> bytes:
        while len(self.buf) < n:
            chunk = self.sock.recv(max(65536, n - len(self.buf)))
            if not chunk:
                raise WSError("connection closed")
            self.buf += chunk
        out, self.buf = self.buf[:n], self.buf[n:]
        return out

    def send(self, payload: bytes | str, opcode: int | None = None) -> None:
        if isinstance(payload, str):
            data, op = payload.encode(), 0x1
        else:
            data, op = payload, 0x2
        if opcode is not None:
            op = opcode
        head = bytes([0x80 | op])
        n = len(data)
        if n < 126:
            head += bytes([0x80 | n])
        elif n < 65536:
            head += bytes([0x80 | 126]) + struct.pack(">H", n)
        else:
            head += bytes([0x80 | 127]) + struct.pack(">Q", n)
        mask = os.urandom(4)
        masked = bytes(b ^ mask[i % 4] for i, b in enumerate(data))
        self.sock.sendall(head + mask + masked)

    def recv(self) -> tuple[int, bytes]:
        """Next complete data message or close frame as (opcode, payload); pings answered, fragments joined."""
        msg_op, msg = 0, b""
        while True:
            b0, b1 = self._read(2)
            fin, op, ln = b0 & 0x80, b0 & 0x0F, b1 & 0x7F
            if ln == 126:
                ln = struct.unpack(">H", self._read(2))[0]
            elif ln == 127:
                ln = struct.unpack(">Q", self._read(8))[0]
            payload = self._read(ln) if ln else b""
            if op == 0x9:
                self.send(payload, opcode=0xA)
                continue
            if op == 0xA:
                continue
            if op == 0x8:
                return 0x8, payload
            if op in (0x1, 0x2):
                msg_op, msg = op, payload
            else:  # continuation
                msg += payload
            if fin:
                return msg_op, msg

    def close(self) -> None:
        try:
            self.send(struct.pack(">H", 1000), opcode=0x8)
        except OSError:
            pass
        try:
            self.sock.close()
        except OSError:
            pass
