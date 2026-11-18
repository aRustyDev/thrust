#!/usr/bin/env python3
"""Generated GitHub PR snapshots and controls for the source authorization boundary."""
import copy
import importlib.util
import json
import os
from pathlib import Path
import random
import subprocess
import sys
import tempfile
import unittest

SCRIPT = Path(os.environ.get("VERIFY_PR_HEAD_SCRIPT", Path(__file__).with_name("verify-pr-head.py")))
spec = importlib.util.spec_from_file_location("verify_pr_head", SCRIPT)
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class HeadTests(unittest.TestCase):
    def test_generated_snapshots_refuse_every_identity_change(self):
        rng = random.Random(57)
        repo = "aRustyDev/thrust"
        for _ in range(200):
            number = rng.randint(1, 999999999)
            sha = f"{rng.getrandbits(160):040x}"
            pull = {"number": number, "state": "open", "head": {"sha": sha, "repo": {"full_name": repo}},
                    "base": {"ref": "main", "repo": {"full_name": repo}}}
            self.assertTrue(module.verify(pull, repo, str(number), sha))
            self.assertEqual(module.select([[pull]], repo, sha), number)
            with self.assertRaises(ValueError):
                module.select([[pull, pull]], repo, sha)
            with self.assertRaises(ValueError):
                module.select([[]], repo, sha)
            with self.assertRaises(ValueError):
                module.select([pull], repo, sha)
            for path, value in (("number", number + 1), ("state", "closed"), ("head.sha", "f" * 40),
                                ("head.repo", None), ("head.repo.full_name", "fork/thrust"),
                                ("base.ref", "other"), ("base.repo.full_name", "other/thrust")):
                wrong = copy.deepcopy(pull)
                keys = path.split(".")
                target = wrong
                for key in keys[:-1]:
                    target = target[key]
                target[keys[-1]] = value
                self.assertFalse(module.verify(wrong, repo, str(number), sha), path)
            for wrong in ("0", "01", "-1", "1000000000"):
                self.assertFalse(module.verify(pull, repo, wrong, sha))
            self.assertFalse(module.verify(pull, repo, str(number), sha[:8]))

    def test_cli_refuses_malformed_json(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "pull.json"
            path.write_text("{")
            result = subprocess.run([sys.executable, "-I", str(SCRIPT), "--pull", str(path), "--repository", "aRustyDev/thrust",
                                     "--number", "1", "--sha", "a" * 40], capture_output=True, text=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertEqual(result.stdout, "")

    def test_cli_selects_then_rechecks_the_same_head(self):
        repo, sha = "aRustyDev/thrust", "a" * 40
        pull = {"number": 57, "state": "open", "head": {"sha": sha, "repo": {"full_name": repo}},
                "base": {"ref": "main", "repo": {"full_name": repo}}}
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "pull.json"
            path.write_text(json.dumps([[pull]]))
            common = [sys.executable, "-I", str(SCRIPT), "--repository", repo, "--sha", sha]
            result = subprocess.run([*common, "--pulls", str(path)], capture_output=True, text=True)
            self.assertEqual((result.returncode, result.stdout), (0, "57\n"), result.stderr)
            path.write_text(json.dumps(pull))
            self.assertEqual(subprocess.run([*common, "--pull", str(path), "--number", "57"], capture_output=True).returncode, 0)
            pull["state"] = "closed"
            path.write_text(json.dumps(pull))
            self.assertNotEqual(subprocess.run([*common, "--pull", str(path), "--number", "57"], capture_output=True).returncode, 0)


if __name__ == "__main__":
    unittest.main()
