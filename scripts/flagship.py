#!/usr/bin/env python3
"""Flagship as code (ADR-0111): the flags of the edge's Flagship app, made and changed only by CI.

    python3 -I scripts/flagship.py [--policy <file>] [--edge-config <file>] <command>
      validate                      the policy alone, offline
      check [--evaluate] [--retries <n>]
      sync
      put <target> <value.json>     create or replace target-<target>
      alias <alias> <target|prod>   point alias-<alias> at a live target, or at production
      delete <target>               remove target-<target>

The app holds the three kinds of flag crates/edge reads (crates/edge/src/route.rs); a `.` in a name is `_` in its key:
  routing          boolean, the kill switch for every target. Its value is the policy's `routing`.
  alias-<alias>    string, the target an alias names (`prod` is production). Only the policy's `aliases`.
  target-<name>    JSON, one per live target. Its `allowed` variation is the target, {url, deployment, release, kind, data,
                   expires}; its default, `denied`, is {}. Its rules are the policy's `targets[<kind>]`: one rule per entry,
                   serving `allowed` when the principal's kind and the audience it opted in through are both listed.
Everything about a flag but the alias's target and the target's value follows from the policy, so:
  check      reads every flag and reports each difference from what the policy makes of it, each flag that is none of the
             three, and each value the edge would refuse (an alias naming no live target is one). With --evaluate it also
             asks Flagship what every flag serves, for every principal kind and audience, and compares that with the policy;
             --retries re-asks a mismatch up to <n> more times, FLAGSHIP_RETRY_SECONDS (default 5) apart, to outwait
             propagation after a write. Exit 1 on any finding.
  sync       rewrites each flag that differs, creates `routing` when it is missing, deletes each flag that is none of the
             three, then reads everything back and checks again. A value it cannot know (an invalid one) it leaves alone
             and reports. Exit 1 when anything is left.
  validate   the policy's shape and names; no `@` anywhere, as the repository is public; its audiences are exactly the
             edge's (the ACCESS_AUD_<AUDIENCE> variables of crates/edge/wrangler.deploy.toml).
put, alias and delete read the flag back and exit 1 when it is not what they wrote.

Environment (all but validate): FLAGSHIP_API_TOKEN (never an argument), FLAGSHIP_ACCOUNT_ID, FLAGSHIP_APP_ID,
PREVIEW_HOST_SUFFIX (the edge's: `-<worker>.<subdomain>.workers.dev`), and FLAGSHIP_API_BASE (default
https://api.cloudflare.com/client/v4; https only, or http to 127.0.0.1 for the tests' fake).
Exit 2: a usage error, a missing or malformed variable, an unreadable policy, or an API call that failed.
"""

import argparse
import datetime
import json
import os
import re
import sys
import time
import tomllib
import urllib.error
import urllib.parse
import urllib.request

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
POLICY = os.path.join(ROOT, ".github", "deploy", "flagship", "policy.json")
EDGE_CONFIG = os.path.join(ROOT, "crates", "edge", "wrangler.deploy.toml")
API = "https://api.cloudflare.com/client/v4"

ROUTING = "routing"
PRODUCTION = "prod"
# The principal kinds the edge writes into a pin (crates/edge/src/pin.rs, Kind::as_str).
PRINCIPAL_KINDS = ("ci", "human")
DATA = ("isolated", "shared")
VALUE_FIELDS = ("data", "deployment", "expires", "kind", "release", "url")
# The edge's grammars (crates/edge/src/pin.rs valid_name and valid_audience, route.rs dns_label and parse_target).
NAME = re.compile(r"^[a-z0-9][a-z0-9.-]{0,56}$")
AUDIENCE = re.compile(r"^[a-z]{1,16}$")
LABEL = re.compile(r"^[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?$")
DEPLOYMENT = re.compile(r"^[a-z0-9-]{1,64}$")
SUFFIX = re.compile(r"^-[a-z0-9.-]+$")
RELEASE = re.compile(r"^[A-Za-z0-9._+-]{1,64}$")
EXPIRES = re.compile(r"^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z$")
TARGET_KIND = re.compile(r"^[a-z]{1,16}$")
ID = re.compile(r"^[A-Za-z0-9-]{1,64}$")

