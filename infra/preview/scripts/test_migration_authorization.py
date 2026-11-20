#!/usr/bin/env python3
"""Generated authorization decisions for independently fetched GitHub metadata."""
import copy
import importlib.util
import os
from pathlib import Path
import random
import unittest

SCRIPT = Path(os.environ.get("AUTHORIZE_MIGRATION_SCRIPT", Path(__file__).with_name("authorize-migration-artifact.py")))
spec = importlib.util.spec_from_file_location("authorize_migration", SCRIPT)
auth = importlib.util.module_from_spec(spec)
spec.loader.exec_module(auth)
selector_spec = importlib.util.spec_from_file_location("select_migration", Path(__file__).with_name("select-migration-artifact.py"))
selector = importlib.util.module_from_spec(selector_spec)
selector_spec.loader.exec_module(selector)

REPOSITORY = "aRustyDev/thrust"


def metadata(number, commit, repo_id, run_id, artifact_id, source_sha):
    run = {"id": run_id, "name": "Prepare preview migration source",
           "path": ".github/workflows/preview-migration-source.yml", "event": "workflow_run",
           "head_branch": "main", "head_sha": source_sha, "status": "completed", "conclusion": "success",
           "repository": {"id": repo_id, "full_name": REPOSITORY},
           "head_repository": {"id": repo_id, "full_name": REPOSITORY}}
    artifact = {"id": artifact_id, "expired": False, "name": f"preview-migrations-pr-{number}-{commit}",
                "workflow_run": {"id": run_id, "repository_id": repo_id, "head_repository_id": repo_id,
                                 "head_branch": "main", "head_sha": source_sha}}
    pull = {"number": number, "state": "open", "head": {"sha": commit, "repo": {"full_name": REPOSITORY}},
            "base": {"ref": "main", "repo": {"full_name": REPOSITORY}}}
    return run, artifact, pull


class AuthorizationTests(unittest.TestCase):
    def test_generated_metadata_accepts_only_the_expected_successful_source_and_open_pr(self):
        rng = random.Random(339)
        for _ in range(30):
            number = rng.randrange(1, 1_000_000_000)
            commit = f"{rng.getrandbits(160):040x}"
            source_sha = f"{rng.getrandbits(160):040x}"
            run, artifact, pull = metadata(number, commit, rng.randrange(10, 1_000_000),
                                           rng.randrange(10, 1_000_000), rng.randrange(10, 1_000_000), source_sha)
            self.assertTrue(auth.authorize(run, artifact, pull, REPOSITORY, str(number), commit))
            pages = [{"artifacts": [artifact]}]
            self.assertEqual(selector.select(pages, run["id"]), (artifact["id"], str(number), commit, artifact["name"]))
            for malformed in ([], [{}], [{"artifacts": []}], [{"artifacts": [artifact, artifact]}],
                              [{"artifacts": [dict(artifact, expired=True)]}],
                              [{"artifacts": [dict(artifact, name="invalid")]}]):
                with self.assertRaises(ValueError):
                    selector.select(malformed, run["id"])
            mutations = [
                (0, ("id",), True), (0, ("name",), "other"), (0, ("path",), "other.yml"),
                (0, ("event",), "pull_request"), (0, ("head_branch",), "feature"),
                (0, ("head_sha",), "not-a-sha"), (0, ("status",), "in_progress"), (0, ("conclusion",), "failure"),
                (0, ("repository", "full_name"), "other/repo"), (0, ("head_repository", "id"), True),
                (0, ("head_repository", "full_name"), "other/repo"),
                (1, ("id",), False), (1, ("expired",), True), (1, ("name",), "other"),
                (1, ("workflow_run", "id"), run["id"] + 1),
                (1, ("workflow_run", "repository_id"), run["repository"]["id"] + 1),
                (1, ("workflow_run", "head_repository_id"), True),
                (1, ("workflow_run", "head_branch"), "other"), (1, ("workflow_run", "head_sha"), "0" * 40),
                (2, ("number",), number + 1), (2, ("state",), "closed"), (2, ("head", "sha"), "0" * 40),
                (2, ("head", "repo", "full_name"), "other/repo"), (2, ("base", "ref"), "other"),
                (2, ("base", "repo", "full_name"), "other/repo"),
            ]
            for item, path, replacement in mutations:
                objects = copy.deepcopy([run, artifact, pull])
                target = objects[item]
                for key in path[:-1]:
                    target = target[key]
                target[path[-1]] = replacement
                self.assertFalse(auth.authorize(*objects, REPOSITORY, str(number), commit), (number, path))
            for invalid in (None, True, [], "bad"):
                self.assertFalse(auth.authorize(invalid, artifact, pull, REPOSITORY, str(number), commit))
                self.assertFalse(auth.authorize(run, invalid, pull, REPOSITORY, str(number), commit))


if __name__ == "__main__":
    unittest.main()
