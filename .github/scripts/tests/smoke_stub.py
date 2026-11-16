"""A stand-in for the deployed Worker, for test-smoke-deploy.sh (issue #49). Standard library only.

It answers the five requests scripts/smoke-deploy.sh makes, as the Worker does, and each flag breaks one thing the smoke has to
catch. It binds 127.0.0.1 on a port the system picks and writes that port to --port-file once it is listening. The password the
sign-in accepts comes from the environment (STUB_PASSWORD), not from a flag, so it is in no process's argv.
"""

import argparse
import json
import os
import sys
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

p = argparse.ArgumentParser()
p.add_argument("--port-file", required=True)
p.add_argument("--version", default="abc123", help="what /healthz says in x-thrust-version")
p.add_argument("--no-version", action="store_true", help="send no x-thrust-version header at all")
p.add_argument("--alternate", help="OLD,NEW: answer /healthz with OLD, then NEW, then OLD, and so on (a half-rolled-out deploy)")
p.add_argument("--health-status", type=int, default=200)
p.add_argument("--health-body", default="ok")
p.add_argument("--email", default="smoke@example.test")
p.add_argument("--workspace", default="smoke-ws")
p.add_argument("--project", default="smoke")
p.add_argument("--no-project", action="store_true", help="the workspace lists no project")
p.add_argument("--drop-cookie", action="store_true", help="a sign-in that answers 200 and sets no session cookie")
# The edge (ADR-0111): with --pin TARGET this stands for the shared hostname with one target. A request carrying the pin cookie
# `__Host-thrust_edge=pin-TARGET` is the target's (--version, `x-thrust-route: TARGET`); any other is production's (--prod-version,
# `x-thrust-route: prod`, and its API answers 421, so a smoke that lost the pin cannot read the target's data).
p.add_argument("--pin", help="the one target this edge routes to, by the pin cookie")
p.add_argument("--prod-version", default="prod-version", help="what production says in x-thrust-version")
p.add_argument("--route", help="what a pinned /healthz says in x-thrust-route (default: the --pin target)")
p.add_argument("--no-route", action="store_true", help="send no x-thrust-route at all (a request that did not pass the edge)")
# /_edge/opt-in/<rc|dev>?target=T: Access admits only the client id and secret in STUB_CLIENT_ID / STUB_CLIENT_SECRET (else 403 with
# an empty body, as Access answers); then the edge sets the pin for the --pin target, after refusing the first --deny-first asks
# as it does while the target's flag propagates.
p.add_argument("--deny-first", type=int, default=0)
p.add_argument("--unavailable-first", type=int, default=0, help="answer the first N opt-ins 503, as an edge that cannot reach Flagship")
p.add_argument("--not-admitted", action="store_true", help="the edge refuses the token itself (403 'not admitted: ...')")
p.add_argument("--opt-in-status", type=int, help="answer the opt-in with this status instead")
p.add_argument("--no-pin-cookie", action="store_true", help="answer the opt-in 303 without setting the pin")
a = p.parse_args()

PASSWORD = os.environ.get("STUB_PASSWORD", "s3cret-pw")
CSRF = "csrf-tok-1"
SESSION = "sess-1"
lock = threading.Lock()
health_calls = [0]
opt_in_calls = [0]
CLIENT_ID = os.environ.get("STUB_CLIENT_ID", "ci-id")
CLIENT_SECRET = os.environ.get("STUB_CLIENT_SECRET", "ci-secret")