DESCRIPTION = {
    "routing": "The kill switch for every target. Made by CI from .github/deploy/flagship/policy.json; do not edit here.",
    "alias": "The target an alias names. Written by CI (scripts/flagship.py alias); do not edit here.",
    "target": "A live target. Written by its deploy and removed by its teardown (scripts/flagship.py); do not edit here.",
}


class Stop(Exception):
    """Exit 2 with a message."""


def say(message):
    print(f"flagship: {message}")


# ---------------------------------------------------------------- the policy


def key_of(prefix, name):
    return f"{prefix}-{name.replace('.', '_')}"


def edge_audiences(path):
    try:
        with open(path, "rb") as f:
            config = tomllib.load(f)
    except (OSError, tomllib.TOMLDecodeError) as e:
        raise Stop(f"cannot read {path}: {e}")
    return {k[len("ACCESS_AUD_"):].lower() for k in config.get("vars", {}) if k.startswith("ACCESS_AUD_")}


def policy_problems(policy, audiences_of_edge):
    found = []
    if not isinstance(policy, dict):
        return ["the policy is not a JSON object"]
    extra = set(policy) - {"$comment", "routing", "audiences", "aliases", "targets"}
    missing = {"routing", "audiences", "aliases", "targets"} - set(policy)
    found += [f"unknown key {k!r}" for k in sorted(extra)] + [f"missing key {k!r}" for k in sorted(missing)]
    if missing:
        return found
    if "$comment" in policy and not isinstance(policy["$comment"], str):
        found.append("$comment is not a string")
    if "@" in json.dumps(policy):
        found.append("the policy contains '@': it names a person, and the repository is public")
    if not isinstance(policy["routing"], bool):
        found.append("routing is not true or false")

    def names(field, pattern, what):
        value = policy[field]
        if not isinstance(value, list) or not value:
            found.append(f"{field} is not a non-empty list")
            return []
        for v in value:
            if not isinstance(v, str) or not pattern.match(v):
                found.append(f"{field}: {v!r} is not {what}")
        if len(set(map(str, value))) != len(value):
            found.append(f"{field} lists a name twice")
        return [v for v in value if isinstance(v, str)]

    audiences = names("audiences", AUDIENCE, "an audience (1 to 16 lowercase letters)")
    names("aliases", NAME, "a target name")
    if audiences_of_edge is not None and set(audiences) != audiences_of_edge:
        found.append(
            f"audiences are {sorted(audiences)} and the edge's are {sorted(audiences_of_edge)} "
            "(its ACCESS_AUD_<AUDIENCE> variables): they must be the same"
        )
    targets = policy["targets"]
    if not isinstance(targets, dict) or not targets:
        found.append("targets is not a non-empty object")
        return found
    for kind, rules in targets.items():
        if not TARGET_KIND.match(kind):
            found.append(f"targets: {kind!r} is not a target kind (1 to 16 lowercase letters)")
        if not isinstance(rules, list) or not rules:
            found.append(f"targets.{kind} is not a non-empty list of rules")
            continue
        for i, rule in enumerate(rules):
            where = f"targets.{kind}[{i}]"
            if not isinstance(rule, dict) or set(rule) != {"kinds", "audiences"}:
                found.append(f"{where} must have exactly kinds and audiences")
                continue
            for field, allowed in (("kinds", PRINCIPAL_KINDS), ("audiences", audiences)):
                value = rule[field]
                if not isinstance(value, list) or not value or len(set(map(str, value))) != len(value):
                    found.append(f"{where}.{field} is not a non-empty list without repeats")
                    continue
                for v in value:
                    if v not in allowed:
                        found.append(f"{where}.{field}: {v!r} is not one of {list(allowed)}")
    return found


