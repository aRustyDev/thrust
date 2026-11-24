#!/usr/bin/env python3
import copy
import importlib.util
import os
from pathlib import Path
import random
import unittest

SCRIPT = Path(os.environ.get("RENDER_PREVIEW_VPC_SERVICES", Path(__file__).with_name("render-preview-vpc-services.py")))
spec = importlib.util.spec_from_file_location("renderer", SCRIPT)
renderer = importlib.util.module_from_spec(spec)
spec.loader.exec_module(renderer)


def document(number, migration, bootstrap, major=16, phase="ready"):
    digest = __import__("hashlib").sha256(b"thrust-preview-snapshot/v1\0" + major.to_bytes(2, "big") + bytes.fromhex(migration) + bytes.fromhex(bootstrap)).hexdigest()
    cluster = f"thrust-preview-pr-{number}-{digest[:12]}"
    return {"api_version": renderer.API_VERSION, "lease": {"phase": phase, "plan": {
        "number": number, "migration_artifact_sha256": migration, "postgres_major": major, "bootstrap_sha256": bootstrap,
        "fingerprint": digest, "base_snapshot": f"thrust-preview-base-pg{major}-{digest[:16]}", "cluster": cluster,
        "vpc_service": cluster, "hyperdrive": cluster, "idempotency_key": f"preview/{number}/{digest}"}}}


class RendererTests(unittest.TestCase):
    def test_generated_valid_leases_render_only_active_canonical_hosts(self):
        rng = random.Random(57)
        for _ in range(100):
            migration = f"{rng.getrandbits(256):064x}"
            bootstrap = f"{rng.getrandbits(256):064x}"
            active = document(rng.randint(1, 999999999), migration, bootstrap, phase="provisioning")
            deleting = document(rng.randint(1, 999999999), migration, bootstrap, phase="deleting")
            result = renderer.render([active, deleting])
            name = active["lease"]["plan"]["vpc_service"]
            self.assertEqual(result, {name: {"hostname": f"{name}-rw.thrust-preview.svc.cluster.local"}})

    def test_refuses_forged_or_ambiguous_active_input(self):
        digest = "a" * 64
        good = document(42, digest, digest)
        for mutate in (
            lambda value: value["lease"]["plan"].__setitem__("cluster", "thrust-preview-pr-42-forged"),
            lambda value: value["lease"].__setitem__("phase", "unknown"),
            lambda value: value.__setitem__("extra", True),
        ):
            bad = copy.deepcopy(good); mutate(bad)
            with self.assertRaises(ValueError): renderer.render([bad])
        with self.assertRaises(ValueError): renderer.render([good, copy.deepcopy(good)])
        with self.assertRaises(ValueError): renderer.render([good], "wrong_namespace_")


if __name__ == "__main__":
    unittest.main()
