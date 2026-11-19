#!/usr/bin/env python3
"""Check a freshly fetched GitHub pull-request object against the requested PR and commit.

The caller must obtain the JSON directly from GitHub, and repeat the check immediately before
any privileged use: this point-in-time result does not prevent a later push or close.
"""
import argparse
import json
import re
import sys


def verify(pull, repository, number, sha):
    if (not isinstance(pull, dict) or not re.fullmatch(r"[1-9][0-9]{0,8}", number)
            or not re.fullmatch(r"[0-9a-f]{40}", sha) or not isinstance(repository, str)):
        return False
    head, base = pull.get("head"), pull.get("base")
    if not isinstance(head, dict) or not isinstance(base, dict):
        return False
    head_repo, base_repo = head.get("repo"), base.get("repo")
    return (type(pull.get("number")) is int and pull["number"] == int(number) and pull.get("state") == "open"
            and isinstance(head_repo, dict) and head_repo.get("full_name") == repository
            and isinstance(base_repo, dict) and base_repo.get("full_name") == repository
            and base.get("ref") == "main" and head.get("sha") == sha)


def select(pages, repository, sha):
    if not isinstance(pages, list) or not all(isinstance(page, list) for page in pages):
        raise ValueError("pull request inventory is not a list of pages")
    matches = []
    for page in pages:
        for pull in page:
            if not isinstance(pull, dict):
                raise ValueError("pull request inventory contains a non-object")
            number = pull.get("number")
            if type(number) is int and verify(pull, repository, str(number), sha):
                matches.append(number)
    if len(matches) != 1:
        raise ValueError("head commit does not identify exactly one eligible open PR")
    return matches[0]


def main():
    parser = argparse.ArgumentParser()
    inputs = parser.add_mutually_exclusive_group(required=True)
    inputs.add_argument("--pull")
    inputs.add_argument("--pulls")
    parser.add_argument("--repository", required=True)
    parser.add_argument("--number")
    parser.add_argument("--sha", required=True)
    args = parser.parse_args()
    try:
        with open(args.pull or args.pulls, encoding="utf-8") as source:
            data = json.load(source)
        if args.pulls:
            if args.number is not None:
                raise ValueError("--number cannot be used when selecting a PR")
            print(select(data, args.repository, args.sha))
        elif args.number is None or not verify(data, args.repository, args.number, args.sha):
            raise ValueError("pull request is closed, changed, from a fork, or targets another base")
    except (OSError, ValueError) as error:
        print(f"verify-pr-head: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
