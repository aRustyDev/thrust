"""A small stand-in for a Worker that serves a web build with Workers static assets (issue #49), for test-check-asset-retention.sh.

    assets_stub.py <directory> <mode> <portfile>

Binds 127.0.0.1 on a free port, writes the port to <portfile>, and serves the files of <directory> until it is killed. A path that is not a
file in <directory> is what differs between the modes, and the modes are the behaviours the check has to tell apart:

  serve         a missing path is 404
  fallback      a missing path is `200 text/html` with index.html: what Workers static assets do for the single-page fallback, and what a
                dropped hashed file gets (ADR-0011, Q-0011.1). The behaviour the retention check exists to catch
  wrong-type    every .js is served as text/plain
  wrong-bytes   every .js is served as JavaScript, with other bytes in it
  flaky:<n>     the first <n> requests for a .js get the fallback, then it serves as `serve` does (a deploy that has not reached the edge yet)
"""

import http.server
import os
import sys
import urllib.parse

DIRECTORY, MODE, PORTFILE = sys.argv[1], sys.argv[2], sys.argv[3]
FLAKY = int(MODE.split(":")[1]) if MODE.startswith("flaky:") else 0
state = {"js_requests": 0}


class Handler(http.server.BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass

    def send(self, code, ctype, body):
        self.send_response(code)
        self.send_header("Content-Type", ctype)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def fallback(self):
        with open(os.path.join(DIRECTORY, "index.html"), "rb") as f:
            self.send(200, "text/html; charset=utf-8", f.read())

    def do_GET(self):
        path = urllib.parse.unquote(urllib.parse.urlparse(self.path).path).lstrip("/")
        full = os.path.normpath(os.path.join(DIRECTORY, path))
        is_file = full.startswith(os.path.normpath(DIRECTORY)) and os.path.isfile(full)
        is_js = path.endswith(".js")
        if is_js:
            state["js_requests"] += 1
        if FLAKY and is_js and state["js_requests"] <= FLAKY:
            return self.fallback()
        if not is_file:
            return self.fallback() if MODE == "fallback" else self.send(404, "text/plain", b"not found")
        with open(full, "rb") as f:
            body = f.read()
        if is_js and MODE == "wrong-type":
            return self.send(200, "text/plain", body)
        if is_js and MODE == "wrong-bytes":
            return self.send(200, "application/javascript", b"// not the file that was asked for\n")
        self.send(200, "application/javascript" if is_js else "application/octet-stream", body)


server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
with open(PORTFILE, "w") as f:
    f.write(str(server.server_address[1]))
server.serve_forever()
