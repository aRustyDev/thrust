#!/usr/bin/env python3
"""Make, or check, one of the edge's two Access service tokens, without its secret ever entering Terraform state (ADR-0111).

    service-token.py ensure --name thrust-ci-opt-in|thrust-edge-previews --credentials-file <path> [--duration 8760h]
    service-token.py check  --name ... --credentials-file <path>

  thrust-ci-opt-in      what the target deploys show Access on the edge's opt-in paths (environments `target`,
                        `target-migrate`: EDGE_CI_CLIENT_ID, EDGE_CI_CLIENT_SECRET)
  thrust-edge-previews  what the edge shows Access on the production Worker's Previews, the only credential admitted there
                        (environment `edge`: PREVIEW_ACCESS_CLIENT_ID, PREVIEW_ACCESS_CLIENT_SECRET)

`ensure` with no credentials file makes the token and writes {id, name, client_id, client_secret} to a NEW file only its owner
can read (refused if a token of that name already exists: its secret cannot be read back, so rotate or delete it first). With
the file, and `check` always, it verifies the file (a private regular file, not a link) against Cloudflare: the same id, name
and client id, and not expired. It prints the token's id (this root's TF_VAR input) and client id, never the secret.

Environment: CLOUDFLARE_ACCOUNT_ID, and CLOUDFLARE_API_TOKEN with Access: Service Tokens Edit (Read for `check`).
Exit status: 0 done; 1 refused or Cloudflare failed (why, on stderr); 2 called wrongly.
"""
import argparse
import datetime
import json
import os
import re
import stat
import sys
import urllib.request

NAMES = ("thrust-ci-opt-in", "thrust-edge-previews")
BASE = "https://api.cloudflare.com/client/v4/accounts/"
ID = re.compile(r"[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}")
WARN_DAYS = 30


class Refused(Exception):
    pass


def request(account, token, method, path, body=None):
    req = urllib.request.Request(
        BASE + account + path,
        data=None if body is None else json.dumps(body).encode(),
        headers={"Authorization": "Bearer " + token, "Content-Type": "application/json"},
        method=method,
    )
    with urllib.request.urlopen(req, timeout=30) as response:
        value = json.load(response)
    if value.get("success") is not True or "result" not in value:
        raise Refused("Cloudflare answered without success")
    return value["result"]


def pages(account, token, path):
    result, page = [], 1
    while True:
        req = urllib.request.Request(BASE + account + f"{path}?per_page=100&page={page}",
                                     headers={"Authorization": "Bearer " + token})
        with urllib.request.urlopen(req, timeout=30) as response:
            value = json.load(response)
        if value.get("success") is not True or not isinstance(value.get("result"), list):
            raise Refused("Cloudflare answered a list without success")
        result.extend(value["result"])
        info = value.get("result_info")
        if not isinstance(info, dict) or "total_pages" not in info:
            raise Refused("Cloudflare's list has no pagination")
        if page >= info["total_pages"]:
            return result
        page += 1


def read_credentials(path):
    """The credentials file, if there is one: a private regular file (no link), holding the four fields."""
    try:
        fd = os.open(path, os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0))
    except FileNotFoundError:
        return None
    except OSError as e:
        raise Refused(f"the credentials file cannot be opened as a plain file ({e.strerror})")
    with os.fdopen(fd) as f:
        mode = os.fstat(f.fileno()).st_mode
        if not stat.S_ISREG(mode) or mode & 0o077:
            raise Refused("the credentials file must be a regular file only its owner can read (chmod 600)")
        try:
            creds = json.load(f)
        except ValueError:
            raise Refused("the credentials file is not JSON")
    if not isinstance(creds, dict) or not all(isinstance(creds.get(k), str) and creds[k] for k in ("id", "name", "client_id", "client_secret")):
        raise Refused("the credentials file lacks id, name, client_id or client_secret")
    if not ID.fullmatch(creds["id"]):
        raise Refused("the credentials file's id is not a service token id")
    return creds


