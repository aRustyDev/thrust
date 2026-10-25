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
import os
import subprocess
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


WS_ONE = "journey-ws"
WS_TWO = "journey-two"
INVITATIONS = "/api/users/me/workspaces/invitations/"
MY_WORKSPACES = "/api/users/me/workspaces/"
SETTINGS = "/api/users/me/settings/"
WORKSPACES = "/api/workspaces/"
SLUG_CHECK = "/api/workspace-slug-check/"


def step_s12(base, pacer):
    """GET /api/users/me/workspaces/invitations/ : a user nobody has invited. (Invitations are P1, F-A07: a stub.)"""
    return [
        ("no session", api(base, pacer, NOT_SIGNED_IN, "GET", INVITATIONS)),
        ("a user with no invitations", api(base, pacer, J, "GET", INVITATIONS)),
    ]


def step_s09_before(base, pacer):
    """GET /api/users/me/workspaces/ before the user has one."""
    return [
        ("no session", api(base, pacer, NOT_SIGNED_IN, "GET", MY_WORKSPACES)),
        ("a user with no workspace", api(base, pacer, J, "GET", MY_WORKSPACES)),
    ]


def step_s08_before(base, pacer):
    """GET /api/users/me/settings/ for a user with no workspace."""
    return [
        ("no session", api(base, pacer, NOT_SIGNED_IN, "GET", SETTINGS)),
        ("a user with no workspace", api(base, pacer, J, "GET", SETTINGS)),
    ]


def step_s13_before(base, pacer):
    """GET /api/workspace-slug-check/ before the workspace exists."""
    check = lambda who, slug: api(base, pacer, who, "GET", f"{SLUG_CHECK}?slug={slug}" if slug is not None else SLUG_CHECK)  # noqa: E731
    return [
        ("no session", check(NOT_SIGNED_IN, WS_ONE)),
        ("no slug", check(J, None)),
        ("an empty slug", check(J, "")),
        ("a slug nobody has", check(J, WS_ONE)),
        ("a reserved slug", check(J, "onboarding")),
        ("another reserved slug", check(J, "api")),
    ]


def step_s14(base, pacer):
    """POST /api/workspaces/ : every refusal, then two workspaces, then what depends on them and on the second one's id.

    The exchanges that follow the creation and need its id live in this one fixture, because a request that carries an
    id from an earlier response is resolved by the fixture's own ordinals."""
    create = lambda who, body: api(base, pacer, who, "POST", WORKSPACES, body)  # noqa: E731
    out = [
        ("no session", create(NOT_SIGNED_IN, {"name": "Nobody", "slug": "nobody"})),
        ("no fields", create(J, {})),
        ("no slug", create(J, {"name": "No Slug"})),
        ("no name", create(J, {"slug": "no-name"})),
        ("a name that is too long", create(J, {"name": "x" * 81, "slug": "long-name"})),
        ("a slug that is too long", create(J, {"name": "Long Slug", "slug": "s" * 49})),
        ("a URL in the name", create(J, {"name": "see example.com", "slug": "url-name"})),
        ("a name with no letter or number", create(J, {"name": "!!!", "slug": "no-alnum"})),
        ("a reserved slug", create(J, {"name": "Reserved", "slug": "onboarding"})),
        ("a slug with a space", create(J, {"name": "Spaced", "slug": "a b"})),
        ("an organization size that is too long", create(J, {"name": "Big", "slug": "big-org", "organization_size": "x" * 21})),
        ("a time zone that does not exist", create(J, {"name": "Zoned", "slug": "zoned", "timezone": "Mars/Olympus"})),
        ("a workspace", create(J, {"name": "Journey Workspace", "slug": WS_ONE, "organization_size": "Just myself"})),
        ("the same slug again", create(J, {"name": "Again", "slug": WS_ONE})),
    ]
    second = create(J, {"name": "Second Workspace", "slug": WS_TWO, "organization_size": "2-10", "company_role": "engineer"})
    out.append(("another workspace, with a company role", second))
    second_id = second["response"]["body"]["id"]
    out += [
        ("settings: workspaces, and no last workspace", api(base, pacer, J, "GET", SETTINGS)),
        ("the last workspace is set", api(base, pacer, J, "PATCH", PROFILE, {"last_workspace_id": second_id})),
    ]
    return out


INSTANCE_CACHE_KEY = ":1:/api/instances/"


def drop_instance_cache():
    """The pin caches the whole answer of `/api/instances/` for two hours (`@cache_response(60 * 60 * 2, user=False)`), and
    works out `workspaces_exist` only when it fills the cache. The first S01 request filled it, so a request right after a
    workspace is made still says `false`, for up to two hours: an artifact of the cache, not of the data. Thrust does not
    cache it and answers what is true (deviation D-025), so the recorder deletes that one key and records the answer the
    endpoint gives once the cache has expired. `ORACLE_REDIS` names the container holding the pin's Valkey."""
    container = os.environ.get("ORACLE_REDIS", "plane-oracle-plane-redis-1")
    done = subprocess.run(["docker", "exec", container, "valkey-cli", "DEL", INSTANCE_CACHE_KEY], capture_output=True, text=True)
    if done.returncode != 0 or done.stdout.strip() not in ("0", "1"):
        raise SystemExit(f"could not drop the instance cache key in {container}: {done.stdout!r} {done.stderr!r}")


def step_s01_after(base, pacer):
    """GET /api/instances/ once a workspace exists: `workspaces_exist` flips (once the pin's cache is out of the way)."""
    drop_instance_cache()
    return [("a workspace exists", fresh(base).request("GET", "/api/instances/"))]


