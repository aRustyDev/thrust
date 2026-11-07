#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""A tiny reverse proxy that corrupts ONE field of ONE response, for must-fail controls.

    client (schemathesis) --> mutating_proxy --> Plane

Why it exists: a conformance harness that has only ever been seen to pass proves nothing; it
might not be able to fail. This proxy sits between the tool and an otherwise unmodified Plane and
introduces exactly one known defect, so the harness can be required to FAIL, on the right
operation, for the right reason.

Modes (--mode):
  passthrough      forward untouched. Control for the control: proves the proxy itself does not
                   cause failures, so a failure under another mode is attributable to the mutation.
  type             results[0].<field> -> a value of the WRONG TYPE (string -> integer)
  value            results[0].<field> -> a value of the right type but OUTSIDE its enum
  drop             remove results[0].<field> entirely
Only a response that (a) matches --target ("METHOD path-regex"), (b) is HTTP 200, and (c) is JSON
with a non-empty `results` list is touched, and only its FIRST item. Everything else is forwarded
byte-for-byte. The number of responses actually mutated is exposed at /__proxy_stats__ and written
to --stats-file on every change: a control whose count is 0 mutated nothing and is VOID.
"""

import argparse
import http.client
import json
import re
import sys
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import urlsplit

HOP = {"connection", "keep-alive", "proxy-authenticate", "proxy-authorization", "te",
       "trailers", "transfer-encoding", "upgrade", "content-length", "content-encoding"}

STATE = {"mutated": 0, "forwarded": 0, "last_mutation": None}
LOCK = threading.Lock()
ARGS = None


def mutate(body):
    """Return (new_body, description) or (None, None) if the response is not a target."""
    if not isinstance(body, dict) or not isinstance(body.get("results"), list) or not body["results"]:
        return None, None
    item = body["results"][0]
    if not isinstance(item, dict) or ARGS.field not in item:
        return None, None
    before = item[ARGS.field]
    if ARGS.mode == "type":
        item[ARGS.field] = 12345 if not isinstance(before, (int, float)) else "not-a-number"
    elif ARGS.mode == "value":
        item[ARGS.field] = "not-a-valid-enum-member"
    elif ARGS.mode == "drop":
        del item[ARGS.field]
    else:
        return None, None
    return body, f"results[0].{ARGS.field}: {before!r} -> {item.get(ARGS.field, '<removed>')!r}"


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *a):  # quiet
        pass

    def _stats(self):
        data = json.dumps(STATE).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def _proxy(self):
        if self.path == "/__proxy_stats__":
            return self._stats()
        up = urlsplit(ARGS.upstream)
        length = int(self.headers.get("Content-Length") or 0)
        body = self.rfile.read(length) if length else None
        headers = {k: v for k, v in self.headers.items() if k.lower() not in HOP | {"host", "accept-encoding"}}
        headers["Accept-Encoding"] = "identity"  # we must be able to read the body
        conn = http.client.HTTPConnection(up.hostname, up.port, timeout=60)
        try:
            conn.request(self.command, self.path, body=body, headers=headers)
            resp = conn.getresponse()
            data = resp.read()
        finally:
            conn.close()

        out = data
        label = f"{self.command} {urlsplit(self.path).path}"
        if (ARGS.mode != "passthrough" and resp.status == 200 and re.search(ARGS.target, label)
                and "json" in (resp.getheader("Content-Type") or "")):
            try:
                parsed = json.loads(data)
            except ValueError:
                parsed = None
            new, what = mutate(parsed)
            if new is not None:
                out = json.dumps(new).encode()
                with LOCK:
                    STATE["mutated"] += 1
                    STATE["last_mutation"] = {"request": label, "change": what}
                    if ARGS.stats_file:
                        json.dump(STATE, open(ARGS.stats_file, "w"))
        with LOCK:
            STATE["forwarded"] += 1

        self.send_response(resp.status)
        for k, v in resp.getheaders():
            if k.lower() not in HOP:
                self.send_header(k, v)
        self.send_header("Content-Length", str(len(out)))
        self.end_headers()
        if self.command != "HEAD":
            self.wfile.write(out)

    do_GET = do_POST = do_PATCH = do_PUT = do_DELETE = do_HEAD = do_OPTIONS = _proxy


def main():
    global ARGS
    ap = argparse.ArgumentParser()
    ap.add_argument("--listen-port", type=int, required=True)
    ap.add_argument("--upstream", required=True, help="e.g. http://127.0.0.1:18080")
    ap.add_argument("--mode", choices=["passthrough", "type", "value", "drop"], default="passthrough")
    ap.add_argument("--target", default=r"^GET /api/v1/workspaces/[^/]+/projects/[0-9a-f-]{36}/states/$",
                    help="regex over 'METHOD path'")
    ap.add_argument("--field", default="name", help="field of results[0] to corrupt")
    ap.add_argument("--stats-file")
    ARGS = ap.parse_args()
    if ARGS.stats_file:
        json.dump(STATE, open(ARGS.stats_file, "w"))
    srv = ThreadingHTTPServer(("127.0.0.1", ARGS.listen_port), Handler)
    srv.daemon_threads = True
    print(f"mutating proxy :{ARGS.listen_port} -> {ARGS.upstream} mode={ARGS.mode} field={ARGS.field}", flush=True)
    srv.serve_forever()


if __name__ == "__main__":
    sys.exit(main())
