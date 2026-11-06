# /// script
# requires-python = ">=3.11"
# dependencies = ["cryptography"]
# ///
"""Local test of the edge Worker's Access-JWT verifier (ADR-0050 Confirmation: "tampered / foreign-audience
Access JWT rejected").

Self-contained and LOCAL ONLY: generates throwaway RSA keys, serves their JWKS from 127.0.0.1, starts its own
`dev.sh` (wrangler dev --local, no credentials) on :8791 with POLICY_AUD / TEAM_DOMAIN / JWKS_URL pointed at that
JWKS, then sends tokens to GET /internal/whoami.

The positive (a correctly signed token is accepted) is only meaningful because every NEGATIVE control is rejected:
if any forged/tampered/foreign token is accepted, the run prints VOID and exits 3.

What this proves: the verifier code path (RS256 + kid lookup + exp + iss + aud, via WebCrypto) works under workerd.
What it cannot prove: that real Access tokens verify (needs the real team JWKS: runbook step G4), or anything about
the Rust Worker's verifier (spike 0003/0001 must re-run the same vectors against it).

  uv run jwt_verifier_test.py
"""

from __future__ import annotations

import base64
import json
import os
import signal
import subprocess
import sys
import threading
import time
import urllib.request
from http.server import BaseHTTPRequestHandler, HTTPServer
from pathlib import Path

from cryptography.hazmat.primitives import hashes
from cryptography.hazmat.primitives.asymmetric import padding, rsa

HERE = Path(__file__).resolve().parent
DEV = HERE.parent / "dev.sh"
PORT = 8791
TEAM = "https://spike-team.cloudflareaccess.com"
AUD = "spike0004-test-aud"


def b64u(b: bytes) -> str:
    return base64.urlsafe_b64encode(b).rstrip(b"=").decode()


def jwk_of(pub, kid: str) -> dict:
    n = pub.public_numbers()
    return {"kty": "RSA", "alg": "RS256", "use": "sig", "kid": kid,
            "n": b64u(n.n.to_bytes((n.n.bit_length() + 7) // 8, "big")),
            "e": b64u(n.e.to_bytes((n.e.bit_length() + 7) // 8, "big"))}


def sign(key, header: dict, payload: dict) -> str:
    h, p = b64u(json.dumps(header).encode()), b64u(json.dumps(payload).encode())
    sig = key.sign(f"{h}.{p}".encode(), padding.PKCS1v15(), hashes.SHA256())
    return f"{h}.{p}.{b64u(sig)}"


def main() -> int:
    good_key = rsa.generate_private_key(65537, 2048)
    other_key = rsa.generate_private_key(65537, 2048)
    jwks = {"keys": [jwk_of(good_key.public_key(), "kid-good")]}

    class H(BaseHTTPRequestHandler):
        def do_GET(self):  # noqa: N802
            body = json.dumps(jwks).encode()
            self.send_response(200)
            self.send_header("content-type", "application/json")
            self.send_header("content-length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def log_message(self, *a):  # silence
            pass

    srv = HTTPServer(("127.0.0.1", 0), H)
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    jwks_url = f"http://127.0.0.1:{srv.server_port}/jwks"

    env = dict(os.environ, PORT=str(PORT))
    proc = subprocess.Popen(
        [str(DEV), "--var", f"POLICY_AUD:{AUD}", "--var", f"TEAM_DOMAIN:{TEAM}", "--var", f"JWKS_URL:{jwks_url}"],
        env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, start_new_session=True,
    )
    ready = threading.Event()
    tail: list[str] = []

    def pump():
        for line in proc.stdout:  # type: ignore[union-attr]
            tail.append(line)
            if "Ready on" in line:
                ready.set()

    threading.Thread(target=pump, daemon=True).start()
    try:
        if not ready.wait(90):
            print("wrangler dev did not become ready:\n" + "".join(tail[-20:]))
            return 2
        now = int(time.time())
        claims = {"type": "app", "aud": [AUD], "iss": TEAM, "iat": now, "exp": now + 300, "common_name": "test", "sub": ""}
        hdr = {"alg": "RS256", "kid": "kid-good", "typ": "JWT"}
        good = sign(good_key, hdr, claims)
        gh, gp, gs = good.split(".")
        forged_payload = b64u(json.dumps({**claims, "aud": ["someone-else"]}).encode())

        cases = [
            # name, token, expected reason
            ("VALID: correctly signed, right aud/iss/exp", good, "ok"),
            ("tampered payload (aud changed) with the original signature", f"{gh}.{forged_payload}.{gs}", "signature"),
            ("foreign audience, validly signed", sign(good_key, hdr, {**claims, "aud": ["other-app"]}), "aud"),
            ("foreign issuer, validly signed", sign(good_key, hdr, {**claims, "iss": "https://evil.cloudflareaccess.com"}), "iss"),
            ("expired, validly signed", sign(good_key, hdr, {**claims, "exp": now - 10}), "expired"),
            ("alg=none, empty signature", f"{b64u(json.dumps({'alg': 'none', 'kid': 'kid-good'}).encode())}.{gp}.", "alg"),
            ("alg=HS256 (algorithm confusion attempt)", f"{b64u(json.dumps({'alg': 'HS256', 'kid': 'kid-good'}).encode())}.{gp}.{gs}", "alg"),
            ("unknown kid", sign(good_key, {**hdr, "kid": "kid-unknown"}, claims), "kid"),
            ("signed by a different key but claiming kid-good", sign(other_key, hdr, claims), "signature"),
            ("malformed (two parts)", f"{gh}.{gp}", "malformed"),
        ]
        rows, bad_controls, positive_ok = [], [], False
        for name, tok, want in cases:
            req = urllib.request.Request(f"http://127.0.0.1:{PORT}/internal/whoami", headers={"Cf-Access-Jwt-Assertion": tok})
            got = json.load(urllib.request.urlopen(req, timeout=15))["verdict"]
            ok_reason = got.get("reason") == want
            accepted = bool(got.get("valid"))
            rows.append((name, want, got.get("reason"), accepted))
            if want == "ok":
                positive_ok = accepted and ok_reason
            elif accepted or not ok_reason:
                bad_controls.append(f"{name}: expected rejection '{want}', got {got}")
        # no header at all
        got = json.load(urllib.request.urlopen(f"http://127.0.0.1:{PORT}/internal/whoami", timeout=15))["verdict"]
        rows.append(("no Cf-Access-Jwt-Assertion header", "no-jwt", got.get("reason"), bool(got.get("valid"))))
        if got.get("valid") or got.get("reason") != "no-jwt":
            bad_controls.append(f"no header: {got}")

        for name, want, reason, accepted in rows:
            print(f"  {'ACCEPT' if accepted else 'reject':<7} reason={reason!s:<10} (want {want:<9}) {name}")
        if bad_controls:
            print("\n*** CONTROL PASSED: RESULT VOID *** a token that must be rejected was accepted or mis-classified:")
            for b in bad_controls:
                print("   " + b)
            return 3
        if not positive_ok:
            print("\nFAIL: the valid token was not accepted")
            return 1
        print("\nPASS: valid token accepted; all 10 forged/tampered/foreign/malformed controls rejected for the expected reason")
        return 0
    finally:
        try:
            os.killpg(proc.pid, signal.SIGTERM)
        except ProcessLookupError:
            pass
        srv.shutdown()


if __name__ == "__main__":
    sys.exit(main())