def step_s13_after(base, pacer):
    """GET /api/workspace-slug-check/ for slugs that are taken."""
    check = lambda slug: api(base, pacer, J, "GET", f"{SLUG_CHECK}?slug={slug}")  # noqa: E731
    return [
        ("a slug that is taken", check(WS_ONE)),
        ("the other slug that is taken", check(WS_TWO)),
        ("the same slug in capitals (the check is case sensitive)", check("Journey-WS")),
    ]


def step_s09_after(base, pacer):
    """GET /api/users/me/workspaces/ with two workspaces: newest first, with the caller's role and the member count."""
    return [
        ("two workspaces", api(base, pacer, J, "GET", MY_WORKSPACES)),
        ("another user", api(base, pacer, SECOND_EMAIL, "GET", MY_WORKSPACES)),
    ]


def step_s08_after(base, pacer):
    """GET /api/users/me/settings/ once the user has a last workspace."""
    return [
        ("a last workspace", api(base, pacer, J, "GET", SETTINGS)),
        ("another user, with no workspace", api(base, pacer, SECOND_EMAIL, "GET", SETTINGS)),
    ]


def step_s15(base, pacer):
    """GET .../workspace-members/me/ : the caller's own membership, and what the pin answers a non-member."""
    me = lambda who, slug: api(base, pacer, who, "GET", f"/api/workspaces/{slug}/workspace-members/me/")  # noqa: E731
    return [
        ("no session", me(NOT_SIGNED_IN, WS_ONE)),
        ("the creator of a workspace", me(J, WS_ONE)),
        ("the creator, with a company role", me(J, WS_TWO)),
        ("not a member", me(SECOND_EMAIL, WS_ONE)),
        ("a workspace that does not exist", me(J, "no-such-workspace")),
    ]


def step_s16(base, pacer):
    """GET .../project-roles/ : the caller's roles in the workspace's projects (none yet)."""
    roles = lambda who, slug: api(base, pacer, who, "GET", f"/api/users/me/workspaces/{slug}/project-roles/")  # noqa: E731
    return [
        ("no session", roles(NOT_SIGNED_IN, WS_ONE)),
        ("a member of a workspace with no projects", roles(J, WS_ONE)),
        ("not a member", roles(SECOND_EMAIL, WS_ONE)),
        ("a workspace that does not exist", roles(J, "no-such-workspace")),
    ]


def step_s18(base, pacer):
    """GET .../members/ : the members of a workspace."""
    members = lambda who, slug: api(base, pacer, who, "GET", f"/api/workspaces/{slug}/members/")  # noqa: E731
    return [
        ("no session", members(NOT_SIGNED_IN, WS_ONE)),
        ("the creator, the only member", members(J, WS_ONE)),
        ("not a member", members(SECOND_EMAIL, WS_ONE)),
        ("a workspace that does not exist", members(J, "no-such-workspace")),
    ]


def workspace_read(base, pacer, template, query=""):
    """The cases every read of a workspace has: no session, a member, someone who is not one, a workspace that is not there.
    `template` is the route with `{slug}` where the workspace goes; `query` is what the stock app appends."""
    get = lambda who, slug: api(base, pacer, who, "GET", template.format(slug=slug) + query)  # noqa: E731
    return [
        ("no session", get(NOT_SIGNED_IN, WS_ONE)),
        ("a member of a workspace", get(J, WS_ONE)),
        ("not a member", get(SECOND_EMAIL, WS_ONE)),
        ("a workspace that does not exist", get(J, "no-such-workspace")),
    ]


def workspace_settings_read(base, pacer, template):
    """The reads that make the caller's row on their first visit (the pin creates it then): the cases above, and the first
    answer again, and what the creator's other workspace holds, which is a row of its own."""
    get = lambda who, slug: api(base, pacer, who, "GET", template.format(slug=slug))  # noqa: E731
    return [
        ("no session", get(NOT_SIGNED_IN, WS_ONE)),
        ("a member's first visit", get(J, WS_ONE)),
        ("the same member again", get(J, WS_ONE)),
        ("the same member in another workspace", get(J, WS_TWO)),
        ("not a member", get(SECOND_EMAIL, WS_ONE)),
        ("a workspace that does not exist", get(J, "no-such-workspace")),
    ]


def step_s17(base, pacer):
    """GET .../projects/ : the projects the caller can see (none yet, in any workspace)."""
    out = workspace_read(base, pacer, "/api/workspaces/{slug}/projects/")
    out.insert(2, ("a member of the other workspace", api(base, pacer, J, "GET", f"/api/workspaces/{WS_TWO}/projects/")))
    return out


def step_s19(base, pacer):
    """GET .../user-favorites/ : what the caller has starred; the stock app asks for all of it."""
    return workspace_read(base, pacer, "/api/workspaces/{slug}/user-favorites/", "?all=true")


def step_s20(base, pacer):
    """GET .../states/ : the work-item states of the projects the caller is in (none yet)."""
    return workspace_read(base, pacer, "/api/workspaces/{slug}/states/")


def step_s21(base, pacer):
    """GET .../sidebar-preferences/ : which entries of the sidebar the caller pins, and in what order."""
    return workspace_settings_read(base, pacer, "/api/workspaces/{slug}/sidebar-preferences/")


def step_s22(base, pacer):
    """GET .../user-properties/ : the caller's view settings for the workspace's work-item lists."""
    return workspace_settings_read(base, pacer, "/api/workspaces/{slug}/user-properties/")


