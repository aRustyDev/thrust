#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Record the compat-slice fixtures from the pinned Plane (oracle/v1-pin-20261018, ba85d4d3d3).

The oracle stack is the one from the Plane fork (oracle/compose.oracle.yml, API on 127.0.0.1:18080); this script does
not start it. It drives the journey's requests in order, normalises each exchange with the rules shared with the Rust
harness (crates/thrust-compat-plane/tests/parity/normalise.json) and writes one fixture per step:

    crates/thrust-compat-plane/tests/parity/fixtures/<step>.json

    uv run tools/compat-record/record.py
    uv run tools/compat-record/record.py --self-test

THE STACK MUST BE FRESH. The journey creates users and the instance admin, so a second run against the same stack is
refused with a message saying so; recreate it with `docker compose -f oracle/compose.oracle.yml --env-file
oracle/.env.oracle down -v` and then `up -d --wait`. (Two recordings on two fresh stacks are byte-identical; that is the
reproducibility check.)

The pin throttles sign-in, sign-up and email-check to 10 per minute per address, so the recorder paces itself and a full
run takes about two minutes. Raw (un-normalised) exchanges go to --raw-dir and are never committed. Fixtures are
written, never hand-edited. The CSRF token of a form post is transport, not contract, and is not stored.
"""

import argparse
import collections
import http.cookiejar
import json
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import normalise  # noqa: E402

REPO = Path(__file__).resolve().parents[2]
PARITY = REPO / "crates/thrust-compat-plane/tests/parity"
PIN = "ba85d4d3d3"

# Synthetic identities: example.test is reserved, and the passwords exist only in the oracle's volume.
ADMIN = {
    "email": "instance-admin@example.test",
    "password": "Oracle-Admin-Passw0rd!",
    "first_name": "Instance",
    "last_name": "Admin",
    "company_name": "Oracle",
    "is_telemetry_enabled": "False",
}
JOURNEY_EMAIL = "journey@example.test"
SECOND_EMAIL = "second@example.test"
STRONG = "Journey-Sample-Pa55word-Zebra-81"
WRONG = "Wrong-Password-For-Journey-1"


class NoRedirect(urllib.request.HTTPRedirectHandler):
    """Hand 3xx back as a response: several rows ARE redirects and the oracle's answer is the 302."""

    def redirect_request(self, *a, **k):
        return None


class Pacer:
    """Keeps the number of throttled calls (sign-in, sign-up, email-check) under the pin's 10 per minute per address.

    DRF keeps the timestamps of the last window and refuses the 11th call inside 60 seconds; every call counts, the
    refused ones and the 400s included. The margin covers clock and network jitter.
    """

    LIMIT = 10
    WINDOW = 60.0
    MARGIN = 1.5

    def __init__(self):
        self.calls = collections.deque()

    def wait(self):
        now = time.monotonic()
        while self.calls and now - self.calls[0] >= self.WINDOW + self.MARGIN:
            self.calls.popleft()
        if len(self.calls) >= self.LIMIT:
            sleep = self.calls[0] + self.WINDOW + self.MARGIN - now
            print(f"  pacing: {sleep:.0f}s until the throttle window frees a slot", flush=True)
            time.sleep(max(sleep, 0))
            self.calls.popleft()
        self.calls.append(time.monotonic())


class Client:
    """One browser-like session: a cookie jar, no redirects followed, CSRF on request."""

    def __init__(self, base):
        self.base = base.rstrip("/")
        self.jar = http.cookiejar.CookieJar()
        self.opener = urllib.request.build_opener(urllib.request.HTTPCookieProcessor(self.jar), NoRedirect)

    def request(self, method, path, body=None, form=None, headers=None, hidden_form=None, as_email=None):
        """Returns the exchange as the raw dict `normalise.exchange` takes (name added by the caller).

        `hidden_form` is sent with a form post but not recorded (the CSRF token). `as_email` is recorded as the request's
        `as`: whose session this client holds, so the harness can send the request with that user's session."""
        hs = dict(headers or {})
        data = None
        if body is not None:
            data = json.dumps(body).encode()
            hs["Content-Type"] = "application/json"
        elif form is not None:
            data = urllib.parse.urlencode({**form, **(hidden_form or {})}).encode()
            hs["Content-Type"] = "application/x-www-form-urlencoded"
        req = urllib.request.Request(self.base + path, data=data, method=method, headers=hs)
        try:
            resp = self.opener.open(req, timeout=30)
        except urllib.error.HTTPError as e:  # 3xx (not followed), 4xx, 5xx all land here
            resp = e
        raw = resp.read()
        status = getattr(resp, "status", None) or resp.code
        ctype = resp.headers.get("Content-Type", "")
        parsed = None
        if raw:
            if "json" not in ctype:
                raise SystemExit(f"{method} {path}: {status} body is {ctype!r}, not JSON; extend the fixture format before recording it")
            parsed = json.loads(raw)
        request = {"method": method, "path": path}
        if as_email is not None:
            request["as"] = as_email
        if body is not None:
            request["body"] = body
        elif form is not None:
            request["body"] = {"form": form}
        return {
            "request": request,
            "response": {"status": status, "headers": {k: v for k, v in resp.headers.items()}, "body": parsed},
        }

    def csrf(self):
        """GET /auth/get-csrf-token/ (S03), keeping the csrftoken cookie; returns the token for X-CSRFToken."""
        return self.request("GET", "/auth/get-csrf-token/")["response"]["body"]["csrf_token"]