def load_policy(path, edge_config):
    try:
        with open(path) as f:
            policy = json.load(f)
    except (OSError, json.JSONDecodeError) as e:
        raise Stop(f"cannot read {path}: {e}")
    found = policy_problems(policy, edge_audiences(edge_config))
    if found:
        raise Stop(f"{path} is not a valid policy (run validate): {found[0]}")
    return policy


def allowed(policy, target_kind, principal_kind, audience):
    return any(principal_kind in r["kinds"] and audience in r["audiences"] for r in policy["targets"][target_kind])


# ---------------------------------------------------------------- the flags the policy makes


def routing_flag(policy):
    return {
        "key": ROUTING,
        "description": DESCRIPTION["routing"],
        "enabled": True,
        "default_variation": "on" if policy["routing"] else "off",
        "variations": {"off": False, "on": True},
        "rules": [],
    }


def alias_flag(alias, target):
    return {
        "key": key_of("alias", alias),
        "description": DESCRIPTION["alias"],
        "enabled": True,
        "default_variation": "target",
        "variations": {"target": target},
        "rules": [],
    }


def target_flag(policy, name, value):
    rules = [
        {
            "priority": i + 1,
            "conditions": [
                {"attribute": "kind", "operator": "in", "value": list(r["kinds"])},
                {"attribute": "audience", "operator": "in", "value": list(r["audiences"])},
            ],
            "serve_variation": "allowed",
        }
        for i, r in enumerate(policy["targets"][value["kind"]])
    ]
    return {
        "key": key_of("target", name),
        "description": DESCRIPTION["target"],
        "enabled": True,
        "default_variation": "denied",
        "variations": {"denied": {}, "allowed": value},
        "rules": rules,
    }


def value_problems(policy, value, suffix):
    """Why the edge would refuse this target, or the policy has no rules for it (empty when it is a target)."""
    if not isinstance(value, dict):
        return ["the value is not an object"]
    found = [f"unknown field {k!r}" for k in sorted(set(value) - set(VALUE_FIELDS))]
    found += [f"missing field {k!r}" for k in VALUE_FIELDS if k not in value]
    if found:
        return found
    if not all(isinstance(value[k], str) for k in VALUE_FIELDS):
        return ["every field must be a string"]
    label = value["url"][len("https://"):-len(suffix)] if value["url"].endswith(suffix) else ""
    if not value["url"].startswith("https://") or not LABEL.match(label) or value["url"] != f"https://{label}{suffix}":
        found.append(f"url {value['url']!r} is not https://<label>{suffix}, a Preview of the production Worker")
    if not DEPLOYMENT.match(value["deployment"]):
        found.append(f"deployment {value['deployment']!r} is not a deployment id")
    if value["kind"] not in policy["targets"]:
        found.append(f"kind {value['kind']!r} is not one of the policy's targets {sorted(policy['targets'])}")
    if value["data"] not in DATA:
        found.append(f"data {value['data']!r} is not one of {list(DATA)}")
    if not RELEASE.match(value["release"]):
        found.append(f"release {value['release']!r} is not 1 to 64 letters, digits, '.', '_', '+' or '-'")
    try:
        ok = EXPIRES.match(value["expires"]) and datetime.datetime.strptime(value["expires"], "%Y-%m-%dT%H:%M:%SZ")
    except ValueError:
        ok = False
    if not ok:
        found.append(f"expires {value['expires']!r} is not a UTC time, YYYY-MM-DDTHH:MM:SSZ")
    return found


# ---------------------------------------------------------------- the API