def step_s23(base, pacer):
    """GET .../users/notifications/unread/ : how many notifications the caller has not read."""
    return workspace_read(base, pacer, "/api/workspaces/{slug}/users/notifications/unread/")


def step_s24(base, pacer):
    """GET .../home-preferences/ : which widgets the home page shows for the caller, and in what order."""
    return workspace_settings_read(base, pacer, "/api/workspaces/{slug}/home-preferences/")


def step_s25(base, pacer):
    """GET .../quick-links/ : the caller's links on the home page."""
    return workspace_read(base, pacer, "/api/workspaces/{slug}/quick-links/")


def step_s26(base, pacer):
    """GET .../recent-visits/ : what the caller opened lately, everything or only one kind of entity."""
    out = workspace_read(base, pacer, "/api/workspaces/{slug}/recent-visits/")
    out.insert(2, ("one kind of entity", api(base, pacer, J, "GET", f"/api/workspaces/{WS_ONE}/recent-visits/?entity_name=project")))
    return out


# ---------------------------------------------------------------------------------------------------------------------
# Projects (stage 5). The ids the pin makes that later steps name in their paths are kept here, by name, and a fixture says
# in its header which it makes (`binds`) and which it names (`uses`), so the replay can give the request the id Thrust made.
# ---------------------------------------------------------------------------------------------------------------------

CTX = {}

# The React payload of "create project" (components/projects/create/root.tsx, utils.ts): `cover_image_url` is read only on
# the pin, so it is ignored there.
PROJECT = {
    "cover_image_url": "/cover-images/image_5.webp",
    "description": "",
    "logo_props": {"in_use": "emoji", "emoji": {"value": "128512"}},
    "identifier": "JRN",
    "name": "Journey Project",
    "network": 2,
    "project_lead": None,
}
NO_SUCH_PROJECT = "7a9c1d5e-3b2f-4e86-9a47-5d1c0b8e6f23"
NO_SUCH_USER = "4f6c2f0a-9d52-4a3c-8c7e-0a1b2c3d4e5f"


def projects_path(slug=WS_ONE):
    return f"/api/workspaces/{slug}/projects/"


def step_s30(base, pacer):
    """POST .../projects/ : every refusal, then the journey's project, then what depends on it, then two more in another
    workspace (the same name and key there is allowed, and the second one is what S32 renames onto)."""
    create = lambda who, body, slug=WS_ONE: api(base, pacer, who, "POST", projects_path(slug), body)  # noqa: E731
    out = [
        ("no session", create(NOT_SIGNED_IN, PROJECT)),
        ("not a member of the workspace", create(SECOND_EMAIL, PROJECT)),
        ("no fields", create(J, {})),
        ("no name", create(J, {k: v for k, v in PROJECT.items() if k != "name"})),
        ("no identifier", create(J, {k: v for k, v in PROJECT.items() if k != "identifier"})),
        ("a blank name", create(J, {**PROJECT, "name": ""})),
        ("a blank identifier", create(J, {**PROJECT, "identifier": ""})),
        ("a name that is too long", create(J, {**PROJECT, "name": "n" * 256})),
        ("an identifier that is too long", create(J, {**PROJECT, "identifier": "I" * 13})),
        ("a name with a forbidden character", create(J, {**PROJECT, "name": "My.Project"})),
        ("an identifier with a forbidden character", create(J, {**PROJECT, "identifier": "A-B"})),
        ("a network that is not one of the two", create(J, {**PROJECT, "network": 3})),
        ("a lead who is not a user", create(J, {**PROJECT, "project_lead": NO_SUCH_USER})),
        ("a time zone that does not exist", create(J, {**PROJECT, "timezone": "Mars/Olympus"})),
        ("a name that is not text", create(J, {**PROJECT, "name": ["x"]})),
        ("a lead that is not an id", create(J, {**PROJECT, "project_lead": "not-an-id"})),
        ("a number of months out of range", create(J, {**PROJECT, "archive_in": 13})),
        ("a logo that is null", create(J, {**PROJECT, "logo_props": None})),
        ("a key with spaces around it, no name", create(J, {**PROJECT, "name": "", "identifier": "  JR4  "})),
    ]
    journey = create(J, PROJECT)
    out.append(("the journey's project", journey))
    CTX["project"] = journey["response"]["body"]["id"]
    out += [
        ("the same name again", create(J, {**PROJECT, "identifier": "JR2"})),
        ("the same name with spaces around it", create(J, {**PROJECT, "name": "  Journey Project  ", "identifier": "JR3"})),
        ("the same identifier again", create(J, {**PROJECT, "name": "Another Name"})),
    ]
    other = create(J, {**PROJECT, "name": "Other Project", "identifier": "OTH"}, WS_TWO)
    out.append(("another project, in the other workspace", other))
    CTX["other"] = other["response"]["body"]["id"]
    third = create(J, {**PROJECT, "name": "Third Project", "identifier": "THR", "network": 0, "description": "Third"}, WS_TWO)
    out.append(("a secret project with a description, in the other workspace", third))
    CTX["third"] = third["response"]["body"]["id"]
    out.append(("the journey's name and key, in the other workspace", create(J, PROJECT, WS_TWO)))
    # Not recorded: a lead who is someone else. The pin makes them an admin of the project even when they are not a member of the
    # workspace; Thrust refuses a lead who is not one (deviation D-026), and no one can be put in the oracle's workspace through its
    # API, so the journey has no such lead.
    return out