def setup_instance_admin(base):
    """Prerequisite, not a recorded step: a fresh pinned Plane answers /api/instances/ with is_setup_done=false, and the
    stock app sends the visitor to the instance-admin setup instead of the sign-up page. Doing the setup the way the
    admin app does (a form post, answered by a 302) is what flips the flag."""
    c = Client(base)
    token = c.csrf()
    ex = c.request("POST", "/api/instances/admins/sign-up/", form=ADMIN, headers={"X-CSRFToken": token}, hidden_form={"csrfmiddlewaretoken": token})
    status = ex["response"]["status"]
    # A rejected sign-up is also a 302, with ?error_code= in Location.
    location = ex["response"]["headers"].get("Location", "")
    if status != 302 or "error_code" in location:
        raise SystemExit(f"instance admin sign-up was refused ({status} {location}): is the stack fresh? See --help")


def require_fresh_stack(base, pacer):
    """The journey creates users, so a stack that has been used is refused rather than recorded wrongly."""
    c = Client(base)
    setup_done = c.request("GET", "/api/instances/")["response"]["body"]["instance"]["is_setup_done"]
    if setup_done:
        raise SystemExit("the instance is already set up: this stack has been recorded against. Recreate it (see --help).")
    setup_instance_admin(base)
    pacer.wait()
    token = c.csrf()
    existing = c.request("POST", "/auth/email-check/", body={"email": JOURNEY_EMAIL}, headers={"X-CSRFToken": token})
    if existing["response"]["body"].get("existing"):
        raise SystemExit(f"{JOURNEY_EMAIL} exists: this stack has been recorded against. Recreate it (see --help).")


# ---------------------------------------------------------------------------------------------------------------------
# The steps, in journey order. A step is a list of named exchanges; each one that signs in or up uses a fresh browser
# session, so no step depends on cookies. A step may depend on users the earlier steps made.
# ---------------------------------------------------------------------------------------------------------------------


def fresh(base):
    return Client(base)


def post_json(base, pacer, path, body):
    c = fresh(base)
    token = c.csrf()
    pacer.wait()
    return c.request("POST", path, body=body, headers={"X-CSRFToken": token})


def post_form(base, pacer, path, form):
    """A form post as the browser makes it (the pin's sign-in and sign-up pages): CSRF token in the header and in the
    form, answered by a 302."""
    c = fresh(base)
    token = c.csrf()
    pacer.wait()
    return c.request("POST", path, form=form, headers={"X-CSRFToken": token}, hidden_form={"csrfmiddlewaretoken": token})


PASSWORDS = {JOURNEY_EMAIL: STRONG, SECOND_EMAIL: STRONG}
_SESSIONS = {}


def session(base, pacer, email):
    """A signed-in client for `email`, signed in once (by the same form post the sign-in page makes) and kept: a browser
    keeps its session. The pin's API authenticates by session cookie and does not check CSRF on it."""
    if email not in _SESSIONS:
        c = Client(base)
        token = c.csrf()
        pacer.wait()
        ex = c.request("POST", "/auth/sign-in/", form={"email": email, "password": PASSWORDS[email]}, headers={"X-CSRFToken": token}, hidden_form={"csrfmiddlewaretoken": token})
        location = ex["response"]["headers"].get("Location", "")
        if ex["response"]["status"] != 302 or "error_code" in location:
            raise SystemExit(f"could not sign {email} in to record a signed-in step: {location}")
        _SESSIONS[email] = c
    return _SESSIONS[email]


