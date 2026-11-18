#!/usr/bin/env python3
"""Tests of service-token.py against a fake Cloudflare (its `request` and `pages` replaced): it makes a token only where none
exists, writes the secret only to a new private file, never prints it, and refuses every file or answer it cannot trust."""
import contextlib
import datetime
import importlib.util
import io
import json
import os
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("service_token", Path(__file__).with_name("service-token.py"))
st = importlib.util.module_from_spec(spec)
spec.loader.exec_module(st)

ACCOUNT = "0123456789abcdef0123456789abcdef"
NOW = datetime.datetime(2026, 11, 18, tzinfo=datetime.timezone.utc)
SECRET = "s3cret-0f1e2d3c4b5a69788796a5b4c3d2e1f0"
TID = "2d8b2f90-3c4d-4e5f-8a1b-2c3d4e5f6071"


class Fake:
    """Cloudflare's service tokens: a dict by id, and every call recorded."""

    def __init__(self, tokens=(), fail_post=False, expires_at="2027-11-18T00:00:00Z", made_id=TID):
        self.tokens = {t["id"]: t for t in tokens}
        self.calls = []
        self.fail_post = fail_post
        self.expires_at = expires_at
        self.made_id = made_id

    def request(self, account, token, method, path, body=None):
        self.calls.append((method, path))
        assert account == ACCOUNT and token == "cf-token"
        if method == "POST" and path == "/access/service_tokens":
            if self.fail_post:
                raise OSError("the API is down")
            made = {"id": self.made_id, "name": body["name"], "client_id": "c1.access", "client_secret": SECRET,
                    "expires_at": self.expires_at, "duration": body["duration"]}
            self.tokens[made["id"]] = made
            return made
        if method == "GET" and path.startswith("/access/service_tokens/"):
            t = self.tokens.get(path.rsplit("/", 1)[1])
            if t is None:
                raise st.Refused("Cloudflare answered without success")
            return {k: v for k, v in t.items() if k != "client_secret"}
        raise AssertionError((method, path))

    def pages(self, account, token, path):
        self.calls.append(("LIST", path))
        assert path == "/access/service_tokens"
        return [{k: v for k, v in t.items() if k != "client_secret"} for t in self.tokens.values()]