def step_s32(base, pacer):
    """PATCH .../projects/{id}/ : the cover image the app sends after the upload, then every rule."""
    patch = lambda who, pid, body, slug=WS_ONE: api(base, pacer, who, "PATCH", f"{projects_path(slug)}{pid}/", body)  # noqa: E731
    p, o = CTX["project"], CTX["other"]
    return [
        ("no session", patch(NOT_SIGNED_IN, p, {"name": "x"})),
        ("not a member of the workspace", patch(SECOND_EMAIL, p, {"name": "x"})),
        ("the cover image the app sends after the upload", patch(J, p, {"cover_image_url": "/cover-images/image_5.webp"})),
        ("an empty patch", patch(J, p, {})),
        ("a project that does not exist", patch(J, NO_SUCH_PROJECT, {"name": "x"})),
        ("a new name", patch(J, o, {"name": "Renamed Other"}, WS_TWO)),
        ("the name of another project", patch(J, o, {"name": "Third Project"}, WS_TWO)),
        ("its own name", patch(J, o, {"name": "Renamed Other"}, WS_TWO)),
        ("the key of another project", patch(J, o, {"identifier": "THR"}, WS_TWO)),
        ("a new key", patch(J, o, {"identifier": "OTHER"}, WS_TWO)),
        ("a name that is too long", patch(J, o, {"name": "n" * 256}, WS_TWO)),
        ("a forbidden character in the name", patch(J, o, {"name": "A&B"}, WS_TWO)),
        ("a network that is not one of the two", patch(J, o, {"network": 1}, WS_TWO)),
        ("a secret network, a description, the views and a time zone", patch(J, o, {"network": 0, "description": "Now secret", "cycle_view": True, "module_view": True, "issue_views_view": True, "page_view": False, "inbox_view": True, "timezone": "Europe/Berlin", "logo_props": {"in_use": "icon", "icon": {"name": "Smile", "color": "#fff"}}}, WS_TWO)),
        ("a view flag that is not a boolean", patch(J, o, {"cycle_view": "maybe"}, WS_TWO)),
        ("fields the pin does not let a patch change", patch(J, o, {"id": NO_SUCH_PROJECT, "workspace": NO_SUCH_PROJECT, "created_at": "2001-01-01T00:00:00Z", "deleted_at": "2001-01-01T00:00:00Z"}, WS_TWO)),
        # what DRF says of each kind of value a field may be sent as (probed against the pin, then recorded): a user as a number, a
        # float, a boolean, a list, an object or blank; a number of months as a float, a string and a value that is no number
        ("a lead and a default assignee that are an integer and a boolean", patch(J, o, {"project_lead": 5, "default_assignee": True}, WS_TWO)),
        ("a lead that is a negative integer", patch(J, o, {"project_lead": -1}, WS_TWO)),
        ("a lead that is a float", patch(J, o, {"project_lead": 1.5, "default_assignee": 0.0}, WS_TWO)),
        ("a lead that is a list", patch(J, o, {"project_lead": ["a", 1, None, True], "default_assignee": []}, WS_TWO)),
        ("a lead that is an object", patch(J, o, {"project_lead": {"a": 1, "b": ["c"]}, "default_assignee": {}}, WS_TWO)),
        ("a lead that is blank", patch(J, o, {"project_lead": "  "}, WS_TWO)),
        ("months that are not whole", patch(J, o, {"archive_in": 5.5, "close_in": "abc"}, WS_TWO)),
        ("months that are a boolean, null or a list", patch(J, o, {"archive_in": True, "close_in": None}, WS_TWO)),
        ("months that are a list and an object", patch(J, o, {"archive_in": [1], "close_in": {}}, WS_TWO)),
        ("months out of range as floats and strings", patch(J, o, {"archive_in": 13.0, "close_in": "-1"}, WS_TWO)),
        ("an estimate, a default state and a cover image that name nothing", patch(J, o, {"estimate": NO_SUCH_USER, "default_state": NO_SUCH_USER, "cover_image_asset": NO_SUCH_USER}, WS_TWO)),
        ("a logo that is null", patch(J, o, {"logo_props": None}, WS_TWO)),
        ("months as a float with no fraction and a string with one", patch(J, o, {"archive_in": 5.0, "close_in": "7.0"}, WS_TWO)),
        ("months as a string with spaces and a sign", patch(J, o, {"archive_in": " 7 ", "close_in": "+3"}, WS_TWO)),
        ("months at the top as a float", patch(J, o, {"archive_in": 12.0, "close_in": 12}, WS_TWO)),
        ("a lead that is an empty string, and values for the JSON columns that allow null or anything", patch(J, o, {"project_lead": "", "description_text": None, "description_html": 5, "icon_prop": None}, WS_TWO)),
        # the edges of the same readings: an integer past the largest signed 64-bit one, how Python writes the strings in a list or an
        # object (quotes, a backslash, a tab, a letter outside ASCII), floats at the edge of the exponent, and strings that are no whole number
        ("a lead that is an integer past the largest signed one", patch(J, o, {"project_lead": 9223372036854775808}, WS_TWO)),
        ("leads that are lists with a quote in a string", patch(J, o, {"project_lead": ["it's"], "default_assignee": ['say "hi"']}, WS_TWO)),
        ("leads that are lists with both quotes, a backslash and a tab", patch(J, o, {"project_lead": ["both ' and \""], "default_assignee": ["back\\slash", "tab\there", "é"]}, WS_TWO)),
        ("a lead that is an object with a quote in a key", patch(J, o, {"project_lead": {"it's": 1}}, WS_TWO)),
        ("leads that are lists with control characters", patch(J, o, {"project_lead": ["\x01"], "default_assignee": ["\x7f", "a\x1fb"]}, WS_TWO)),
        ("months as floats at the edge of the exponent", patch(J, o, {"archive_in": 1e16, "close_in": 9999999999999998.0}, WS_TWO)),
        ("months as a string with a fraction", patch(J, o, {"archive_in": "5.5"}, WS_TWO)),
        ("months as strings with zeros after the point", patch(J, o, {"archive_in": "12.000", "close_in": "0."}, WS_TWO)),
        ("a relation that is blank or no UUID, and one that is empty", patch(J, o, {"estimate": " ", "default_state": "x", "cover_image_asset": ""}, WS_TWO)),
        ("relations that are empty strings", patch(J, o, {"estimate": "", "default_state": "", "cover_image_asset": ""}, WS_TWO)),
        # the pin keeps null, a string and an empty list or object in `description_html`, and writes anything else as Python's str()
        ("a description_html that is a boolean", patch(J, o, {"description_html": True}, WS_TWO)),
        ("a description_html that is a float", patch(J, o, {"description_html": 1.5}, WS_TWO)),
        ("a description_html that is an object", patch(J, o, {"description_html": {"a": 1, "b": [None, "c"]}}, WS_TWO)),
        ("a description_html that is a list", patch(J, o, {"description_html": [1, "b", False]}, WS_TWO)),
        ("a description_html that is an empty object", patch(J, o, {"description_html": {}}, WS_TWO)),
        ("a description_html that is an empty list", patch(J, o, {"description_html": []}, WS_TWO)),
        ("months back to none", patch(J, o, {"archive_in": 0, "close_in": 0, "description_html": None}, WS_TWO)),
    ]


