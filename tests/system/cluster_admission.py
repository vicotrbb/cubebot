"""Actual Kubernetes admission/defaulting against an owned disposable kind cluster."""

import hashlib
import json
import os
import secrets
import subprocess
import sys
import tempfile
import uuid
from datetime import UTC, datetime
from pathlib import Path

import integration

KIND = Path(".tools/kind-v0.33.0").resolve()
NODE = (
    "kindest/node:v1.36.4@sha256:"
    "099e049362a1526b2db71494e1947aae99bd16290d7c895f2b7ea312e3cbfaed"
)


def run(*args, input=None, env=None, timeout=60):
    result = subprocess.run(
        args,
        input=input,
        env=env,
        text=True,
        capture_output=True,
        timeout=timeout,
        check=False,
    )
    if result.returncode:
        raise RuntimeError(result.stderr.strip())
    return result.stdout.strip()


def kubectl(config, *args, input=None, timeout=60):
    return run(
        "kubectl", "--kubeconfig", str(config), *args, input=input, timeout=timeout
    )


def fingerprint():
    digest = hashlib.sha256()
    names = run(
        "git", "ls-files", "--cached", "--others", "--exclude-standard"
    ).splitlines()
    for name in sorted(names):
        path = Path(name)
        if path.is_file():
            digest.update(name.encode() + b"\0" + path.read_bytes())
    return digest.hexdigest()


def namespaces(
    config,
    fixture,
    profiles=(
        ("cubebot-admission", "privileged"),
        ("cubebot-admission-restricted", "restricted"),
    ),
):
    for name, policy in profiles:
        namespace = {
            "apiVersion": "v1",
            "kind": "Namespace",
            "metadata": {
                "name": name,
                "labels": {
                    "cubebot.dev/fixture": fixture,
                    "pod-security.kubernetes.io/enforce": policy,
                    "pod-security.kubernetes.io/enforce-version": "v1.36",
                },
            },
        }
        kubectl(config, "create", "-f", "-", input=json.dumps(namespace))
        kubectl(
            config, "create", "serviceaccount", "cubebot-runtime", "--namespace", name
        )


def verify_database(_, env):
    result = run("target/debug/examples/cluster-admission", env=env, timeout=120)
    report = json.loads(result)
    destination = Path(env["CUBEBOT_CLUSTER_ARTIFACTS"]) / "admission.json"
    destination.write_text(json.dumps(report, indent=2) + "\n")


def node_profile(config):
    nodes = json.loads(kubectl(config, "get", "nodes", "-o", "json"))["items"]
    return [
        {
            "name": node["metadata"]["name"],
            "pod_cidrs": node["spec"].get("podCIDRs", []),
            "runtime": node["status"]["nodeInfo"],
            "features": node["status"].get("features", {}),
            "capacity": node["status"].get("capacity", {}),
            "allocatable": node["status"].get("allocatable", {}),
        }
        for node in nodes
    ]


def verify(config, name, fixture, artifacts):
    version = json.loads(kubectl(config, "version", "-o", "json"))["serverVersion"]
    if version["gitVersion"] != "v1.36.4":
        raise RuntimeError("unexpected Kubernetes qualification version")
    namespaces(config, fixture)
    dns = json.loads(
        kubectl(config, "get", "service", "kube-dns", "-n", "kube-system", "-o", "json")
    )["spec"]["clusterIP"]
    password = secrets.token_hex(24)
    env = os.environ | {
        "POSTGRES_PASSWORD": password,
        "CUBEBOT_CLUSTER_KUBECONFIG": str(config),
        "CUBEBOT_CLUSTER_CONTEXT": f"kind-{name}",
        "CUBEBOT_CLUSTER_FIXTURE": fixture,
        "CUBEBOT_CLUSTER_DNS": dns,
        "CUBEBOT_CLUSTER_ARTIFACTS": str(artifacts),
    }
    with tempfile.TemporaryDirectory(prefix="postgres-", dir=artifacts) as data:
        integration.run_database(
            f"{name}-postgres", env, password, data, verify_database
        )
    return {
        "timestamp": datetime.now(UTC).isoformat(),
        "node_image": NODE,
        "kind": run(str(KIND), "version"),
        "server": version,
        "nodes": node_profile(config),
        "scope": "API admission and prepared-manifest persistence; "
        "no running agent or G1-G3 qualification",
        "results": json.loads((artifacts / "admission.json").read_text()),
    }


def cluster(config, name, artifacts, workflow):
    if name in run(str(KIND), "get", "clusters").splitlines():
        raise RuntimeError("refusing to reuse an existing cluster")
    configuration = {
        "kind": "Cluster",
        "apiVersion": "kind.x-k8s.io/v1alpha4",
        "networking": {"ipFamily": "dual", "apiServerAddress": "127.0.0.1"},
        "nodes": [{"role": "control-plane"}],
    }
    source = artifacts / "kind.json"
    source.write_text(json.dumps(configuration))
    failed = False
    try:
        print("Creating owned Kubernetes 1.36.4 admission fixture", flush=True)
        run(
            str(KIND),
            "create",
            "cluster",
            "--name",
            name,
            "--kubeconfig",
            str(config),
            "--image",
            NODE,
            "--config",
            str(source),
            "--wait",
            "120s",
            timeout=360,
        )
        return workflow(config, name, name, artifacts)
    except BaseException:
        failed = True
        raise
    finally:
        try:
            cleanup(config, name)
        except Exception as error:
            print(f"Owned cluster cleanup failed: {error}", file=sys.stderr)
            if not failed:
                raise


def cleanup(config, name):
    run(
        str(KIND),
        "delete",
        "cluster",
        "--name",
        name,
        "--kubeconfig",
        str(config),
        timeout=120,
    )
    remaining = run(
        "docker",
        "ps",
        "--all",
        "--filter",
        f"label=io.x-k8s.kind.cluster={name}",
        "--format",
        "{{.Names}}",
    )
    if remaining:
        raise RuntimeError("owned cluster node remains after deletion")


def main():
    Path("target/cluster-admission.json").unlink(missing_ok=True)
    if not KIND.is_file() or "v0.33.0" not in run(str(KIND), "version"):
        raise RuntimeError(
            "install the verified kind v0.33.0 binary at .tools/kind-v0.33.0"
        )
    baseline = fingerprint()
    integration.run(
        "cargo",
        "build",
        "-p",
        "cubebot-orchestrator",
        "--locked",
        "--example",
        "cluster-admission",
    )
    if fingerprint() != baseline:
        raise RuntimeError("source changed during compilation; rerun admission fixture")
    name = f"cubebot-admission-{uuid.uuid4().hex[:12]}"
    artifacts = Path("target") / name
    artifacts.mkdir()
    with tempfile.TemporaryDirectory(prefix="private-", dir=artifacts) as private:
        report = cluster(
            Path(private).resolve() / "kubeconfig", name, artifacts.resolve(), verify
        )
    if fingerprint() != baseline:
        raise RuntimeError(
            "source changed during admission fixture; rerun before accepting evidence"
        )
    report["source_sha256"] = baseline
    report["base_commit"] = run("git", "rev-parse", "HEAD")
    remaining = run(
        "docker", "ps", "--all", "--filter", f"name={name}", "--format", "{{.Names}}"
    )
    if remaining:
        raise RuntimeError("owned fixture container remains after cleanup")
    report["cleanup_completed"] = True
    Path("target/cluster-admission.json").write_text(
        json.dumps(report, indent=2) + "\n"
    )
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