class Api:
    def __init__(self, env):
        missing = [v for v in ("FLAGSHIP_API_TOKEN", "FLAGSHIP_ACCOUNT_ID", "FLAGSHIP_APP_ID", "PREVIEW_HOST_SUFFIX") if not env.get(v)]
        if missing:
            raise Stop(f"missing environment variable(s): {', '.join(missing)}")
        for v in ("FLAGSHIP_ACCOUNT_ID", "FLAGSHIP_APP_ID"):
            if not ID.match(env[v]):
                raise Stop(f"{v} is not an id (letters, digits and '-')")
        self.suffix = env["PREVIEW_HOST_SUFFIX"]
        if not SUFFIX.match(self.suffix):
            raise Stop("PREVIEW_HOST_SUFFIX is not -<worker>.<subdomain>.workers.dev")
        base = env.get("FLAGSHIP_API_BASE") or API
        parts = urllib.parse.urlsplit(base)
        if not (parts.scheme == "https" or (parts.scheme == "http" and parts.hostname == "127.0.0.1")):
            raise Stop("FLAGSHIP_API_BASE must be https (or http to 127.0.0.1, for the tests)")
        self.token = env["FLAGSHIP_API_TOKEN"]
        self.prefix = f"{base.rstrip('/')}/accounts/{env['FLAGSHIP_ACCOUNT_ID']}/flagship/apps/{env['FLAGSHIP_APP_ID']}"

    def call(self, method, path, body=None):
        data = None if body is None else json.dumps(body).encode()
        req = urllib.request.Request(self.prefix + path, data=data, method=method)
        req.add_header("Authorization", f"Bearer {self.token}")
        if data is not None:
            req.add_header("Content-Type", "application/json")
        try:
            with urllib.request.urlopen(req, timeout=30) as r:
                status, raw = r.status, r.read()
        except urllib.error.HTTPError as e:
            status, raw = e.code, e.read()
        except (urllib.error.URLError, OSError) as e:
            raise Stop(f"{method} {path}: {getattr(e, 'reason', e)}")
        try:
            parsed = json.loads(raw) if raw else None
        except json.JSONDecodeError:
            parsed = None
        return status, parsed

    def expect(self, method, path, body=None, ok=(200,)):
        status, parsed = self.call(method, path, body)
        if status not in ok:
            errors = parsed.get("errors") if isinstance(parsed, dict) else None
            raise Stop(f"{method} {path}: HTTP {status} {json.dumps(errors) if errors else ''}".rstrip())
        return status, parsed

    def flags(self):
        out, cursor = [], None
        for _ in range(100):
            query = "?limit=200" + (f"&cursor={urllib.parse.quote(cursor, safe='')}" if cursor else "")
            _, page = self.expect("GET", f"/flags{query}")
            if not isinstance(page, dict) or not isinstance(page.get("result"), list):
                raise Stop("GET /flags: the answer has no list of flags")
            out += page["result"]
            cursor = (page.get("result_info") or {}).get("cursor")
            if not cursor:
                return out
        raise Stop("GET /flags: more than 100 pages")

    def get(self, key):
        status, parsed = self.expect("GET", f"/flags/{key}", ok=(200, 404))
        return parsed.get("result") if status == 200 and isinstance(parsed, dict) else None

    def write(self, flag):
        if self.get(flag["key"]) is None:
            self.expect("POST", "/flags", flag)
        else:
            self.expect("PUT", f"/flags/{flag['key']}", flag)

    def delete(self, key):
        return self.expect("DELETE", f"/flags/{key}", ok=(200, 404))[0] == 200

    def evaluate(self, key, context):
        _, parsed = self.expect("POST", "/evaluate", {"flagKey": key, "context": context})
        result = parsed.get("result", parsed) if isinstance(parsed, dict) and "variant" not in parsed else parsed
        if not isinstance(result, dict) or "variant" not in result:
            raise Stop(f"POST /evaluate {key}: the answer names no variant")
        return result["variant"], result.get("value")


# ---------------------------------------------------------------- comparing


def managed(flag):
    """The part of a flag the policy decides, in a form that compares equal when it means the same."""
    out = {k: flag.get(k) for k in ("key", "description", "enabled", "default_variation", "variations")}
    rules = flag.get("rules")
    out["rules"] = sorted(rules, key=lambda r: r.get("priority", 0)) if isinstance(rules, list) else rules
    if flag.get("metadata"):
        out["metadata"] = flag["metadata"]
    return out