def project_read(base, pacer, template, query=""):
    """The cases every read of the journey's project has: no session, the creator, someone outside the workspace, and a
    project that does not exist. `template` is the route with `{slug}` and `{project}`."""
    get = lambda who, pid=None: api(base, pacer, who, "GET", template.format(slug=WS_ONE, project=pid or CTX["project"]) + query)  # noqa: E731
    return [
        ("no session", get(NOT_SIGNED_IN)),
        ("the creator of the project", get(J)),
        ("not a member of the workspace", get(SECOND_EMAIL)),
        ("a project that does not exist", get(J, NO_SUCH_PROJECT)),
    ]


def step_s34(base, pacer):
    """GET .../projects/{id}/ : the project, as the app opens it."""
    out = project_read(base, pacer, "/api/workspaces/{slug}/projects/{project}/")
    out.append(("the creator of a secret project they are in", api(base, pacer, J, "GET", f"{projects_path(WS_TWO)}{CTX['third']}/")))
    return out


def step_s35(base, pacer):
    """GET .../project-stats/ : counts for the projects of the workspace (all of them, whoever asks)."""
    get = lambda who, slug=WS_ONE, query="": api(base, pacer, who, "GET", f"/api/workspaces/{slug}/project-stats/{query}")  # noqa: E731
    return [
        ("no session", get(NOT_SIGNED_IN)),
        ("every project of the workspace", get(J)),
        ("the other workspace, with two projects", get(J, WS_TWO)),
        ("one project and the fields the app asks for", get(J, WS_ONE, f"?project_ids={CTX['project']}&fields=total_issues,completed_issues")),
        ("fields that are not counts", get(J, WS_ONE, "?fields=name,nothing")),
        ("a project of another workspace", get(J, WS_ONE, f"?project_ids={CTX['other']}")),
        ("not a member of the workspace", get(SECOND_EMAIL)),
        ("a workspace that does not exist", get(J, "no-such-workspace")),
        ("a project id that is not an id", get(J, WS_ONE, "?project_ids=not-an-id")),
    ]


def step_s36(base, pacer):
    return project_read(base, pacer, "/api/workspaces/{slug}/projects/{project}/project-members/me/")


def step_s37(base, pacer):
    out = project_read(base, pacer, "/api/workspaces/{slug}/projects/{project}/user-properties/")
    out.insert(2, ("the creator again", api(base, pacer, J, "GET", f"{projects_path()}{CTX['project']}/user-properties/")))
    return out


def step_s38(base, pacer):
    return project_read(base, pacer, "/api/workspaces/{slug}/projects/{project}/issue-labels/")


def step_s39(base, pacer):
    return project_read(base, pacer, "/api/workspaces/{slug}/projects/{project}/members/")


def step_s40(base, pacer):
    return project_read(base, pacer, "/api/workspaces/{slug}/projects/{project}/states/")


def step_s41(base, pacer):
    out = project_read(base, pacer, "/api/workspaces/{slug}/projects/{project}/intake-state/")
    out.insert(2, ("the creator again", api(base, pacer, J, "GET", f"{projects_path()}{CTX['project']}/intake-state/")))
    return out


def step_s42(base, pacer):
    return project_read(base, pacer, "/api/workspaces/{slug}/projects/{project}/estimates/")


def step_s43(base, pacer):
    return project_read(base, pacer, "/api/workspaces/{slug}/projects/{project}/cycles/")


def step_s44(base, pacer):
    """GET .../modules/ : the workspace's modules (the app asks for them next to the project's)."""
    return workspace_read(base, pacer, "/api/workspaces/{slug}/modules/")


def step_s45(base, pacer):
    return project_read(base, pacer, "/api/workspaces/{slug}/projects/{project}/modules/")


def step_s46(base, pacer):
    return project_read(base, pacer, "/api/workspaces/{slug}/projects/{project}/views/")


