#!/usr/bin/env python3
"""Exercise artifact construction against real Git objects, including hostile trees."""

import hashlib
import json
import os
from pathlib import Path
import random
import subprocess
import sys
import tempfile
import unittest

SCRIPT = Path(os.environ.get("MIGRATION_ARTIFACT_SCRIPT", Path(__file__).with_name("migration-artifact.py")))
DIR = Path("crates/adapters/postgres/migrations")


class ArtifactTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.repo = Path(self.temp.name)
        self.git("init", "-q")
        self.path = self.repo / DIR
        self.path.mkdir(parents=True)

    def tearDown(self):
        self.temp.cleanup()

    def git(self, *args):
        return subprocess.run(["git", "-C", str(self.repo), "-c", "user.name=Test", "-c", "user.email=test@example.test",
                               "-c", "commit.gpgsign=false", "-c", "core.hooksPath=/dev/null", *args],
                              check=True, capture_output=True, text=True).stdout.strip()

    def commit(self):
        self.git("add", "--all")
        self.git("commit", "--allow-empty", "-qm", "fixture")
        return self.git("rev-parse", "HEAD")

    def run_artifact(self, commit):
        return subprocess.run([sys.executable, "-I", str(SCRIPT), "--repo", str(self.repo), "--commit", commit],
                              capture_output=True, text=True)

    def test_generated_git_contents_are_bound_to_commit_and_ignore_worktree(self):
        rng = random.Random(57)
        expected = {}
        for number in range(1, 21):
            name = f"{number:04d}_test.sql"
            sql = f"-- generated {rng.getrandbits(64)}\nSELECT {number};\n"
            (self.path / name).write_text(sql)
            expected[name] = sql
        old = self.commit()
        (self.path / "0001_test.sql").write_text("SELECT 999;\n")
        new = self.commit()
        (self.path / "0001_test.sql").write_text("uncommitted changes")
        result = self.run_artifact(old)
        self.assertEqual(result.returncode, 0, result.stderr)
        data = json.loads(result.stdout)
        self.assertEqual((data["version"], data["commit"]), (1, old))
        self.assertEqual([f["name"] for f in data["files"]], sorted(expected))
        for item in data["files"]:
            self.assertEqual(item["sql"], expected[item["name"]])
            self.assertEqual(item["sha256"], hashlib.sha256(item["sql"].encode()).hexdigest())
        self.assertNotEqual(json.loads(self.run_artifact(new).stdout)["files"][0]["sha256"], data["files"][0]["sha256"])

    def test_invalid_input_and_tree_fail_without_output(self):
        path = self.path / "0001_test.sql"
        path.write_text("SELECT 1;\n")
        good = self.commit()
        for bad in ("HEAD", good[:8], good.upper(), "0" * 40):
            result = self.run_artifact(bad)
            self.assertNotEqual(result.returncode, 0)
            self.assertEqual(result.stdout, "")
        cases = [("0001_other.sql", b"SELECT 1;"), ("0002_test.sql", b"bad\x00sql"),
                 ("0003_test.sql", b"\xff"), ("0004_test.sql", b"x" * (512 * 1024 + 1)),
                 ("bad.sql", b"SELECT 1;")]
        for name, data in cases:
            with self.subTest(name=name):
                bad = self.path / name
                bad.write_bytes(data)
                result = self.run_artifact(self.commit())
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual(result.stdout, "")
                bad.unlink()
        link = self.path / "0005_link.sql"
        os.symlink("0001_test.sql", link)
        result = self.run_artifact(self.commit())
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(result.stdout, "")


if __name__ == "__main__":
    unittest.main()