class Handler(BaseHTTPRequestHandler):
    def log_message(self, fmt, *args):  # one line per request, on stderr, for the test to read
        sys.stderr.write("%s %s\n" % (self.command, self.path))
        sys.stderr.flush()

    def reply(self, status, body, headers=()):
        data = body.encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        for k, v in headers:
            self.send_header(k, v)
        self.end_headers()
        self.wfile.write(data)

    def signed_in(self):
        return ("session=" + SESSION) in (self.headers.get("Cookie") or "")

    def pinned(self):
        return a.pin is None or ("__Host-thrust_edge=pin-" + a.pin) in (self.headers.get("Cookie") or "")

    def opt_in(self, path):
        audience = path[len("/_edge/opt-in/"):]
        query = self.path.split("?", 1)[1] if "?" in self.path else ""
        target = dict(kv.split("=", 1) for kv in query.split("&") if "=" in kv).get("target", "")
        with lock:
            opt_in_calls[0] += 1
            n = opt_in_calls[0]
        if self.headers.get("CF-Access-Client-Id") != CLIENT_ID or self.headers.get("CF-Access-Client-Secret") != CLIENT_SECRET:
            self.send_response(403)
            self.send_header("Content-Length", "0")
            self.end_headers()
        elif audience not in ("rc", "dev"):
            self.reply(404, "no such audience")
        elif n <= a.unavailable_first:
            self.reply(503, "routing is unavailable")
        elif a.opt_in_status:
            self.reply(a.opt_in_status, "routing is unavailable" if a.opt_in_status == 503 else "something else")
        elif a.not_admitted:
            self.reply(403, "not admitted: the token's audience is not this path's")
        elif target != a.pin or n <= a.deny_first:
            self.reply(403, "that target does not exist or is not open to you")
        else:
            headers = [("Location", "/")]
            if not a.no_pin_cookie:
                headers.append(("Set-Cookie", "__Host-thrust_edge=pin-%s; Path=/; Secure; HttpOnly; SameSite=Lax; Max-Age=3600" % target))
            self.reply(303, "", headers)

    def do_GET(self):
        path = self.path.split("?")[0]
        if path.startswith("/_edge/opt-in/"):
            self.opt_in(path)
        elif a.pin is not None and not self.pinned():
            # production, as the edge serves it to a request without a valid pin
            if path == "/healthz":
                self.reply(200, "ok", [("x-thrust-version", a.prod_version), ("x-thrust-route", "prod")])
            else:
                self.reply(421, json.dumps({"error": "routed to production"}))
        elif path == "/healthz":
            with lock:
                health_calls[0] += 1
                n = health_calls[0]
            headers = []
            if a.pin is not None and not a.no_route:
                headers.append(("x-thrust-route", a.route or a.pin))
            if a.alternate:
                old, new = a.alternate.split(",")
                headers.append(("x-thrust-version", old if n % 2 == 1 else new))
            elif not a.no_version:
                headers.append(("x-thrust-version", a.version))
            self.reply(a.health_status, a.health_body, headers)
        elif path == "/auth/get-csrf-token/":
            self.reply(200, json.dumps({"csrf_token": CSRF}), [("Set-Cookie", "csrf=%s; Path=/" % CSRF)])
        elif path == "/api/users/me/":
            if not self.signed_in():
                self.reply(401, json.dumps({"error": "unauthenticated"}))
            else:
                # pretty-printed on purpose: the smoke must not depend on how a server spaces its JSON
                self.reply(200, json.dumps({"id": "u-1", "email": a.email, "is_active": True}, indent=2))
        elif path == "/api/workspaces/%s/projects/" % a.workspace:
            if not self.signed_in():
                self.reply(401, json.dumps({"error": "unauthenticated"}))
            else:
                projects = [] if a.no_project else [{"id": "p-1", "name": a.project, "identifier": "SMK"}]
                self.reply(200, json.dumps(projects, indent=2))
        else:
            self.reply(404, json.dumps({"error": "not found"}))

    def do_POST(self):
        path = self.path.split("?")[0]
        length = int(self.headers.get("Content-Length") or 0)
        raw = self.rfile.read(length)
        if not self.pinned():
            self.reply(421, json.dumps({"error": "routed to production"}))
            return
        if path != "/auth/sign-in/":
            self.reply(404, json.dumps({"error": "not found"}))
            return
        if self.headers.get("X-CSRFToken") != CSRF:
            self.reply(400, json.dumps({"error_code": 5000, "error_message": "CSRF"}))
            return
        try:
            body = json.loads(raw)
        except ValueError:
            body = {}
        if body.get("email") == a.email and body.get("password") == PASSWORD:
            headers = [] if a.drop_cookie else [("Set-Cookie", "session=%s; Path=/; HttpOnly" % SESSION)]
            self.reply(200, json.dumps({"redirect_path": "/"}), headers)
        else:
            self.reply(400, json.dumps({"error_code": 5035, "error_message": "AUTHENTICATION_FAILED_SIGN_IN"}))


srv = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
tmp = a.port_file + ".tmp"
with open(tmp, "w") as f:
    f.write(str(srv.server_address[1]))
os.replace(tmp, a.port_file)
srv.serve_forever()
