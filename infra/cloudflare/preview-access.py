#!/usr/bin/env python3
"""Create/check Worker-level Access without putting credentials in Terraform state.

The caller keeps the generated JSON credential file in a secure location and copies its
client_id and client_secret to the GitHub environments. Reusing the same file is idempotent.
"""
import argparse
import json
import os
import re
import stat
import sys
import time
import urllib.error
import urllib.request
import urllib.parse

NAME = "thrust-preview-provisioner"
BASE = "https://api.cloudflare.com/client/v4/accounts/"


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
        raise ValueError("Cloudflare API returned an unsuccessful response")
    return value


def pages(account, token, path):
    result = []
    page = 1
    while True:
        value = request(account, token, "GET", f"{path}?per_page=100&page={page}")
        if not isinstance(value["result"], list):
            raise ValueError("Cloudflare API returned an invalid list")
        result.extend(value["result"])
        info = value.get("result_info")
        if not isinstance(info, dict) or "total_pages" not in info:
            raise ValueError("Cloudflare API omitted pagination metadata")
        if page >= info["total_pages"]:
            return result
        page += 1


def policy_is_exact(policies, token_id):
    if len(policies) != 1:
        return False
    p = policies[0]
    return (p.get("decision") == "non_identity" and p.get("include") == [{"service_token": {"token_id": token_id}}]
            and not p.get("require") and not p.get("exclude"))


def apply(account, api_token, worker_id, credentials_file, check_only=False):
    worker = request(account, api_token, "GET", "/workers/workers/" + worker_id)["result"]
    if worker.get("id") != worker_id or worker.get("name") != NAME:
        raise ValueError("Worker ID does not identify thrust-preview-provisioner")
    apps = pages(account, api_token, "/access/apps")
    matches = [a for a in apps if any(d.get("type") == "worker" and d.get("worker_id") == worker_id
                                      for d in a.get("destinations", []))]
    if len(matches) > 1:
        raise ValueError("multiple Worker Access applications require operator review")
    try:
        fd = os.open(credentials_file, os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0))
    except FileNotFoundError:
        if check_only:
            raise ValueError("credential file is required for verification")
        if matches:
            raise ValueError("Access application exists but credential file is absent")
        fd = os.open(credentials_file, os.O_WRONLY | os.O_CREAT | os.O_EXCL | getattr(os, "O_NOFOLLOW", 0), 0o600)
        with os.fdopen(fd, "w") as f:
            result = request(account, api_token, "POST", "/access/service_tokens", {"name": NAME})["result"]
            credentials = {key: result[key] for key in ("id", "client_id", "client_secret")}
            json.dump(credentials, f)
            f.flush()
            os.fsync(f.fileno())
    else:
        with os.fdopen(fd) as f:
            mode = os.fstat(f.fileno()).st_mode
            if not stat.S_ISREG(mode) or mode & 0o077:
                raise ValueError("credential file must be a private regular file")
            credentials = json.load(f)
        if not all(isinstance(credentials.get(k), str) and credentials[k] for k in ("id", "client_id", "client_secret")):
            raise ValueError("invalid credential file")
        if not re.fullmatch(r"[A-Za-z0-9_-]{1,128}", credentials["id"]):
            raise ValueError("invalid service token identifier")
        token = request(account, api_token, "GET", "/access/service_tokens/" + credentials["id"])["result"]
        if token.get("id") != credentials["id"] or token.get("client_id") != credentials["client_id"] or token.get("enabled") is False:
            raise ValueError("stored service token does not match Cloudflare")
    token_id = credentials["id"]
    if not re.fullmatch(r"[A-Za-z0-9_-]{1,128}", token_id):
        raise ValueError("invalid service token identifier")
    if not matches:
        if check_only:
            raise ValueError("Worker Access application is missing")
        app = {"type": "self_hosted", "name": NAME,
               "destinations": [{"type": "worker", "worker_id": worker_id}],
               "policies": [{"name": NAME, "decision": "non_identity", "include": [{"service_token": {"token_id": token_id}}]}]}
        matches = [request(account, api_token, "POST", "/access/apps", app)["result"]]
    app = matches[0]
    if app.get("name") != NAME:
        raise ValueError("unexpected Access application owns the Worker")
    if not re.fullmatch(r"[A-Za-z0-9_-]{1,128}", app.get("id", "")):
        raise ValueError("invalid Access application identifier")
    policies = pages(account, api_token, "/access/apps/" + app["id"] + "/policies")
    if not policy_is_exact(policies, token_id):
        raise ValueError("Access policy differs from the required service-token-only policy")
    print("Worker Access policy verified; credentials stored in", credentials_file)


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        return None


