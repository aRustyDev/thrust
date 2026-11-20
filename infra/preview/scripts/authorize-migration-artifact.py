#!/usr/bin/env python3
"""Authorize and reproduce a migration artifact from freshly fetched GitHub API metadata.

The caller must fetch the run, artifact and PR objects directly from GitHub, use a trusted
checkout and recheck the PR immediately before privileged use. These point-in-time checks do
not prevent a later push or closure and do not authorize a database binding or Worker route.
"""
import argparse
import importlib.util
import json
from pathlib import Path
import re
import subprocess
import sys


def load_script(name, filename):
    spec = importlib.util.spec_from_file_location(name, Path(__file__).with_name(filename))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


pr_check = load_script("verify_pr_head", "verify-pr-head.py")
artifact_check = load_script("verify_migration_artifact", "verify-migration-artifact.py")


def positive_int(value):
    return type(value) is int and value > 0


def authorize(run, artifact, pull, repository, number, commit):
    if (not isinstance(run, dict) or not isinstance(artifact, dict)
            or not isinstance(repository, str) or not re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", repository)
            or not isinstance(number, str) or not isinstance(commit, str)
            or not pr_check.verify(pull, repository, number, commit)):
        return False
    repo, head_repo, source = run.get("repository"), run.get("head_repository"), artifact.get("workflow_run")
    if not isinstance(repo, dict) or not isinstance(head_repo, dict) or not isinstance(source, dict):
        return False
    repo_id, run_id = repo.get("id"), run.get("id")
    head_sha = run.get("head_sha")
    return (positive_int(repo_id) and positive_int(run_id) and positive_int(artifact.get("id"))
            and positive_int(head_repo.get("id")) and positive_int(source.get("id"))
            and positive_int(source.get("repository_id")) and positive_int(source.get("head_repository_id"))
            and repo.get("full_name") == repository and head_repo.get("full_name") == repository
            and head_repo.get("id") == repo_id
            and run.get("name") == "Prepare preview migration source"
            and run.get("path") == ".github/workflows/preview-migration-source.yml"
            and run.get("event") == "workflow_run" and run.get("head_branch") == "main"
            and isinstance(head_sha, str) and re.fullmatch(r"[0-9a-f]{40}", head_sha) is not None
            and run.get("status") == "completed" and run.get("conclusion") == "success"
            and artifact.get("expired") is False
            and artifact.get("name") == f"preview-migrations-pr-{number}-{commit}"
            and source.get("id") == run_id and source.get("repository_id") == repo_id
            and source.get("head_repository_id") == repo_id and source.get("head_branch") == "main"
            and source.get("head_sha") == head_sha)


def read_json(path):
    with open(path, "rb") as source:
        data = source.read(2 * 1024 * 1024 + 1)
    if len(data) > 2 * 1024 * 1024:
        raise ValueError("GitHub metadata exceeds its limit")
    return json.loads(data)


def main():
    parser = argparse.ArgumentParser()
    for name in ("repo", "repository", "number", "commit", "run", "metadata", "pull", "artifact"):
        parser.add_argument(f"--{name}", required=True)
    args = parser.parse_args()
    try:
        if not authorize(read_json(args.run), read_json(args.metadata), read_json(args.pull),
                         args.repository, args.number, args.commit):
            raise ValueError("GitHub metadata does not authorize this open PR and source artifact")
        verified = artifact_check.verify(args.repo, args.commit, args.artifact)
    except (OSError, UnicodeError, ValueError, subprocess.TimeoutExpired) as error:
        print(f"authorize-migration-artifact: {error}", file=sys.stderr)
        return 1
    sys.stdout.buffer.write(verified)
    return 0


if __name__ == "__main__":
    sys.exit(main())
