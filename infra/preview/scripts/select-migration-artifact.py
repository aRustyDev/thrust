#!/usr/bin/env python3
"""Select exactly one source-run artifact without trusting its name as authorization."""
import argparse
import json
import re
import sys

NAME = re.compile(r"preview-migrations-pr-([1-9][0-9]{0,8})-([0-9a-f]{40})\Z")


def select(pages, run_id):
    if not isinstance(pages, list) or not pages or type(run_id) is not int or run_id <= 0:
        raise ValueError("invalid artifact inventory")
    entries = []
    for page in pages:
        if not isinstance(page, dict) or not isinstance(page.get("artifacts"), list):
            raise ValueError("invalid artifact page")
        entries.extend(page["artifacts"])
    if len(entries) != 1 or not isinstance(entries[0], dict):
        raise ValueError("source run must have exactly one artifact")
    item = entries[0]
    source = item.get("workflow_run")
    if (not isinstance(item.get("name"), str) or type(item.get("id")) is not int or item["id"] <= 0
            or item.get("expired") is not False or not isinstance(source, dict)
            or type(source.get("id")) is not int or source["id"] != run_id):
        raise ValueError("invalid or expired artifact")
    match = NAME.fullmatch(item["name"])
    if match is None:
        raise ValueError("invalid artifact name")
    return item["id"], match.group(1), match.group(2), item["name"]


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--inventory", required=True)
    parser.add_argument("--run", type=int, required=True)
    args = parser.parse_args()
    try:
        with open(args.inventory, "rb") as source:
            data = source.read(2 * 1024 * 1024 + 1)
        if len(data) > 2 * 1024 * 1024:
            raise ValueError("artifact inventory exceeds its limit")
        artifact_id, number, commit, name = select(json.loads(data), args.run)
    except (OSError, ValueError) as error:
        print(f"select-migration-artifact: {error}", file=sys.stderr)
        return 1
    print(f"id={artifact_id}\nnumber={number}\ncommit={commit}\nname={name}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
