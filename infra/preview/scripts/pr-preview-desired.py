#!/usr/bin/env python3
"""Emit the same-repository PRs eligible for a Thrust preview, newest first.

The caller supplies immutable JSON snapshots so it can fetch GitHub data before any
database or Cloudflare mutation. This keeps a failed or partial GitHub query from being
mistaken for an empty desired set.
"""
import argparse
import datetime as dt
import json
import sys


def timestamp(value: str) -> dt.datetime:
    return dt.datetime.fromisoformat(value.replace("Z", "+00:00"))


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--repository", required=True)
    parser.add_argument("--pulls", required=True, type=argparse.FileType())
    parser.add_argument("--events", required=True, type=argparse.FileType())
    parser.add_argument("--now", required=True)
    parser.add_argument("--max-age-days", type=int, default=14)
    parser.add_argument("--max-previews", type=int, default=50)
    args = parser.parse_args()
    if args.max_age_days < 1 or not 1 <= args.max_previews <= 50:
        parser.error("max-age-days must be positive and max-previews must be 1..50")

    pulls = json.load(args.pulls)
    events = json.load(args.events)
    if not isinstance(pulls, list) or not isinstance(events, list):
        raise ValueError("GitHub responses must be JSON arrays")
    # `gh api --paginate --slurp` produces a list of pages. Accept a single page too so the
    # hermetic fixture is small, but reject arbitrary mixed JSON rather than sweeping on it.
    if pulls and isinstance(pulls[0], list):
        if not all(isinstance(page, list) for page in pulls):
            raise ValueError("pull response mixes pages and objects")
        pulls = [pull for page in pulls for pull in page]
    if events and isinstance(events[0], list):
        if not all(isinstance(page, list) for page in events):
            raise ValueError("event response mixes pages and objects")
        events = [event for page in events for event in page]

    pushed = {}
    for event in events:
        if not isinstance(event, dict):
            raise ValueError("event response contains a non-object")
        payload = event.get("payload", {})
        if event.get("type") != "PullRequestEvent" or payload.get("action") != "synchronize":
            continue
        number = payload.get("number")
        created_at = event.get("created_at")
        if isinstance(number, int) and isinstance(created_at, str):
            pushed[number] = max(pushed.get(number, timestamp(created_at)), timestamp(created_at))

    now = timestamp(args.now)
    cutoff = now - dt.timedelta(days=args.max_age_days)
    desired = []
    for pull in pulls:
        if not isinstance(pull, dict):
            raise ValueError("pull response contains a non-object")
        number = pull.get("number")
        head_repo = (pull.get("head") or {}).get("repo") or {}
        created_at = pull.get("created_at")
        updated_at = pull.get("updated_at")
        if head_repo.get("full_name") != args.repository:
            continue
        if type(number) is not int or not 1 <= number <= 999999999 or not isinstance(created_at, str) or not isinstance(updated_at, str):
            raise ValueError("same-repository pull request has invalid number or timestamps")
        # GitHub's events feed can be truncated; PR updated_at is a conservative fallback:
        # comments can keep a preview longer, but an unseen push must not delete it early.
        last_activity = max(timestamp(created_at), timestamp(updated_at), pushed.get(number, timestamp(created_at)))
        if last_activity >= cutoff:
            desired.append((last_activity, number))
    desired.sort(reverse=True)
    for _, number in desired[:args.max_previews]:
        print(number)
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (ValueError, json.JSONDecodeError) as error:
        print(f"pr-preview-desired: could not evaluate GitHub inventory: {error}", file=sys.stderr)
        raise SystemExit(3)
