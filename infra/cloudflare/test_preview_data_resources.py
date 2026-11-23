#!/usr/bin/env python3
"""Generated hermetic controls for preview-data-resources.sh."""
import json
import os
from pathlib import Path
import random
import subprocess
import tempfile
import textwrap
import unittest


ROOT = Path(__file__).resolve().parent
SCRIPT = ROOT / "preview-data-resources.sh"
ACCOUNT = "a" * 32

FAKE_CURL = r'''#!/usr/bin/env python3
import json, os, sys
from pathlib import Path

state_path = Path(os.environ["FAKE_CF_STATE"])
state = json.loads(state_path.read_text())
args = sys.argv[1:]
method = args[args.index("-X") + 1]
url = args[-1]
body = sys.stdin.read()
state["calls"].append({"method": method, "url": url, "body": body})

def response(result=None, success=True):
    print(json.dumps({"success": success, "result": {} if result is None else result,
                      "errors": [] if success else [{"message": "not empty"}]}))

if "/r2/buckets?" in url and method == "GET":
    print(json.dumps({"success": True, "result": {"buckets": [{"name": name} for name in state["buckets"]]},
                      "result_info": {"cursor": None}}))
elif "/queues?" in url and method == "GET":
    print(json.dumps({"success": True, "result": state["queues"],
                      "result_info": {"page": 1, "total_pages": 1}}))
elif url.endswith("/r2/buckets") and method == "POST":
    name = json.loads(body)["name"]
    if name not in state["buckets"]: state["buckets"].append(name)
    response({"name": name})
elif url.endswith("/queues") and method == "POST":
    name = json.loads(body)["queue_name"]
    if not any(q["queue_name"] == name for q in state["queues"]):
        state["queues"].append({"queue_name": name, "queue_id": "q-" + str(len(state["queues"]) + 1),
                                "consumers_total_count": 0, "producers_total_count": 0})
    response()
elif "/queues/" in url and method == "DELETE":
    ident = url.rsplit("/", 1)[1]
    state["queues"] = [q for q in state["queues"] if q["queue_id"] != ident]
    response()
elif "/r2/buckets/" in url and url.endswith("/lifecycle") and method == "GET":
    response({"rules": state["rules"]})
elif "/r2/buckets/" in url and url.endswith("/lifecycle") and method == "PUT":
    state["rules"] = json.loads(body)["rules"]
    response()
elif "/r2/buckets/" in url and method == "DELETE":
    if state.get("nonempty", False): response(success=False)
    else:
        name = url.rsplit("/", 1)[1]
        state["buckets"] = [x for x in state["buckets"] if x != name]
        response()
else:
    raise SystemExit(f"unhandled fake request {method} {url}")

state_path.write_text(json.dumps(state))
'''


class PreviewDataResourcesTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.path = Path(self.temp.name)
        self.fake = self.path / "curl"
        self.fake.write_text(FAKE_CURL)
        self.fake.chmod(0o755)
        self.state = self.path / "state.json"

    def tearDown(self):
        self.temp.cleanup()

    def invoke(self, command, state):
        self.state.write_text(json.dumps({"calls": [], "rules": [], "nonempty": False, **state}))
        env = os.environ | {"CURL": str(self.fake), "FAKE_CF_STATE": str(self.state),
                            "CLOUDFLARE_ACCOUNT_ID": ACCOUNT, "PREVIEW_DATA_API_TOKEN": "test-token"}
        result = subprocess.run([str(SCRIPT), *command], env=env, text=True, capture_output=True)
        return result, json.loads(self.state.read_text())

    def test_generated_complete_pairs_are_listed_and_verified(self):
        rng = random.Random(57)
        for _ in range(20):
            number = rng.randrange(1, 1_000_000_000)
            bucket = f"thrust-pr-{number}-blobs"
            queue = f"thrust-pr-{number}-events"
            state = {"buckets": [bucket, "not-preview"],
                     "queues": [{"queue_name": queue, "queue_id": f"id-{number}", "consumers_total_count": 0, "producers_total_count": 0}]}
            listed, _ = self.invoke(["list-pr"], state)
            self.assertEqual(listed.returncode, 0, listed.stderr)
            self.assertEqual(json.loads(listed.stdout), [{"number": number, "bucket": bucket, "queue": queue, "queue_id": f"id-{number}", "consumers_total_count": 0, "producers_total_count": 0}])
            verified, _ = self.invoke(["verify-pr", str(number)], state)
            self.assertEqual(verified.returncode, 0, verified.stderr)

    def test_partial_pair_is_a_non_mutating_control(self):
        result, state = self.invoke(["ensure-pr", "81"], {"buckets": ["thrust-pr-81-blobs"], "queues": []})
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("refuse adoption", result.stderr)
        self.assertFalse(any(call["method"] == "POST" for call in state["calls"]))

    def test_missing_pair_is_created_and_reverified(self):
        result, state = self.invoke(["ensure-pr", "82"], {"buckets": [], "queues": []})
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(state["buckets"], ["thrust-pr-82-blobs"])
        self.assertEqual(state["queues"], [{"queue_name": "thrust-pr-82-events", "queue_id": "q-1", "consumers_total_count": 0, "producers_total_count": 0}])
        self.assertEqual([call["method"] for call in state["calls"] if call["method"] == "POST"], ["POST", "POST"])

    def test_nonempty_bucket_stages_only_the_named_expiry_rule(self):
        state = {"buckets": ["thrust-pr-83-blobs"], "nonempty": True,
                 "queues": [{"queue_name": "thrust-pr-83-events", "queue_id": "q-83", "consumers_total_count": 0, "producers_total_count": 0}]}
        result, observed = self.invoke(["drop-pr", "83"], state)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(observed["queues"], [])
        self.assertEqual(observed["rules"], [{"id": "thrust-preview-cleanup-pr-83", "enabled": True,
                                                "conditions": {"prefix": ""},
                                                "deleteObjectsTransition": {"condition": {"type": "Age", "maxAge": 86400}}}])

    def test_bound_queue_is_a_non_deleting_control(self):
        state = {"buckets": ["thrust-pr-84-blobs"],
                 "queues": [{"queue_name": "thrust-pr-84-events", "queue_id": "q-84",
                             "consumers_total_count": 0, "producers_total_count": 1}]}
        result, observed = self.invoke(["drop-pr", "84"], state)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("still bound", result.stderr)
        self.assertEqual(observed["queues"], state["queues"])
        self.assertFalse(any(call["method"] == "DELETE" for call in observed["calls"]))


if __name__ == "__main__":
    unittest.main()
