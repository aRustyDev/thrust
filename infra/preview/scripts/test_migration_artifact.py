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
VERIFY = Path(os.environ.get("VERIFY_MIGRATION_ARTIFACT_SCRIPT", Path(__file__).with_name("verify-migration-artifact.py")))
AUTHORIZE = Path(os.environ.get("AUTHORIZE_MIGRATION_SCRIPT", Path(__file__).with_name("authorize-migration-artifact.py")))
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

    def run_verify(self, commit, artifact):
        return subprocess.run([sys.executable, "-I", str(VERIFY), "--repo", str(self.repo), "--commit", commit,
                               "--artifact", str(artifact)], capture_output=True)

    def test_generated_artifacts_are_reproduced_from_git_and_tampering_is_rejected(self):
        rng = random.Random(339)
        for number in range(1, 7):
            (self.path / f"{number:04d}_test.sql").write_text(f"SELECT {rng.getrandbits(48)};\n")
        commit = self.commit()
        produced = self.run_artifact(commit)
        self.assertEqual(produced.returncode, 0, produced.stderr)
        raw = produced.stdout.encode()
        artifact = self.repo / "artifact.json"
        artifact.write_bytes(raw)
        valid = self.run_verify(commit, artifact)
        self.assertEqual(valid.returncode, 0, valid.stderr)
        self.assertEqual(valid.stdout, raw)

        # The command must combine freshly fetched metadata and the independently reproduced bytes.
        repo, number, source_sha = "aRustyDev/thrust", 57, "a" * 40
        run = {"id": 5, "name": "Prepare preview migration source", "event": "workflow_run", "head_branch": "main",
               "head_sha": source_sha, "path": ".github/workflows/preview-migration-source.yml", "status": "completed",
               "conclusion": "success", "repository": {"id": 3, "full_name": repo},
               "head_repository": {"id": 3, "full_name": repo}}
        info = {"id": 7, "expired": False, "name": f"preview-migrations-pr-{number}-{commit}",
                "workflow_run": {"id": 5, "repository_id": 3, "head_repository_id": 3,
                                 "head_branch": "main", "head_sha": source_sha}}
        pull = {"number": number, "state": "open", "head": {"sha": commit, "repo": {"full_name": repo}},
                "base": {"ref": "main", "repo": {"full_name": repo}}}
        metadata_files = [self.repo / name for name in ("run.json", "info.json", "pull.json")]
        for path, data in zip(metadata_files, (run, info, pull)):
            path.write_text(json.dumps(data))
        command = [sys.executable, "-I", str(AUTHORIZE), "--repo", str(self.repo), "--repository", repo,
                   "--number", str(number), "--commit", commit, "--run", str(metadata_files[0]),
                   "--metadata", str(metadata_files[1]), "--pull", str(metadata_files[2]), "--artifact", str(artifact)]
        accepted = subprocess.run(command, capture_output=True)
        self.assertEqual(accepted.returncode, 0, accepted.stderr)
        self.assertEqual(accepted.stdout, raw)
        pull["state"] = "closed"
        metadata_files[2].write_text(json.dumps(pull))
        closed = subprocess.run(command, capture_output=True)
        self.assertNotEqual(closed.returncode, 0)
        self.assertEqual(closed.stdout, b"")
        pull["state"] = "open"
        metadata_files[2].write_text(json.dumps(pull))

        # A hostile producer can recompute the embedded checksum, so validate actual SQL against Git.
        value = json.loads(raw)
        for index in range(len(value["files"])):
            forged = json.loads(raw)
            forged["files"][index]["sql"] += f"-- forged {rng.getrandbits(64)}\n"
            forged["files"][index]["sha256"] = hashlib.sha256(forged["files"][index]["sql"].encode()).hexdigest()
            artifact.write_text(json.dumps(forged, ensure_ascii=True, separators=(",", ":")) + "\n")
            rejected = self.run_verify(commit, artifact)
            self.assertNotEqual(rejected.returncode, 0, index)
            self.assertEqual(rejected.stdout, b"")
            if index in (0, len(value["files"]) - 1):
                rejected = subprocess.run(command, capture_output=True)
                self.assertNotEqual(rejected.returncode, 0, index)
                self.assertEqual(rejected.stdout, b"")
        for forged in (raw + b" ", raw[:-1], b"{}", b"", b"x" * (32 * 1024 * 1024 + 1)):
            artifact.write_bytes(forged)
            rejected = self.run_verify(commit, artifact)
            self.assertNotEqual(rejected.returncode, 0)
            self.assertEqual(rejected.stdout, b"")

        # The same supplied artifact must not validate as a later commit or through a symlink.
        artifact.write_bytes(raw)
        (self.path / "0001_test.sql").write_text("SELECT 999;\n")
        later = self.commit()
        self.assertNotEqual(self.run_verify(later, artifact).returncode, 0)
        link = self.repo / "artifact-link"
        os.symlink(artifact, link)
        self.assertNotEqual(self.run_verify(commit, link).returncode, 0)

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