def classify(key, policy):
    if key == ROUTING:
        return "routing", None
    for prefix, kind, known in (("alias-", "alias", policy["aliases"]), ("target-", "target", None)):
        if key.startswith(prefix):
            name = key[len(prefix):].replace("_", ".")
            if NAME.match(name) and key_of(prefix[:-1], name) == key and (known is None or name in known):
                return kind, name
    return "unknown", None


def review(policy, flags, suffix):
    """(findings, fixes): each finding a line; each fix ("write", flag) or ("delete", key)."""
    findings, fixes = [], []
    live = {f.get("key"): f for f in flags}
    targets = {classify(k, policy)[1] for k in live if isinstance(k, str) and classify(k, policy)[0] == "target"}
    if ROUTING not in live:
        findings.append(f"{ROUTING}: missing")
        fixes.append(("write", routing_flag(policy)))
    for key in sorted(live, key=str):
        flag = live[key]
        if not isinstance(key, str):
            findings.append(f"a flag without a key: {json.dumps(flag)[:200]}")
            continue
        kind, name = classify(key, policy)
        if kind == "unknown":
            findings.append(f"{key}: not a flag the policy makes (only {ROUTING}, alias-<{'|'.join(policy['aliases'])}>, target-<name>)")
            fixes.append(("delete", key))
            continue
        variations = flag.get("variations") if isinstance(flag.get("variations"), dict) else {}
        if kind == "routing":
            want = routing_flag(policy)
        elif kind == "alias":
            target = variations.get(flag.get("default_variation"))
            if not (isinstance(target, str) and (target == PRODUCTION or target in targets)):
                findings.append(f"{key}: names {target!r}, which is neither {PRODUCTION!r} nor a live target")
                continue
            want = alias_flag(name, target)
        else:
            value = variations.get("allowed")
            bad = value_problems(policy, value, suffix)
            if bad:
                findings.append(f"{key}: the target is invalid: {'; '.join(bad)}")
                continue
            want = target_flag(policy, name, value)
        have = managed(flag)
        if have != want:
            for field in sorted(set(have) | set(want)):
                if have.get(field) != want.get(field):
                    findings.append(f"{key}: {field} is {json.dumps(have.get(field))}, the policy makes it {json.dumps(want.get(field))}")
            fixes.append(("write", want))
    return findings, fixes


def evaluations(policy, flags):
    """(key, context, expected variant) for every flag the policy makes and every principal kind and audience."""
    out = []
    contexts = [{}] + [{"kind": k, "audience": a} for k in PRINCIPAL_KINDS for a in policy["audiences"] + ["none"]]
    for flag in flags:
        if not isinstance(flag.get("key"), str):
            continue
        kind, _ = classify(flag["key"], policy)
        if kind == "routing":
            out.append((ROUTING, {}, "on" if policy["routing"] else "off"))
        elif kind == "alias":
            out.append((flag["key"], {}, "target"))
        elif kind == "target":
            value = (flag.get("variations") or {}).get("allowed") or {}
            if value.get("kind") in policy["targets"]:
                for c in contexts:
                    ok = c and allowed(policy, value["kind"], c["kind"], c["audience"])
                    out.append((flag["key"], c, "allowed" if ok else "denied"))
    return out


# ---------------------------------------------------------------- commands


def cmd_check(policy, api, evaluate, retries):
    flags = api.flags()
    findings, _ = review(policy, flags, api.suffix)
    if evaluate:
        pending = evaluations(policy, flags)
        wait = float(os.environ.get("FLAGSHIP_RETRY_SECONDS", "5"))
        for attempt in range(retries + 1):
            missed = []
            for key, context, want in pending:
                got, _ = api.evaluate(key, context)
                if got != want:
                    missed.append((key, context, want, got))
            if not missed or attempt == retries:
                break
            pending = [(k, c, w) for k, c, w, _ in missed]
            time.sleep(wait)
        for key, context, want, got in missed:
            findings.append(f"{key}: serves {got!r} for {json.dumps(context)}, the policy says {want!r}")
    for f in findings:
        say(f)
    say(f"{len(flags)} flags, {len(findings)} findings" + (" (evaluated)" if evaluate else ""))
    return 1 if findings else 0


