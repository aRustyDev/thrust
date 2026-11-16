"""A stand-in for Cloudflare's Flagship API (one app), for test_flagship.py (ADR-0111, #195).

    flagship_fake.py <state.json> <portfile> <calls.log>

Binds 127.0.0.1 on a free port, writes the port to <portfile>, and serves until killed. It reads <state.json> on every request
and writes it back after every change, so the test seeds and inspects the app through that file:

    {"token": "...", "account": "...", "app": "...", "flags": {"<key>": <flag>}, "modes": {...}}

What it implements, after the published schema (/client/v4/accounts/{account_id}/flagship/apps/{app_id}/...): list flags (limit 1-200,
an opaque cursor, result_info.cursor null on the last page), get, create (409 when the key exists), update with PUT (404 when
it does not; metadata omitted is cleared), delete, and evaluate with a JSON context. A body is checked as the schema says
(required and allowed fields, the key's pattern, default_variation and every serve_variation naming a variation, one type for
every variation, unique priorities, the operators, clauses of AND/OR) and refused with 400 otherwise. A request without the
bearer token is 401. Every request is one line of <calls.log>: `<METHOD> <path>`.

Evaluation follows the documentation: a disabled flag serves its default; otherwise the rules in ascending priority, the first
whose conditions all match (an empty list matches everything); a condition on an attribute the context lacks does not match.

"modes" switches, for the failures the script has to notice:
  page: <n>          list at most n flags a page, whatever limit asks
  put_noop: true     PUT answers 200 and changes nothing
  eval_or: true      a rule matches when ANY of its conditions does (a service whose semantics are not the documented ones)
  eval_lag: <n>      the first n evaluations answer the flag's default variation, as during propagation (counted in "evals")
  eval_envelope: true  evaluate answers in Cloudflare's envelope {success, result: {...}} instead of the bare result
"""

import http.server
import json
import os
import re
import sys
import urllib.parse

STATE, PORTFILE, CALLS = sys.argv[1], sys.argv[2], sys.argv[3]
KEY = re.compile(r"^[a-zA-Z0-9_-]{1,64}$")
OPERATORS = {"equals", "not_equals", "greater_than", "less_than", "greater_than_or_equals", "less_than_or_equals",
             "contains", "starts_with", "ends_with", "in", "not_in", "has", "not_has"}
FLAG_FIELDS = {"key", "description", "enabled", "default_variation", "variations", "rules", "metadata", "type"}
RULE_FIELDS = {"priority", "conditions", "serve_variation", "rollout"}


def load():
    with open(STATE) as f:
        return json.load(f)


def save(state):
    with open(STATE, "w") as f:
        json.dump(state, f, indent=1, sort_keys=True)


def type_of(v):
    if isinstance(v, bool):
        return "boolean"
    if isinstance(v, str):
        return "string"
    if isinstance(v, (int, float)):
        return "number"
    return "json"


def condition_errors(c, depth=1):
    if not isinstance(c, dict):
        return ["a condition is not an object"]
    if "logical_operator" in c or "clauses" in c:
        if set(c) != {"logical_operator", "clauses"} or c["logical_operator"] not in ("AND", "OR"):
            return ["a group needs exactly logical_operator (AND|OR) and clauses"]
        if depth > 5 or not isinstance(c["clauses"], list) or not c["clauses"]:
            return ["a group's clauses are empty or nested too deep"]
        return [e for x in c["clauses"] for e in condition_errors(x, depth + 1)]
    if set(c) != {"attribute", "operator", "value"}:
        return ["a condition needs exactly attribute, operator and value"]
    if c["operator"] not in OPERATORS:
        return [f"unknown operator {c['operator']!r}"]
    return []