def api(base, pacer, email, method, path, body=None):
    """One API call, as `email` (a signed-in session) or, for `None`, with no session."""
    c = session(base, pacer, email) if email else fresh(base)
    return c.request(method, path, body=body, as_email=email)


def step_s01(base, pacer):
    """GET /api/instances/ : the boot request. Recorded with no session, after the instance admin exists."""
    return [("boot, no session, instance set up", fresh(base).request("GET", "/api/instances/"))]


def step_s02(base, pacer):
    """GET /api/users/me/ without a session. The pin answers 401 with a DRF `detail` body: a JSON object with no `id`,
    which is how the React app learns it is signed out (research/0003 section 9, row 3)."""
    return [("no session", fresh(base).request("GET", "/api/users/me/"))]


def step_s03(base, pacer):
    """GET /auth/get-csrf-token/ : the token the sign-in and sign-up forms carry."""
    return [("a token, no session", fresh(base).request("GET", "/auth/get-csrf-token/"))]


def step_s04(base, pacer):
    """POST /auth/email-check/ before anyone has signed up: what the first page of the sign-in flow asks."""
    return [
        ("an email nobody has", post_json(base, pacer, "/auth/email-check/", {"email": JOURNEY_EMAIL})),
        ("no email", post_json(base, pacer, "/auth/email-check/", {})),
        ("not an email", post_json(base, pacer, "/auth/email-check/", {"email": "not-an-email"})),
    ]


def step_s05(base, pacer):
    """POST /auth/sign-up/ : every refusal first, then the sign-up that works, then the same email again."""
    up = "/auth/sign-up/"
    return [
        ("no fields", post_form(base, pacer, up, {})),
        ("empty fields", post_form(base, pacer, up, {"email": "", "password": ""})),
        # one field of the two: "required" is either missing, and the payload's email is the one given or "False"
        ("an email and no password", post_form(base, pacer, up, {"email": JOURNEY_EMAIL})),
        ("a password and no email", post_form(base, pacer, up, {"password": STRONG})),
        ("not an email", post_form(base, pacer, up, {"email": "nope", "password": STRONG})),
        ("weak password", post_form(base, pacer, up, {"email": JOURNEY_EMAIL, "password": "password"})),
        ("sign up", post_form(base, pacer, up, {"email": JOURNEY_EMAIL, "password": STRONG})),
        ("the same email again", post_form(base, pacer, up, {"email": JOURNEY_EMAIL, "password": STRONG})),
        # two faults at once pin the order the pin checks them in: the email's shape, then whether it is taken, then
        # the password's strength
        ("taken email and a weak password", post_form(base, pacer, up, {"email": JOURNEY_EMAIL, "password": "password"})),
        ("not an email and a weak password", post_form(base, pacer, up, {"email": "nope", "password": "password"})),
        ("sign up with a next_path", post_form(base, pacer, up, {"email": SECOND_EMAIL, "password": STRONG, "next_path": "/dashboard"})),
    ]


def step_s04_existing(base, pacer):
    """POST /auth/email-check/ again, now that the journey user exists (a later step of the same row: its answer
    depends on S05, so it is replayed after it)."""
    return [("an email somebody has", post_json(base, pacer, "/auth/email-check/", {"email": JOURNEY_EMAIL}))]


def step_s06(base, pacer):
    """POST /auth/sign-in/ : every refusal first, then the sign-ins that work."""
    inn = "/auth/sign-in/"
    return [
        ("no fields", post_form(base, pacer, inn, {})),
        ("an email and no password", post_form(base, pacer, inn, {"email": JOURNEY_EMAIL})),
        ("a password and no email", post_form(base, pacer, inn, {"password": STRONG})),
        ("not an email", post_form(base, pacer, inn, {"email": "nope", "password": STRONG})),
        ("an email nobody has", post_form(base, pacer, inn, {"email": "nobody@example.test", "password": STRONG})),
        ("wrong password", post_form(base, pacer, inn, {"email": JOURNEY_EMAIL, "password": WRONG})),
        ("sign in", post_form(base, pacer, inn, {"email": JOURNEY_EMAIL, "password": STRONG})),
        ("sign in with a next_path", post_form(base, pacer, inn, {"email": JOURNEY_EMAIL, "password": STRONG, "next_path": "/dashboard"})),
        ("an email in another case", post_form(base, pacer, inn, {"email": "Journey@Example.TEST", "password": STRONG})),
    ]


J = JOURNEY_EMAIL
NOT_SIGNED_IN = None
ME = "/api/users/me/"
PROFILE = "/api/users/me/profile/"


