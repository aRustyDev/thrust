#!/usr/bin/env python3
import importlib.util
import io
import os
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("preview_access", Path(__file__).with_name("preview-access.py"))
access = importlib.util.module_from_spec(spec)
spec.loader.exec_module(access)


class AccessSetup(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.file = str(Path(self.temp.name) / "credential.json")
        self.apps = []
        self.policies = []
        self.calls = []

    def request(self, account, token, method, path, body=None):
        self.calls.append((method, path))
        if path.startswith('/workers/workers/'):
            result = {'id': 'worker', 'name': access.NAME, 'subdomain': {'url': 'https://thrust-preview-provisioner.test.workers.dev'}}
        elif path == '/access/service_tokens' and method == 'POST':
            result = {'id': 'token', 'client_id': 'client', 'client_secret': 'secret'}
        elif path == '/access/service_tokens/token':
            result = {'id': 'token', 'client_id': 'client', 'enabled': True}
        elif path == '/access/apps' and method == 'POST':
            result = {'id': 'app', 'name': access.NAME, 'destinations': body['destinations']}
            self.apps.append(result)
            self.policies = body['policies']
        elif path == '/workers/scripts/thrust-preview-provisioner/subdomain' and method == 'POST':
            result = body
        else:
            raise AssertionError((method, path))
        return {'success': True, 'result': result}

    def pages(self, account, token, path):
        if path == '/access/apps':
            return self.apps
        if path == '/access/apps/app/policies':
            return self.policies
        raise AssertionError(path)

    def run_apply(self, check=False):
        with patch.object(access, 'request', self.request), patch.object(access, 'pages', self.pages):
            access.apply('account', 'api-token', 'worker', self.file, check)

    def test_create_then_repeat_without_writes(self):
        self.run_apply()
        self.assertEqual(os.stat(self.file).st_mode & 0o777, 0o600)
        self.calls.clear()
        self.run_apply(True)
        self.assertFalse(any(method == 'POST' for method, _ in self.calls))

    def test_extra_policy_fails_closed(self):
        self.run_apply()
        self.policies.append({'decision': 'bypass', 'include': [{'everyone': {}}]})
        with self.assertRaises(ValueError):
            self.run_apply(True)

    def test_check_only_never_creates(self):
        with self.assertRaises(ValueError):
            self.run_apply(True)
        self.assertFalse(any(method == 'POST' for method, _ in self.calls))

    def test_nonprivate_file_is_refused(self):
        self.run_apply()
        os.chmod(self.file, 0o644)
        with self.assertRaises(ValueError):
            self.run_apply(True)

    def test_enable_closes_route_when_anonymous_request_is_accepted(self):
        self.run_apply()
        self.calls.clear()
        class Response(io.BytesIO):
            status = 200
        class Opener:
            def open(self, *args, **kwargs):
                return Response(b'[]')
        with patch.object(access, 'request', self.request), patch.object(access, 'pages', self.pages), patch.object(access.urllib.request, 'build_opener', return_value=Opener()):
            with self.assertRaises(ValueError):
                access.enable('account', 'token', 'worker', self.file, 'https://thrust-preview-provisioner.test.workers.dev')
        route_calls = [call for call in self.calls if call == ('POST', '/workers/scripts/thrust-preview-provisioner/subdomain')]
        self.assertEqual(len(route_calls), 2)

    def test_enable_requires_denial_and_authenticated_inventory(self):
        self.run_apply()
        self.calls.clear()
        class Response(io.BytesIO):
            status = 200
        class Opener:
            def open(self, req, **kwargs):
                if isinstance(req, str):
                    raise access.urllib.error.HTTPError(req, 403, 'denied', {}, io.BytesIO())
                return Response(b'[]')
        with patch.object(access, 'request', self.request), patch.object(access, 'pages', self.pages), patch.object(access.urllib.request, 'build_opener', return_value=Opener()):
            access.enable('account', 'token', 'worker', self.file, 'https://thrust-preview-provisioner.test.workers.dev')
        route_calls = [call for call in self.calls if call == ('POST', '/workers/scripts/thrust-preview-provisioner/subdomain')]
        self.assertEqual(len(route_calls), 1)


if __name__ == '__main__':
    unittest.main()