def flag_errors(body):
    if not isinstance(body, dict):
        return ["the body is not an object"]
    errors = [f"unknown field {k}" for k in set(body) - FLAG_FIELDS]
    errors += [f"missing field {k}" for k in ("key", "default_variation", "variations", "rules", "enabled") if k not in body]
    if errors:
        return errors
    if not isinstance(body["key"], str) or not KEY.match(body["key"]):
        errors.append("key does not match ^[a-zA-Z0-9_-]+$ (1-64)")
    variations = body["variations"]
    if not isinstance(variations, dict) or not variations:
        return errors + ["variations is not a non-empty object"]
    if len({type_of(v) for v in variations.values()}) != 1:
        errors.append("the variations are not all one type")
    if body["default_variation"] not in variations:
        errors.append("default_variation is not a variation")
    if not isinstance(body["enabled"], bool):
        errors.append("enabled is not a boolean")
    d = body.get("description")
    if d is not None and (not isinstance(d, str) or len(d) > 512):
        errors.append("description is not a string of at most 512")
    rules = body["rules"]
    if not isinstance(rules, list):
        return errors + ["rules is not a list"]
    priorities = []
    for r in rules:
        if not isinstance(r, dict) or set(r) - RULE_FIELDS or not {"priority", "conditions", "serve_variation"} <= set(r):
            errors.append("a rule needs priority, conditions and serve_variation, and nothing but rollout besides")
            continue
        if not isinstance(r["priority"], int) or isinstance(r["priority"], bool) or r["priority"] < 1:
            errors.append("a priority is not an integer of at least 1")
        priorities.append(r["priority"])
        if r["serve_variation"] not in variations:
            errors.append("a serve_variation is not a variation")
        if not isinstance(r["conditions"], list):
            errors.append("conditions is not a list")
        else:
            errors += [e for c in r["conditions"] for e in condition_errors(c)]
    if len(set(map(str, priorities))) != len(priorities):
        errors.append("two rules have the same priority")
    return errors


def matches(c, ctx, any_of):
    if "logical_operator" in c:
        results = [matches(x, ctx, any_of) for x in c["clauses"]]
        return all(results) if c["logical_operator"] == "AND" else any(results)
    if c["attribute"] not in ctx:
        return False
    v, op, x = ctx[c["attribute"]], c["operator"], c["value"]
    if op == "equals":
        return v == x
    if op == "not_equals":
        return v != x
    if op == "in":
        return isinstance(x, list) and v in x
    if op == "not_in":
        return isinstance(x, list) and v not in x
    if op == "has":
        return isinstance(v, list) and x in v
    if op == "not_has":
        return isinstance(v, list) and x not in v
    return False


def evaluate(flag, ctx, any_of):
    if not flag["enabled"]:
        return flag["default_variation"], "DISABLED"
    for rule in sorted(flag["rules"], key=lambda r: r["priority"]):
        conds = rule["conditions"]
        hits = [matches(c, ctx, any_of) for c in conds]
        if not conds or (any(hits) if any_of else all(hits)):
            return rule["serve_variation"], "TARGETING_MATCH"
    return flag["default_variation"], "DEFAULT"