def step_s17_after(base, pacer):
    """GET .../projects/ once a project exists: the caller's role and sort order. One project only: the pin's list of several is
    in no order the request names (the query groups, which drops the model's ordering, and Postgres returns the groups by id), and
    ids differ between the pin and Thrust, so no implementation can match it."""
    get = lambda who, slug: api(base, pacer, who, "GET", projects_path(slug))  # noqa: E731
    return [
        ("one project", get(J, WS_ONE)),
        ("not a member of the workspace", get(SECOND_EMAIL, WS_ONE)),
    ]


def step_s20_after(base, pacer):
    """GET .../states/ once a project has states (one project only: states of two projects tie on `sequence`, and the pin's
    order for ties is not defined)."""
    get = lambda who: api(base, pacer, who, "GET", f"/api/workspaces/{WS_ONE}/states/")  # noqa: E731
    return [("the states of the one project", get(J)), ("not a member of the workspace", get(SECOND_EMAIL))]


def issues_path(project=None, slug=WS_ONE):
    return f"{projects_path(slug)}{project or CTX['project']}/issues/"


# What the stock app sends when it opens a project's work item list for the first time (list layout, nothing grouped, no
# filter): the display filters' defaults, the first page of 100 (apps/web store/issue/helpers/issue-filter-helper.store.ts).
FIRST_PAGE = "cursor=100:0:0&per_page=100&order_by=-created_at&sub_issue=true&layout=list&filters=%7B%7D"
# And what it sends to open one (store/issue/issue-details/issue.store.ts).
OPEN_ISSUE = "expand=issue_reactions,issue_attachments,issue_link,parent"


def step_s48(base, pacer):
    """POST .../projects/{id}/issues/ : every refusal, the work item the create modal sends, and a spread of others that the list
    and the detail then show (a priority, a state of each group, dates, an assignee, text that the pin cleans)."""
    post = lambda who, body, project=None: api(base, pacer, who, "POST", issues_path(project), body)  # noqa: E731
    states = api(base, pacer, J, "GET", f"{projects_path()}{CTX['project']}/states/")
    by_name = {s["name"]: s["id"] for s in states["response"]["body"]}
    me = api(base, pacer, J, "GET", ME)
    second = api(base, pacer, SECOND_EMAIL, "GET", ME)
    me_id, second_id = me["response"]["body"]["id"], second["response"]["body"]["id"]
    # the payload of the create modal (packages/constants/src/issue/modal.ts), with the name and state the person chose
    modal = {
        "project_id": CTX["project"], "type_id": None, "name": "Journey work item", "description_html": "<p>The first work item</p>",
        "estimate_point": None, "state_id": by_name["Backlog"], "parent_id": None, "priority": "none", "assignee_ids": [], "label_ids": [],
        "cycle_id": None, "module_ids": None, "start_date": None, "target_date": None,
    }
    out = [
        ("no session", post(NOT_SIGNED_IN, {"name": "x"})),
        ("not a member of the workspace", post(SECOND_EMAIL, {"name": "x"})),
        ("a project that does not exist", post(J, {"name": "x"}, NO_SUCH_PROJECT)),
        ("the states of the project", states),
        ("the signed-in user", me),
        ("the other user", second),
        ("no fields", post(J, {})),
        ("a blank name", post(J, {"name": ""})),
        ("a name of spaces", post(J, {"name": "   "})),
        ("a name that is too long", post(J, {"name": "n" * 256})),
        ("a name that is null", post(J, {"name": None})),
        ("a name that is a list", post(J, {"name": ["x"]})),
        ("a priority that is not one", post(J, {"name": "p", "priority": "huge"})),
        ("a priority that is null", post(J, {"name": "p", "priority": None})),
        ("a priority that is empty", post(J, {"name": "p", "priority": ""})),
        ("a start date after the target date", post(J, {"name": "d", "start_date": "2026-12-01", "target_date": "2026-11-01"})),
        ("a date that is not a date", post(J, {"name": "d", "start_date": "yesterday"})),
        ("a date with a time", post(J, {"name": "d", "target_date": "2026-11-01T10:00:00Z"})),
        ("a date with a space after it", post(J, {"name": "d", "start_date": "2026-01-05 "})),
        ("a date that does not exist", post(J, {"name": "d", "start_date": "2026-02-30"})),
        ("a date that is a number", post(J, {"name": "d", "start_date": 20260105})),
        ("a state that does not exist", post(J, {"name": "s", "state_id": NO_SUCH_PROJECT})),
        ("a state that is not an id", post(J, {"name": "s", "state_id": "nope"})),
        ("a parent that does not exist", post(J, {"name": "s", "parent_id": NO_SUCH_PROJECT})),
        ("an assignee that does not exist", post(J, {"name": "a", "assignee_ids": [NO_SUCH_USER]})),
        ("assignees that are not a list", post(J, {"name": "a", "assignee_ids": me_id})),
        ("a label that does not exist", post(J, {"name": "l", "label_ids": [NO_SUCH_USER]})),
        ("an estimate that does not exist", post(J, {"name": "e", "estimate_point": NO_SUCH_USER})),
        ("a description that is null", post(J, {"name": "h", "description_html": None})),
        ("a description that is a list", post(J, {"name": "h", "description_html": ["a"]})),
        # a list is read member by member: a member that is null or empty, and a value that is null or a boolean (DRF's messages)
        ("an assignee that is null", post(J, {"name": "a", "assignee_ids": [None]})),
        ("a label that is empty", post(J, {"name": "l", "label_ids": [""]})),
        ("labels that are null", post(J, {"name": "l", "label_ids": None})),
        ("labels that are true", post(J, {"name": "l", "label_ids": True})),
        # the edges of what a date can be (Python's date and ISO weeks): a week the year has not, a weekday that is not one, a year past the last
        ("a week that the year does not have", post(J, {"name": "d", "start_date": "2025-W53-1"})),
        ("a weekday of eight", post(J, {"name": "d", "start_date": "2026-W01-8"})),
        ("a year of five digits", post(J, {"name": "d", "start_date": "10000-01-01"})),
    ]
    first = post(J, modal)
    out.append(("the journey's work item", first))
    CTX["issue"] = first["response"]["body"]["id"]
    full = post(J, {**modal, "name": "Urgent, with an assignee and dates", "priority": "urgent", "assignee_ids": [me_id], "state_id": by_name["Todo"], "start_date": "2026-11-01", "target_date": "2026-11-15"})
    out.append(("an urgent work item with an assignee, dates and a state", full))
    CTX["full_issue"] = full["response"]["body"]["id"]
    out += [
        ("a high priority one in progress", post(J, {**modal, "name": "High, in progress", "priority": "high", "state_id": by_name["In Progress"], "target_date": "2026-10-20"})),
        ("a medium priority one that is done", post(J, {**modal, "name": "Medium, done", "priority": "medium", "state_id": by_name["Done"]})),
        ("a low priority one that is cancelled", post(J, {**modal, "name": "Low, cancelled", "priority": "low", "state_id": by_name["Cancelled"], "start_date": "2026-09-01"})),
        ("a name with spaces around it", post(J, {"name": "  padded  "})),
        ("a name that is a number", post(J, {"name": 5})),
        ("only a name", post(J, {"name": "Only a name"})),
        ("a state that is null and a blank one", post(J, {"name": "No state", "state_id": None})),
        ("the empty state", post(J, {"name": "Empty state", "state_id": ""})),
        ("the same assignee twice", post(J, {"name": "Twice", "assignee_ids": [me_id, me_id]})),
        ("an assignee who is not in the project", post(J, {"name": "Stranger", "assignee_ids": [second_id]})),
        # the other ways the pin reads a date: without the zeros, as eight digits, as an ISO week
        ("dates without the zeros and as eight digits", post(J, {"name": "Dates", "start_date": "2026-1-5", "target_date": "20260131"})),
        ("a date as an ISO week", post(J, {"name": "Week", "start_date": "2026-W01-1"})),
        ("fields that the pin sets itself", post(J, {"name": "Ignored", "sort_order": 5, "sequence_id": 99, "completed_at": "2026-01-01T00:00:00Z", "created_by": NO_SUCH_USER, "project_id": NO_SUCH_PROJECT, "type_id": NO_SUCH_USER, "cycle_id": NO_SUCH_USER, "module_ids": [NO_SUCH_USER]})),
    ]
    html = post(J, {"name": "Cleaned text", "description_html": "<script>alert(1)</script><p onclick='x()'>y</p><a href='javascript:alert(1)'>l</a><img src=x onerror=alert(1)>"})
    out.append(("a description with markup the pin cleans", html))
    CTX["html_issue"] = html["response"]["body"]["id"]
    out += [
        ("a description of plain text", post(J, {"name": "Plain", "description_html": "text"})),
        ("a description that is a number", post(J, {"name": "Number", "description_html": 5})),
    ]
    return out