def cmd_sync(policy, api):
    findings, fixes = review(policy, api.flags(), api.suffix)
    for f in findings:
        say(f)
    for action, what in fixes:
        if action == "write":
            api.write(what)
            say(f"{what['key']}: written")
        else:
            api.delete(what)
            say(f"{what}: deleted")
    left, _ = review(policy, api.flags(), api.suffix)
    for f in left:
        say(f"left: {f}")
    say(f"{len(fixes)} fixed, {len(left)} left")
    return 1 if left else 0


def verify(api, flag):
    got = api.get(flag["key"])
    if got is None or managed(got) != flag:
        say(f"{flag['key']}: read back {json.dumps(managed(got)) if got else 'nothing'}, not what was written")
        return 1
    say(f"{flag['key']}: written and read back")
    return 0


def cmd_put(policy, api, name, path):
    if not NAME.match(name):
        raise Stop(f"{name!r} is not a target name")
    try:
        with open(path) as f:
            value = json.load(f)
    except (OSError, json.JSONDecodeError) as e:
        raise Stop(f"cannot read {path}: {e}")
    bad = value_problems(policy, value, api.suffix)
    if bad:
        for b in bad:
            say(f"{key_of('target', name)}: {b}")
        return 1
    flag = target_flag(policy, name, value)
    api.write(flag)
    return verify(api, flag)


def cmd_alias(policy, api, alias, target):
    if alias not in policy["aliases"]:
        raise Stop(f"{alias!r} is not one of the policy's aliases {policy['aliases']}")
    if target != PRODUCTION and (not NAME.match(target) or api.get(key_of("target", target)) is None):
        say(f"{key_of('alias', alias)}: {target!r} is neither {PRODUCTION!r} nor a live target")
        return 1
    flag = alias_flag(alias, target)
    api.write(flag)
    return verify(api, flag)


def cmd_delete(api, name):
    if not NAME.match(name):
        raise Stop(f"{name!r} is not a target name")
    key = key_of("target", name)
    say(f"{key}: {'deleted' if api.delete(key) else 'already absent'}")
    if api.get(key) is not None:
        say(f"{key}: still there after the delete")
        return 1
    return 0


def main(argv):
    parser = argparse.ArgumentParser(prog="flagship.py")
    parser.add_argument("--policy", default=POLICY)
    parser.add_argument("--edge-config", default=EDGE_CONFIG)
    sub = parser.add_subparsers(dest="command", required=True)
    sub.add_parser("validate")
    check = sub.add_parser("check")
    check.add_argument("--evaluate", action="store_true")
    check.add_argument("--retries", type=int, default=0)
    sub.add_parser("sync")
    put = sub.add_parser("put")
    put.add_argument("target")
    put.add_argument("value")
    alias = sub.add_parser("alias")
    alias.add_argument("alias")
    alias.add_argument("target")
    delete = sub.add_parser("delete")
    delete.add_argument("target")
    args = parser.parse_args(argv[1:])
    try:
        if args.command == "validate":
            try:
                with open(args.policy) as f:
                    policy = json.load(f)
            except (OSError, json.JSONDecodeError) as e:
                raise Stop(f"cannot read {args.policy}: {e}")
            found = policy_problems(policy, edge_audiences(args.edge_config))
            for f in found:
                say(f"{args.policy}: {f}")
            if not found:
                say(f"{args.policy}: valid ({len(policy['targets'])} target kinds, audiences {policy['audiences']})")
            return 1 if found else 0
        policy = load_policy(args.policy, args.edge_config)
        api = Api(os.environ)
        if args.command == "check":
            return cmd_check(policy, api, args.evaluate, max(0, args.retries))
        if args.command == "sync":
            return cmd_sync(policy, api)
        if args.command == "put":
            return cmd_put(policy, api, args.target, args.value)
        if args.command == "alias":
            return cmd_alias(policy, api, args.alias, args.target)
        return cmd_delete(api, args.target)
    except Stop as e:
        print(f"flagship: {e}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv))