def expiry_note(expires_at, now):
    if not expires_at:
        return "no expiry"
    try:
        when = datetime.datetime.fromisoformat(expires_at.replace("Z", "+00:00"))
    except ValueError:
        raise Refused(f"Cloudflare's expires_at is not a date: {expires_at!r}")
    if when <= now:
        raise Refused(f"the token expired at {expires_at}: rotate it")
    days = (when - now).days
    return f"expires {expires_at}" + (f" (in {days} days: ROTATE IT SOON)" if days < WARN_DAYS else "")


def verify(account, token, name, creds, now):
    if creds["name"] != name:
        raise Refused(f"the credentials file is for {creds['name']!r}, not {name!r}")
    live = request(account, token, "GET", "/access/service_tokens/" + creds["id"])
    if live.get("id") != creds["id"] or live.get("name") != name or live.get("client_id") != creds["client_id"]:
        raise Refused("the token in Cloudflare is not the one in the credentials file (id, name or client id differ)")
    return expiry_note(live.get("expires_at"), now)


def create(account, token, name, path, duration):
    if any(t.get("name") == name for t in pages(account, token, "/access/service_tokens")):
        raise Refused(f"a service token named {name!r} exists and there is no credentials file for it: its secret cannot be read "
                      "back. Find the file, or rotate or delete the token in Cloudflare and run again")
    # the file is made (new, private) BEFORE the token, so a secret is never made with nowhere to put it; it is removed again if
    # the token cannot be made or written
    f = os.fdopen(os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | getattr(os, "O_NOFOLLOW", 0), 0o600), "w")
    try:
        with f:
            made = request(account, token, "POST", "/access/service_tokens", {"name": name, "duration": duration})
            creds = {"id": made["id"], "name": name, "client_id": made["client_id"], "client_secret": made["client_secret"]}
            if not ID.fullmatch(creds["id"]):
                raise Refused("Cloudflare answered a token whose id is not a service token id")
            json.dump(creds, f)
            f.flush()
            os.fsync(f.fileno())
    except BaseException:
        try:
            os.unlink(path)
        except OSError:
            pass
        raise
    return creds, f"made, valid for {duration}"


def main(argv=None, now=None):
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    p.add_argument("command", choices=("ensure", "check"))
    p.add_argument("--name", required=True, choices=NAMES)
    p.add_argument("--credentials-file", required=True)
    p.add_argument("--duration", default="8760h")
    a = p.parse_args(argv)
    if not re.fullmatch(r"[1-9][0-9]{0,5}h", a.duration):
        p.error("--duration is a whole number of hours, such as 8760h")
    account = os.environ.get("CLOUDFLARE_ACCOUNT_ID", "")
    token = os.environ.get("CLOUDFLARE_API_TOKEN", "")
    if not re.fullmatch(r"[0-9a-f]{32}", account) or not token:
        p.error("set CLOUDFLARE_ACCOUNT_ID (32 hex digits) and CLOUDFLARE_API_TOKEN")
    now = now or datetime.datetime.now(datetime.timezone.utc)
    try:
        creds = read_credentials(a.credentials_file)
        if creds is None:
            if a.command == "check":
                raise Refused("there is no credentials file to check")
            creds, note = create(account, token, a.name, a.credentials_file, a.duration)
        else:
            note = verify(account, token, a.name, creds, now)
    except Refused as e:
        print(f"service-token: refused: {e}", file=sys.stderr)
        return 1
    except (OSError, KeyError, ValueError) as e:
        print(f"service-token: Cloudflare could not be asked, or answered unexpectedly: {type(e).__name__}", file=sys.stderr)
        return 1
    print(f"{a.name}: {note}")
    print(f"id={creds['id']}")
    print(f"client_id={creds['client_id']}")
    print(f"the secret stays in {a.credentials_file}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