def step_s47(base, pacer):
    """GET .../projects/{id}/issues/ : the first page the app asks for, the pages and orders it can ask, and what it is refused."""
    get = lambda query="", who=J, project=None: api(base, pacer, who, "GET", issues_path(project) + (f"?{query}" if query else ""))  # noqa: E731
    out = [
        ("no session", get(FIRST_PAGE, NOT_SIGNED_IN)),
        ("not a member of the workspace", get(FIRST_PAGE, SECOND_EMAIL)),
        ("a project that does not exist", get(FIRST_PAGE, J, NO_SUCH_PROJECT)),
        ("the first page the app asks for", get(FIRST_PAGE)),
        ("no query at all", get()),
        ("a cursor and no page size", get("cursor=3:0:0")),
        ("a page size and no cursor", get("per_page=3")),
        ("the first page of three", get("cursor=3:0:0&per_page=3&order_by=-created_at")),
        ("the second page", get("cursor=3:1:0&per_page=3&order_by=-created_at")),
        ("the last page", get("cursor=3:5:0&per_page=3&order_by=-created_at")),
        ("a page past the end", get("cursor=3:20:0&per_page=3&order_by=-created_at")),
        ("a cursor to the previous page", get("cursor=3:1:1&per_page=3&order_by=-created_at")),
        ("a cursor with a float for a size", get("cursor=3.5:0:0&per_page=3")),
        ("a cursor whose size disagrees with the page size", get("cursor=100:0:0&per_page=3")),
        ("pages of one", get("cursor=1:2:0&per_page=1")),
        ("the largest page size", get("cursor=1000:0:0&per_page=1000")),
    ]
    for order in ["created_at", "sort_order", "-sort_order", "sequence_id", "-sequence_id", "updated_at", "-updated_at", "target_date", "-target_date", "start_date", "-start_date", "priority", "-priority", "state__name", "-state__name", "state__group", "-state__group", "completed_at", "-completed_at", "archived_at", "name", "--created_at", "bogus", ""]:
        out.append((f"ordered by {order or 'nothing'}", get(f"cursor=100:0:0&per_page=100&order_by={order}")))
    out += [
        ("sub work items left out", get("cursor=100:0:0&per_page=100&sub_issue=false&order_by=sequence_id")),
        ("changed after a date in the far future", get("cursor=100:0:0&per_page=100&updated_at__gt=2099-01-01T00:00:00Z")),
        ("changed after a date long ago", get("cursor=100:0:0&per_page=100&updated_at__gt=2001-01-01T00:00:00Z&order_by=sequence_id")),
        ("a change date that is not a date", get("cursor=100:0:0&per_page=100&updated_at__gt=nonsense")),
        ("a page size over the cap", get("per_page=1001")),
        ("a page size that is not a number", get("per_page=abc")),
        ("a page size with spaces around it", get("per_page=%202%20")),
        ("a page size with a plus sign", get("per_page=%2B2")),
        ("a page size that is a decimal", get("per_page=1.0")),
        ("a cursor that is not a cursor", get("cursor=abc")),
        ("a cursor with two parts", get("cursor=3:0")),
        ("a cursor with a size that is not a number", get("cursor=x:0:0")),
        ("a cursor with a page that is not a number", get("cursor=3:x:0")),
        ("a cursor to a page before the first", get("cursor=3:-1:1&per_page=3")),
        ("filters that are not JSON", get("cursor=100:0:0&per_page=100&filters=nonsense")),
    ]
    return out