def step_s02_signed_in(base, pacer):
    """GET /api/users/me/ signed in: the user a sign-up made, and (to pin that it is the caller's own) another one."""
    return [
        ("a new user", api(base, pacer, J, "GET", ME)),
        ("another user", api(base, pacer, SECOND_EMAIL, "GET", ME)),
    ]


def step_s07(base, pacer):
    """GET /api/users/me/profile/ : a new user's profile, with every default."""
    return [
        ("no session", api(base, pacer, NOT_SIGNED_IN, "GET", PROFILE)),
        ("a new user's profile", api(base, pacer, J, "GET", PROFILE)),
    ]


def step_s10(base, pacer):
    """PATCH /api/users/me/ : the onboarding profile step, the preferences, and what the pin ignores or refuses."""
    patch = lambda who, body: api(base, pacer, who, "PATCH", ME, body)  # noqa: E731
    return [
        ("no session", patch(NOT_SIGNED_IN, {"first_name": "Nobody"})),
        ("the names the onboarding page sends", patch(J, {"first_name": "Journey", "last_name": "Tester"})),
        ("a first name with space around it", patch(J, {"first_name": "  Journey  "})),
        ("a first name that is not a string", patch(J, {"first_name": ["a"]})),
        ("a first name that is null", patch(J, {"first_name": None})),
        ("a first name that is too long", patch(J, {"first_name": "x" * 256})),
        ("a URL in the first name", patch(J, {"first_name": "see example.com"})),
        ("a URL in the last name", patch(J, {"last_name": "http://example.test/x"})),
        ("a blank display name", patch(J, {"display_name": ""})),
        ("a time zone that does not exist", patch(J, {"user_timezone": "Mars/Olympus"})),
        ("a time zone", patch(J, {"user_timezone": "Asia/Kolkata"})),
        ("fields that are read only are ignored", patch(J, {"email": "changed@example.test", "is_bot": True, "username": "changed", "avatar_url": "https://example.test/a.png"})),
        ("an empty body", patch(J, {})),
        ("a cover image that is not a URL", patch(J, {"cover_image": "nope"})),
        ("a cover image", patch(J, {"cover_image": "https://example.test/cover.png"})),
        ("a cover image cleared", patch(J, {"cover_image": None})),
        ("a display name", patch(J, {"display_name": "Journey T."})),
        ("back to UTC", patch(J, {"user_timezone": "UTC"})),
        ("the user now", api(base, pacer, J, "GET", ME)),
    ]


def step_s11(base, pacer):
    """PATCH /api/users/me/profile/ : what the onboarding steps and the settings pages send, and what the pin refuses."""
    patch = lambda who, body: api(base, pacer, who, "PATCH", PROFILE, body)  # noqa: E731
    steps = {"profile_complete": True, "workspace_create": False, "workspace_invite": False, "workspace_join": False}
    return [
        ("no session", patch(NOT_SIGNED_IN, {"role": "developer"})),
        ("marketing consent", patch(J, {"has_marketing_email_consent": False})),
        ("a role", patch(J, {"role": "developer"})),
        ("a use case", patch(J, {"use_case": "Plan work. Track bugs"})),
        ("an onboarding step", patch(J, {"onboarding_step": steps})),
        ("a theme", patch(J, {"theme": {"theme": "dark", "primary": "#3f76ff", "background": "#1a1a1a", "darkPalette": True}})),
        ("the start of the week", patch(J, {"start_of_the_week": 1})),
        ("a start of the week out of range", patch(J, {"start_of_the_week": 9})),
        ("a notification view that does not exist", patch(J, {"notification_view_mode": "huge"})),
        ("a last workspace that is not an id", patch(J, {"last_workspace_id": "not-a-uuid"})),
        ("a role that is too long", patch(J, {"role": "x" * 301})),
        ("a boolean that is not one", patch(J, {"is_onboarded": "maybe"})),
        ("a boolean that is null", patch(J, {"is_tour_completed": None})),
        ("a boolean spelled yes", patch(J, {"has_marketing_email_consent": "yes"})),
        ("a number for the role", patch(J, {"role": 5})),
        ("a theme that is null", patch(J, {"theme": None})),
        ("a language that is blank", patch(J, {"language": ""})),
        ("the start of the week as text", patch(J, {"start_of_the_week": "2"})),
        ("an unknown field", patch(J, {"nonsense": 1})),
        ("fields that are read only are ignored", patch(J, {"user": "00000000-0000-4000-8000-000000000001", "id": "00000000-0000-4000-8000-000000000002"})),
        ("an empty body", patch(J, {})),
        ("the profile now", api(base, pacer, J, "GET", PROFILE)),
    ]


