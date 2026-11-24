#!/usr/bin/env python3
"""Render Terraform's additive Preview VPC-service map from durable cluster leases.

This is a read-only boundary: it never invokes OpenTofu or Cloudflare. A caller must use its
output as protected ``TF_VAR_preview_vpc_services`` only after it has atomically read the active
lease set. Invalid input is never treated as an empty desired set.
"""
import argparse
import hashlib
import json
import re
import sys

API_VERSION = "thrust.dev/preview-cluster-lease/v1"
MAX_LEASES = 50
SHA256 = re.compile(r"[0-9a-f]{64}\Z")
NAMESPACE = re.compile(r"[a-z0-9](?:[a-z0-9-]*[a-z0-9])?\Z")


def plan_is_valid(plan):
    fields = {"number", "migration_artifact_sha256", "postgres_major", "bootstrap_sha256", "fingerprint",
              "base_snapshot", "cluster", "vpc_service", "hyperdrive", "idempotency_key"}
    if not isinstance(plan, dict) or set(plan) != fields:
        return False
    number, major = plan["number"], plan["postgres_major"]
    migration, bootstrap = plan["migration_artifact_sha256"], plan["bootstrap_sha256"]
    if type(number) is not int or not 1 <= number <= 999_999_999 or type(major) is not int or not 14 <= major <= 18:
        return False
    if not isinstance(migration, str) or not isinstance(bootstrap, str) or not SHA256.fullmatch(migration) or not SHA256.fullmatch(bootstrap):
        return False
    digest = hashlib.sha256(b"thrust-preview-snapshot/v1\0" + major.to_bytes(2, "big") + bytes.fromhex(migration) + bytes.fromhex(bootstrap)).hexdigest()
    cluster = f"thrust-preview-pr-{number}-{digest[:12]}"
    return (plan["fingerprint"] == digest and plan["base_snapshot"] == f"thrust-preview-base-pg{major}-{digest[:16]}"
            and plan["cluster"] == cluster and plan["vpc_service"] == cluster and plan["hyperdrive"] == cluster
            and plan["idempotency_key"] == f"preview/{number}/{digest}")


def render(documents, namespace="thrust-preview"):
    if not isinstance(documents, list) or len(documents) > MAX_LEASES or not NAMESPACE.fullmatch(namespace):
        raise ValueError("lease collection or namespace is invalid")
    services, numbers = {}, set()
    for document in documents:
        if not isinstance(document, dict) or set(document) != {"api_version", "lease"} or document["api_version"] != API_VERSION:
            raise ValueError("lease document is not the supported bounded version")
        lease = document["lease"]
        if not isinstance(lease, dict) or set(lease) != {"plan", "phase"} or lease["phase"] not in {"provisioning", "ready", "deleting"} or not plan_is_valid(lease["plan"]):
            raise ValueError("lease is malformed or has a forged plan")
        if lease["phase"] == "deleting":
            continue
        plan = lease["plan"]
        if plan["number"] in numbers or plan["vpc_service"] in services:
            raise ValueError("active lease collection has duplicate preview ownership")
        numbers.add(plan["number"])
        services[plan["vpc_service"]] = {"hostname": f"{plan['cluster']}-rw.{namespace}.svc.cluster.local"}
    return services


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--leases", type=argparse.FileType(), required=True)
    parser.add_argument("--namespace", default="thrust-preview")
    args = parser.parse_args()
    json.dump(render(json.load(args.leases), args.namespace), sys.stdout, sort_keys=True, separators=(",", ":"))
    print()


if __name__ == "__main__":
    try:
        main()
    except (ValueError, json.JSONDecodeError) as error:
        print(f"render-preview-vpc-services: could not render active leases: {error}", file=sys.stderr)
        raise SystemExit(3)