def step_x01(base, pacer):
    """GET .../projects/{id}/issues/{id}/ : the work item the app opens, with what it asks to be expanded."""
    get = lambda issue, query="", who=J, project=None: api(base, pacer, who, "GET", f"{issues_path(project)}{issue}/" + (f"?{query}" if query else ""))  # noqa: E731
    first, full, html = CTX["issue"], CTX["full_issue"], CTX["html_issue"]
    return [
        ("no session", get(first, OPEN_ISSUE, NOT_SIGNED_IN)),
        ("not a member of the workspace", get(first, OPEN_ISSUE, SECOND_EMAIL)),
        ("the journey's work item, as the app opens it", get(first, OPEN_ISSUE)),
        ("the same without what the app expands", get(first)),
        ("one with an assignee, dates and a state", get(full, OPEN_ISSUE)),
        ("one with a description the pin cleaned", get(html, OPEN_ISSUE)),
        ("a work item that does not exist", get(NO_SUCH_PROJECT, OPEN_ISSUE)),
        ("an id that is not an id", get("nope", OPEN_ISSUE)),
    ]


# Which fixtures make an id that later ones use (`binds`), and which name one in their requests (`uses`).
USES_PROJECT = {"uses": ["project"]}
FIXTURE_META = {
    "S30": {"binds": {"project": {"exchange": "the journey's project", "path": ["id"]}, "other": {"exchange": "another project, in the other workspace", "path": ["id"]}, "third": {"exchange": "a secret project with a description, in the other workspace", "path": ["id"]}}},
    "S32": {"uses": ["project", "other"]},
    "S34": {"uses": ["project", "third"]},
    "S36": USES_PROJECT,
    "S37": USES_PROJECT,
    "S38": USES_PROJECT,
    "S39": USES_PROJECT,
    "S40": USES_PROJECT,
    "S41": USES_PROJECT,
    "S42": USES_PROJECT,
    "S43": USES_PROJECT,
    "S45": USES_PROJECT,
    "S46": USES_PROJECT,
    "S35": {"uses": ["project", "other"]},
    "S48": {
        "uses": ["project"],
        "binds": {
            "issue": {"exchange": "the journey's work item", "path": ["id"]},
            "full_issue": {"exchange": "an urgent work item with an assignee, dates and a state", "path": ["id"]},
            "html_issue": {"exchange": "a description with markup the pin cleans", "path": ["id"]},
        },
    },
    "S47": USES_PROJECT,
    "X01": {"uses": ["project", "issue", "full_issue", "html_issue"]},
}


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
    ("S12", step_s12),
    ("S09", step_s09_before),
    ("S08", step_s08_before),
    ("S13", step_s13_before),
    ("S14", step_s14),
    ("S01-after", step_s01_after),
    ("S13-after", step_s13_after),
    ("S09-after", step_s09_after),
    ("S08-after", step_s08_after),
    ("S15", step_s15),
    ("S16", step_s16),
    ("S17", step_s17),
    ("S18", step_s18),
    ("S19", step_s19),
    ("S20", step_s20),
    ("S21", step_s21),
    ("S22", step_s22),
    ("S23", step_s23),
    ("S24", step_s24),
    ("S25", step_s25),
    ("S26", step_s26),
    ("S30", step_s30),
    ("S32", step_s32),
    ("S34", step_s34),
    ("S35", step_s35),
    ("S36", step_s36),
    ("S37", step_s37),
    ("S38", step_s38),
    ("S39", step_s39),
    ("S40", step_s40),
    ("S41", step_s41),
    ("S42", step_s42),
    ("S43", step_s43),
    ("S44", step_s44),
    ("S45", step_s45),
    ("S46", step_s46),
    ("S17-after", step_s17_after),
    ("S20-after", step_s20_after),
    ("S48", step_s48),
    ("S47", step_s47),
    ("X01", step_x01),
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
        meta = FIXTURE_META.get(step, {})
        # an id an earlier fixture made, which this one's requests name, is numbered first, in the order the fixture lists
        for name in meta.get("uses", []):
            ids.preseed(CTX[name])
        fixture = {
            "format_version": 1,
            "recorded_from": PIN,
            "row": step,
            **meta,
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