def enable(account, api_token, worker_id, credentials_file, url):
    apply(account, api_token, worker_id, credentials_file, check_only=True)
    worker = request(account, api_token, "GET", "/workers/workers/" + worker_id)["result"]
    parsed = urllib.parse.urlsplit(url)
    if (worker.get("subdomain", {}).get("url", "").rstrip("/") != url.rstrip("/") or parsed.scheme != "https"
            or not parsed.hostname or not parsed.hostname.endswith(".workers.dev") or parsed.path not in ("", "/")
            or parsed.query or parsed.fragment or parsed.username or parsed.port):
        raise ValueError("URL does not match the Worker's workers.dev origin")
    with open(credentials_file) as f:
        credentials = json.load(f)
    endpoint = url.rstrip("/") + "/v1/previews"
    path = "/workers/scripts/" + NAME + "/subdomain"
    passed = False
    try:
        request(account, api_token, "POST", path, {"enabled": True, "previews_enabled": False})
        opener = urllib.request.build_opener(NoRedirect)
        user_agent = "thrust-preview-control-plane/1.0"
        for attempt in range(13):
            try:
                with opener.open(urllib.request.Request(endpoint, headers={"User-Agent": user_agent}), timeout=30):
                    raise ValueError("anonymous request unexpectedly reached the Worker")
            except urllib.error.HTTPError as exc:
                login_host = urllib.parse.urlsplit(exc.headers.get("Location", "")).hostname or ""
                body = exc.read(512)
                allowed_denial = ((exc.code == 403 and b"error code: 1010" not in body)
                                  or (exc.code == 302 and login_host.endswith(".cloudflareaccess.com")))
                exc.close()
                if exc.code == 404 and b"error code: 1042" in body and attempt < 12:
                    time.sleep(5)
                    continue
                if not allowed_denial:
                    raise ValueError("anonymous request was not rejected by Access") from exc
                break
        headers = {"User-Agent": user_agent, "CF-Access-Client-Id": credentials["client_id"], "CF-Access-Client-Secret": credentials["client_secret"]}
        with opener.open(urllib.request.Request(endpoint, headers=headers), timeout=30) as response:
            if response.status != 200 or not isinstance(json.load(response), list):
                raise ValueError("authenticated inventory smoke failed")
        passed = True
    finally:
        if not passed:
            try:
                closed = request(account, api_token, "POST", path, {"enabled": False, "previews_enabled": False})["result"]
                if closed.get("enabled") is not False or closed.get("previews_enabled") is not False:
                    raise ValueError("route disable was not confirmed")
            except Exception:
                print("CRITICAL: could not confirm route disable; close thrust-preview-provisioner in Cloudflare immediately", file=sys.stderr)
                raise
    print("Worker route enabled and Access smoke checks passed")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--worker-id", required=True)
    parser.add_argument("--credentials-file", required=True)
    parser.add_argument("--check-only", action="store_true")
    parser.add_argument("--enable-url", help="enable and smoke-test this exact workers.dev origin")
    args = parser.parse_args()
    account = os.environ.get("CLOUDFLARE_ACCOUNT_ID", "")
    token = os.environ.get("CLOUDFLARE_API_TOKEN", "")
    if not re.fullmatch(r"[0-9a-f]{32}", account) or not re.fullmatch(r"[0-9a-f]{32}", args.worker_id) or not token:
        parser.error("set CLOUDFLARE_ACCOUNT_ID, CLOUDFLARE_API_TOKEN and a 32-hex Worker ID")
    try:
        if args.enable_url:
            enable(account, token, args.worker_id, args.credentials_file, args.enable_url)
        else:
            apply(account, token, args.worker_id, args.credentials_file, args.check_only)
    except (ValueError, KeyError, OSError, urllib.error.URLError) as exc:
        print(f"preview-access: {type(exc).__name__}: setup or verification failed; inspect Cloudflare state", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