class ServiceToken(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.TemporaryDirectory()
        self.addCleanup(self.dir.cleanup)
        self.file = str(Path(self.dir.name) / "creds.json")
        self.env = patch.dict(os.environ, {"CLOUDFLARE_ACCOUNT_ID": ACCOUNT, "CLOUDFLARE_API_TOKEN": "cf-token"})
        self.env.start()
        self.addCleanup(self.env.stop)

    def run_main(self, fake, *argv):
        out, err = io.StringIO(), io.StringIO()
        with patch.object(st, "request", fake.request), patch.object(st, "pages", fake.pages), \
                contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            try:
                rc = st.main(list(argv), now=NOW)
            except SystemExit as e:
                rc = e.code
        self.out, self.err = out.getvalue(), err.getvalue()
        return rc

    def write(self, creds, mode=0o600):
        with open(self.file, "w") as f:
            json.dump(creds, f)
        os.chmod(self.file, mode)

    def good(self, **over):
        c = {"id": TID, "name": "thrust-ci-opt-in", "client_id": "c1.access", "client_secret": SECRET}
        c.update(over)
        return c

    def assert_no_secret_shown(self):
        self.assertNotIn(SECRET, self.out + self.err)

    # ---- ensure, no file -------------------------------------------------------------------------------------------------
    def test_ensure_makes_the_token_and_keeps_its_secret_in_a_new_private_file(self):
        fake = Fake()
        for name in st.NAMES:
            with self.subTest(name=name):
                if os.path.exists(self.file):
                    os.remove(self.file)
                fake = Fake()
                self.assertEqual(self.run_main(fake, "ensure", "--name", name, "--credentials-file", self.file), 0, self.err)
                self.assertIn(("POST", "/access/service_tokens"), fake.calls)
                self.assertEqual(os.stat(self.file).st_mode & 0o777, 0o600)
                saved = json.load(open(self.file))
                self.assertEqual(saved, {"id": TID, "name": name, "client_id": "c1.access", "client_secret": SECRET})
                self.assertIn(f"id={TID}", self.out)
                self.assertIn("client_id=c1.access", self.out)
                self.assert_no_secret_shown()

    def test_ensure_asks_for_the_duration_given(self):
        fake = Fake()
        seen = []
        real = fake.request
        fake.request = lambda a, t, m, p, body=None: (seen.append(body), real(a, t, m, p, body))[1]
        self.assertEqual(self.run_main(fake, "ensure", "--name", "thrust-ci-opt-in", "--credentials-file", self.file,
                                       "--duration", "720h"), 0, self.err)
        self.assertIn({"name": "thrust-ci-opt-in", "duration": "720h"}, seen)

    def test_ensure_refuses_when_a_token_of_that_name_exists_without_a_file(self):
        fake = Fake(tokens=[{"id": TID, "name": "thrust-ci-opt-in", "client_id": "c1.access", "client_secret": SECRET}])
        self.assertEqual(self.run_main(fake, "ensure", "--name", "thrust-ci-opt-in", "--credentials-file", self.file), 1)
        self.assertIn("its secret cannot be read back", self.err)
        self.assertNotIn(("POST", "/access/service_tokens"), fake.calls)
        self.assertFalse(os.path.exists(self.file))

    def test_a_token_that_cannot_be_made_leaves_no_file(self):
        fake = Fake(fail_post=True)
        self.assertEqual(self.run_main(fake, "ensure", "--name", "thrust-ci-opt-in", "--credentials-file", self.file), 1)
        self.assertFalse(os.path.exists(self.file))
        self.assert_no_secret_shown()

    def test_an_answer_whose_id_is_not_an_id_leaves_no_file(self):
        fake = Fake(made_id="not-an-id")
        self.assertEqual(self.run_main(fake, "ensure", "--name", "thrust-ci-opt-in", "--credentials-file", self.file), 1)
        self.assertIn("not a service token id", self.err)
        self.assertFalse(os.path.exists(self.file))

    # ---- ensure or check, with a file -------------------------------------------------------------------------------------
    def test_a_file_that_matches_cloudflare_is_reused_and_nothing_is_made(self):
        for command in ("ensure", "check"):
            with self.subTest(command=command):
                fake = Fake(tokens=[self.good(expires_at="2027-11-18T00:00:00Z")])
                self.write(self.good())
                self.assertEqual(self.run_main(fake, command, "--name", "thrust-ci-opt-in", "--credentials-file", self.file), 0, self.err)
                self.assertNotIn(("POST", "/access/service_tokens"), fake.calls)
                self.assertIn("expires 2027-11-18T00:00:00Z", self.out)
                self.assertNotIn("ROTATE", self.out)
                self.assert_no_secret_shown()

    def test_a_token_expiring_within_30_days_says_rotate(self):
        fake = Fake(tokens=[self.good(expires_at="2026-12-01T00:00:00Z")])
        self.write(self.good())
        self.assertEqual(self.run_main(fake, "check", "--name", "thrust-ci-opt-in", "--credentials-file", self.file), 0, self.err)
        self.assertIn("in 13 days: ROTATE IT SOON", self.out)

    def test_an_expired_token_is_refused(self):
        fake = Fake(tokens=[self.good(expires_at="2026-11-01T00:00:00Z")])
        self.write(self.good())
        self.assertEqual(self.run_main(fake, "check", "--name", "thrust-ci-opt-in", "--credentials-file", self.file), 1)
        self.assertIn("expired", self.err)

    def test_a_file_that_differs_from_cloudflare_is_refused(self):
        cases = {
            "another client id": (self.good(client_id="other.access"), "not the one in the credentials file"),
            "another token's name in the file": (self.good(name="thrust-edge-previews"), "is for 'thrust-edge-previews'"),
            "a token Cloudflare does not have": (self.good(id="3e9c3a01-4d5e-4f60-9b2c-3d4e5f607182"), "without success"),
            "an id that is not one": (self.good(id="../../x"), "not a service token id"),
            "a field missing": ({"id": TID, "name": "thrust-ci-opt-in", "client_id": "c1.access"}, "lacks"),
        }
        for label, (creds, why) in cases.items():
            with self.subTest(label):
                fake = Fake(tokens=[self.good()])
                self.write(creds)
                self.assertEqual(self.run_main(fake, "check", "--name", "thrust-ci-opt-in", "--credentials-file", self.file), 1)
                self.assertIn(why, self.err)
                self.assertNotIn(("POST", "/access/service_tokens"), fake.calls)
                self.assert_no_secret_shown()

    def test_a_file_others_can_read_or_a_link_is_refused(self):
        for mode in (0o640, 0o604, 0o644):
            with self.subTest(mode=oct(mode)):
                self.write(self.good(), mode)
                self.assertEqual(self.run_main(Fake(tokens=[self.good()]), "check", "--name", "thrust-ci-opt-in",
                                               "--credentials-file", self.file), 1)
                self.assertIn("only its owner can read", self.err)
        os.remove(self.file)
        target = str(Path(self.dir.name) / "real.json")
        with open(target, "w") as f:
            json.dump(self.good(), f)
        os.chmod(target, 0o600)
        os.symlink(target, self.file)
        self.assertEqual(self.run_main(Fake(tokens=[self.good()]), "check", "--name", "thrust-ci-opt-in",
                                       "--credentials-file", self.file), 1)
        self.assertIn("plain file", self.err)

    def test_check_without_a_file_makes_nothing(self):
        fake = Fake()
        self.assertEqual(self.run_main(fake, "check", "--name", "thrust-ci-opt-in", "--credentials-file", self.file), 1)
        self.assertIn("no credentials file", self.err)
        self.assertEqual(fake.calls, [])

    # ---- usage ------------------------------------------------------------------------------------------------------------
    def test_usage_errors_call_nothing(self):
        cases = [
            ("ensure", "--name", "thrust-edge", "--credentials-file", self.file),
            ("ensure", "--name", "thrust-ci-opt-in", "--credentials-file", self.file, "--duration", "1y"),
            ("rotate", "--name", "thrust-ci-opt-in", "--credentials-file", self.file),
        ]
        for argv in cases:
            with self.subTest(argv=argv):
                fake = Fake()
                self.assertEqual(self.run_main(fake, *argv), 2)
                self.assertEqual(fake.calls, [])
        for env in ({"CLOUDFLARE_ACCOUNT_ID": "acct"}, {"CLOUDFLARE_API_TOKEN": ""}):
            with self.subTest(env=env), patch.dict(os.environ, env):
                fake = Fake()
                self.assertEqual(self.run_main(fake, "ensure", "--name", "thrust-ci-opt-in", "--credentials-file", self.file), 2)
                self.assertEqual(fake.calls, [])


if __name__ == "__main__":
    unittest.main(verbosity=1)