def step_s27(base, pacer):
    """PATCH /api/users/me/tour-completed/ : the home page's tour overlay."""
    tour = "/api/users/me/tour-completed/"
    patch = lambda who, body: api(base, pacer, who, "PATCH", tour, body)  # noqa: E731
    return [
        ("no session", patch(NOT_SIGNED_IN, {"is_tour_completed": True})),
        ("not a boolean", patch(J, {"is_tour_completed": "abc"})),
        ("no key sets it to false", patch(J, {})),
        ("the tour is completed", patch(J, {"is_tour_completed": True})),
    ]


def step_x04_onboard(base, pacer):
    """PATCH /api/users/me/onboard/ : not one of the 48 rows of research/0003 section 8.1, but the last request of the
    stock onboarding (`finishUserOnboarding` awaits it, and a 404 would stop the page from finishing)."""
    onboard = "/api/users/me/onboard/"
    patch = lambda who, body: api(base, pacer, who, "PATCH", onboard, body)  # noqa: E731
    return [
        ("no session", patch(NOT_SIGNED_IN, {"is_onboarded": True})),
        ("not a boolean", patch(J, {"is_onboarded": "abc"})),
        ("no key sets it to false", patch(J, {})),
        ("onboarded", patch(J, {"is_onboarded": True})),
        ("the profile now", api(base, pacer, J, "GET", PROFILE)),
    ]


# Journey order. The file name is the step id; `S04-existing` is S04's route again, after S05, and `S02-signed-in` is
# S02's route again, once there is a session. `X04` is a route the stock onboarding calls that the inventory lacks.
STEPS = [
    ("S01", step_s01),
    ("S02", step_s02),
    ("S03", step_s03),
    ("S04", step_s04),
    ("S05", step_s05),
    ("S04-existing", step_s04_existing),
    ("S06", step_s06),
    ("S02-signed-in", step_s02_signed_in),
    ("S07", step_s07),
    ("S10", step_s10),
    ("S11", step_s11),
    ("S27", step_s27),
    ("X04", step_x04_onboard),
]


def record(base, raw_dir):
    rules = json.loads((PARITY / "normalise.json").read_text())
    out_dir = PARITY / "fixtures"
    out_dir.mkdir(parents=True, exist_ok=True)
    raw_dir.mkdir(parents=True, exist_ok=True)
    pacer = Pacer()
    require_fresh_stack(base, pacer)
    for step, run in STEPS:
        print(f"{step} ...", flush=True)
        named = run(base, pacer)
        raw = [{"name": n, **ex} for n, ex in named]
        (raw_dir / f"{step}.raw.json").write_text(json.dumps(raw, indent=2, sort_keys=True) + "\n")
        ids = normalise.Ids(rules["uuid_token"])
        fixture = {
            "format_version": 1,
            "recorded_from": PIN,
            "row": step,
            "exchanges": [normalise.exchange(rules, ids, ex) for ex in raw],
        }
        (out_dir / f"{step}.json").write_text(json.dumps(fixture, indent=2, sort_keys=True) + "\n")
        print(f"{step}: {len(raw)} exchange(s) -> fixtures/{step}.json")


def self_test():
    """Run the shared normaliser cases. A case is {name, input, expected}; `input`/`expected` are lists of exchanges."""
    rules = json.loads((PARITY / "normalise.json").read_text())
    cases = json.loads((PARITY / "normalise-cases.json").read_text())["cases"]
    failed = 0
    for case in cases:
        ids = normalise.Ids(rules["uuid_token"])
        got = [normalise.exchange(rules, ids, ex) for ex in case["input"]]
        if got != case["expected"]:
            failed += 1
            print(f"FAIL {case['name']}\n  got      {json.dumps(got, sort_keys=True)}\n  expected {json.dumps(case['expected'], sort_keys=True)}")
    if failed:
        raise SystemExit(f"{failed} of {len(cases)} normaliser case(s) failed")
    print(f"ok: {len(cases)} normaliser case(s)")


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--base-url", default="http://127.0.0.1:18080")
    ap.add_argument("--raw-dir", type=Path, default=REPO / "target/compat-record/raw")
    ap.add_argument("--self-test", action="store_true")
    args = ap.parse_args()
    if args.self_test:
        return self_test()
    record(args.base_url, args.raw_dir)


if __name__ == "__main__":
    main()