class Handler(http.server.BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass

    def reply(self, code, body):
        raw = json.dumps(body).encode()
        self.send_response(code)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(raw)))
        self.end_headers()
        self.wfile.write(raw)

    def envelope(self, code, result=None, errors=(), result_info=None):
        body = {"success": code < 400, "errors": [{"code": code, "message": m} for m in errors], "messages": [], "result": result}
        if result_info is not None:
            body["result_info"] = result_info
        self.reply(code, body)

    def handle_any(self, method):
        url = urllib.parse.urlsplit(self.path)
        with open(CALLS, "a") as f:
            f.write(f"{method} {url.path}{'?' + url.query if url.query else ''}\n")
        state = load()
        if self.headers.get("Authorization") != f"Bearer {state['token']}":
            return self.envelope(401, errors=["Authentication error"])
        prefix = f"/client/v4/accounts/{state['account']}/flagship/apps/{state['app']}"
        if not url.path.startswith(prefix + "/"):
            return self.envelope(404, errors=["no such app"])
        rest = url.path[len(prefix):]
        body = None
        if method in ("POST", "PUT"):
            try:
                body = json.loads(self.rfile.read(int(self.headers.get("Content-Length", "0"))) or b"null")
            except json.JSONDecodeError:
                return self.envelope(400, errors=["the body is not JSON"])
        flags, modes = state["flags"], state.get("modes", {})

        if rest == "/flags" and method == "GET":
            q = urllib.parse.parse_qs(url.query)
            limit = int(q.get("limit", ["50"])[0])
            if not 1 <= limit <= 200:
                return self.envelope(400, errors=["limit must be 1 to 200"])
            size = min(limit, modes.get("page", limit))
            start = int(q["cursor"][0].removeprefix("c")) if "cursor" in q else 0
            keys = sorted(flags)
            page = [flags[k] for k in keys[start:start + size]]
            more = start + size < len(keys)
            return self.envelope(200, page, result_info={"count": len(page), "cursor": f"c{start + size}" if more else None})
        if rest == "/flags" and method == "POST":
            errors = flag_errors(body)
            if errors:
                return self.envelope(400, errors=errors)
            if body["key"] in flags:
                return self.envelope(409, errors=["a flag with this key exists"])
            flags[body["key"]] = self.stored(body)
            save(state)
            return self.envelope(200, flags[body["key"]])
        if rest == "/evaluate" and method == "POST":
            if not isinstance(body, dict) or "flagKey" not in body or set(body) - {"flagKey", "context"}:
                return self.envelope(400, errors=["the body needs flagKey, and context at most"])
            flag = flags.get(body["flagKey"])
            if flag is None:
                return self.envelope(404, errors=["flag not found"])
            state["evals"] = state.get("evals", 0) + 1
            save(state)
            if state["evals"] <= modes.get("eval_lag", 0):
                variant, reason = flag["default_variation"], "DEFAULT"
            else:
                variant, reason = evaluate(flag, body.get("context") or {}, modes.get("eval_or", False))
            result = {"flagKey": flag["key"], "variant": variant, "value": flag["variations"][variant], "reason": reason}
            return self.envelope(200, result) if modes.get("eval_envelope") else self.reply(200, result)
        m = re.match(r"^/flags/([^/]+)$", rest)
        if not m:
            return self.envelope(404, errors=["no such route"])
        key = urllib.parse.unquote(m.group(1))
        if key not in flags:
            return self.envelope(404, errors=["flag not found"])
        if method == "GET":
            return self.envelope(200, flags[key])
        if method == "DELETE":
            del flags[key]
            save(state)
            return self.envelope(200, {"key": key})
        if method == "PUT":
            errors = flag_errors(body)
            if not errors and body["key"] != key:
                errors = ["the body's key is not the path's"]
            if errors:
                return self.envelope(400, errors=errors)
            if not modes.get("put_noop"):
                flags[key] = self.stored(body)
                save(state)
            return self.envelope(200, flags[key])
        return self.envelope(405, errors=["method not allowed"])

    @staticmethod
    def stored(body):
        flag = {k: v for k, v in body.items() if k != "type"}
        flag.setdefault("description", None)
        flag["type"] = type_of(next(iter(body["variations"].values())))
        flag["updated_at"] = "2026-01-01T00:00:00Z"
        flag["updated_by"] = "fake"
        return flag

    def do_GET(self):
        self.handle_any("GET")

    def do_POST(self):
        self.handle_any("POST")

    def do_PUT(self):
        self.handle_any("PUT")

    def do_DELETE(self):
        self.handle_any("DELETE")


server = http.server.HTTPServer(("127.0.0.1", 0), Handler)
with open(PORTFILE + ".tmp", "w") as f:
    f.write(str(server.server_address[1]))
os.replace(PORTFILE + ".tmp", PORTFILE)
server.serve_forever()
