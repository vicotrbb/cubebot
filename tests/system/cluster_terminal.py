"""Owned dual-stack Kubernetes terminal mechanism qualification, never a deployment.

Evidence stays outside the checkout. This fixture is not website/provider acceptance.
"""

import json
import os
import shlex
import sys
import time
import uuid
from concurrent.futures import ThreadPoolExecutor
from functools import partial

import cluster_admission as admission
import cluster_startup as startup
import container_isolation as isolation
from cluster_chat import evidence_directory

NAMESPACE = "cubebot-terminal"


def security(uid, capabilities=()):
    return {
        "runAsUser": uid,
        "runAsGroup": uid,
        "runAsNonRoot": uid != 0,
        "allowPrivilegeEscalation": False,
        "readOnlyRootFilesystem": True,
        "capabilities": {"drop": ["ALL"], "add": list(capabilities)},
        "seccompProfile": {"type": "RuntimeDefault"},
    }


def container(name, image, uid, command, mounts=(), capabilities=()):
    return {
        "name": name,
        "image": image,
        "command": list(command),
        "securityContext": security(uid, capabilities),
        "volumeMounts": list(mounts),
        "resources": {"limits": {"memory": "256Mi", "cpu": "1"}},
    }


def mount(name, path, subpath=None):
    result = {"name": name, "mountPath": path}
    if subpath:
        result["subPath"] = subpath
    return result


def pod(image, dns):
    initializers = [
        container(
            "network",
            image,
            0,
            (
                "/usr/local/bin/cubebot-agent",
                "--initialize-network",
                json.dumps({"dns": [dns], "infrastructure": []}),
            ),
            capabilities=("NET_ADMIN",),
        ),
        container(
            "addresses",
            image,
            0,
            (
                "/bin/sh",
                "-ec",
                "ip addr add 9.9.9.9/32 dev lo; "
                "ip -6 addr add 2606:4700:4700::1111/128 dev lo; chmod 0755 /ipc",
            ),
            (mount("ipc", "/ipc"),),
            ("NET_ADMIN",),
        ),
    ]
    for role, uid in (("runtime", 10001), ("tools", 10002), ("executor", 0)):
        initializers.append(
            container(
                "initialize-" + role,
                image,
                uid,
                ("/usr/local/bin/cubebot-agent", "--initialize-storage", role),
                (mount("workspace", "/storage"),),
            )
        )
    runtime = container(
        "runtime",
        image,
        10001,
        ("/usr/local/bin/isolation-probe", "serve"),
        (
            mount("workspace", "/var/lib/cubebot", "runtime"),
            mount("ipc", "/run/cubebot-executor"),
        ),
    )
    tools = container(
        "tools",
        image,
        0,
        ("/usr/local/bin/cubebot-tool-runner",),
        (
            mount("workspace", "/home/agent", "tools"),
            mount("workspace", "/var/lib/cubebot-executor", "executor"),
            mount("ipc", "/run/cubebot-executor"),
            mount("tmp", "/tmp"),
        ),
        ("SETUID", "SETGID", "KILL"),
    )
    tools["env"] = [{"name": "CUBEBOT_TERMINAL_QUALIFICATION", "value": "1"}]
    return {
        "apiVersion": "v1",
        "kind": "Pod",
        "metadata": {"name": "terminal", "namespace": NAMESPACE},
        "spec": {
            "restartPolicy": "Never",
            "automountServiceAccountToken": False,
            "shareProcessNamespace": False,
            "securityContext": {
                "fsGroup": 20000,
                "supplementalGroupsPolicy": "Strict",
                "fsGroupChangePolicy": "OnRootMismatch",
            },
            "initContainers": initializers,
            "containers": [runtime, tools],
            "volumes": [
                {
                    "name": "workspace",
                    "persistentVolumeClaim": {"claimName": "workspace"},
                },
                {"name": "ipc", "emptyDir": {}},
                {"name": "tmp", "emptyDir": {"sizeLimit": "64Mi"}},
            ],
        },
    }


def k(config, *args, **kwargs):
    return admission.kubectl(config, "-n", NAMESPACE, *args, **kwargs)


def invoke(config, command, lease_seconds=3, identity=None, renew=False):
    request = identity or {
        "id": str(uuid.uuid4()),
        "authority": str(uuid.uuid4()),
        "command": command,
    }
    result = k(
        config,
        "exec",
        "terminal",
        "-c",
        "runtime",
        "--",
        "/usr/local/bin/isolation-probe",
        "terminal-renew" if renew else "terminal",
        json.dumps(request),
        str(lease_seconds * 1000),
        timeout=150,
    )
    result = json.loads(result)
    if "\n\nFull output: " in result.get("output", ""):
        preview, _, path = result["output"].rpartition("\n\nFull output: ")
        assert path == "/home/agent/.cubebot-terminal-output/" + request["id"] + ".log"
        result["output"] = preview
    return result


def create_pod(config, manifest):
    k(config, "create", "-f", "-", input=json.dumps(manifest))
    k(
        config,
        "wait",
        "--for=condition=Ready",
        "pod/terminal",
        "--timeout=120s",
        timeout=130,
    )


def commands(config):
    bash = invoke(config, r"""test -n "$BASH_VERSION" && echo -e 'first\nsecond'""")
    assert bash == {"status": "exited", "code": 0, "output": "first\nsecond\n"}, bash
    result = invoke(config, "printf retained > retained; printf hello; exit 7")
    assert result == {"status": "exited", "code": 7, "output": "hello"}, result
    result = invoke(
        config,
        "id; cat /var/lib/cubebot-executor/journal/state; "
        "cat /var/lib/cubebot/bootstrap-token; "
        "cat /run/secrets/kubernetes.io/serviceaccount/token",
    )
    assert "uid=10002" in result["output"] and "fixture-only" not in result["output"], (
        result
    )
    assert "Permission denied" in result["output"], result
    large = {
        "id": str(uuid.uuid4()),
        "authority": str(uuid.uuid4()),
        "command": "head -c 1048576 /dev/zero",
    }
    result = invoke(config, "", identity=large)
    assert result["status"] == "exited" and result["code"] == 0, result
    retained = invoke(
        config, "wc -c < .cubebot-terminal-output/" + large["id"] + ".log"
    )
    assert retained["output"].strip() == "1048576", retained
    result = invoke(
        config,
        "setsid sh -c 'sleep 2; echo escaped > escaped' "
        "</dev/null >/dev/null 2>&1 & sleep 20",
        lease_seconds=1,
    )
    assert result["status"] == "stopped" and result["reason"] == "authority_expired", (
        result
    )
    result = invoke(
        config, "sleep 3; test ! -e escaped && cat retained", lease_seconds=5
    )
    assert result == {"status": "exited", "code": 0, "output": "retained"}, result


def hostile(config):
    request = {
        "id": str(uuid.uuid4()),
        "authority": str(uuid.uuid4()),
        "command": "touch unauthorized",
    }
    result = invoke(
        config,
        "/usr/local/bin/isolation-probe terminal " + shlex.quote(json.dumps(request)),
    )
    assert result["status"] == "exited" and result["code"] != 0, result
    assert invoke(config, "test ! -e unauthorized")["code"] == 0
    result = invoke(config, "cat /proc/1/environ; kill -KILL 1")
    assert "Permission denied" in result["output"], result
    result = invoke(config, "unshare -Ur true")
    assert result["code"] != 0, result
    request["command"] = "sleep 2; echo escaped > cancel-escaped"
    k(
        config,
        "exec",
        "terminal",
        "-c",
        "runtime",
        "--",
        "/usr/local/bin/isolation-probe",
        "terminal-cancel",
        json.dumps(request),
    )
    result = invoke(config, "sleep 3; test ! -e cancel-escaped", lease_seconds=5)
    assert result == {"status": "exited", "code": 0, "output": ""}, result


def half_closed_authority(config):
    request = {
        "id": str(uuid.uuid4()),
        "authority": str(uuid.uuid4()),
        "command": "touch halfclosed-effect",
    }
    response = k(
        config,
        "exec",
        "terminal",
        "-c",
        "runtime",
        "--",
        "/usr/local/bin/isolation-probe",
        "terminal-half-close",
        json.dumps(request),
    )
    assert json.loads(response) == {
        "status": "stopped",
        "reason": "cancelled",
        "output": "",
    }
    assert invoke(config, "test ! -e halfclosed-effect")["code"] == 0


def hostile_output(config):
    request = {"id": str(uuid.uuid4()), "authority": str(uuid.uuid4()), "command": ""}
    path = ".cubebot-terminal-output/" + request["id"] + ".log"
    request["command"] = "rm -- " + path + "; ln -s /proc/1/environ " + path
    result = invoke(config, "", identity=request)
    assert result["code"] == 0 and "preview unavailable" in result["output"], result
    request["id"] = str(uuid.uuid4())
    path = ".cubebot-terminal-output/" + request["id"] + ".log"
    request["command"] = "rm -- " + path + "; mkfifo " + path
    started = time.monotonic()
    result = invoke(config, "", identity=request, lease_seconds=1, renew=True)
    assert result["code"] == 0 and "preview unavailable" in result["output"], result
    assert time.monotonic() - started < 8, result


def process_limit(config):
    started = time.monotonic()
    result = invoke(
        config,
        'i=0; while [ "$i" -lt 200 ]; do sleep 20 & i=$((i+1)); done; wait',
        lease_seconds=2,
    )
    assert time.monotonic() - started < 8, result
    assert result["status"] in ("exited", "stopped"), result
    assert invoke(config, "printf alive")["output"] == "alive"


def queued_authority(config):
    with ThreadPoolExecutor(max_workers=3) as pool:
        first = pool.submit(
            invoke,
            config,
            "echo first > serial; touch serial-start; sleep 4; echo done >> serial",
            8,
        )
        wait_started(config)
        second = pool.submit(
            invoke,
            config,
            'test "$(tail -n 1 serial)" = done && echo second >> serial',
            10,
        )
        expired = pool.submit(invoke, config, "touch expired-effect", 1)
        assert first.result(timeout=45)["code"] == 0
        assert second.result(timeout=45)["code"] == 0
        assert expired.result(timeout=45)["status"] == "stopped"
    result = invoke(config, "test ! -e expired-effect; cat serial")
    assert result["output"] == "first\ndone\nsecond\n", result


def wait_started(config):
    from cluster_chat_setup import wait_for

    def inspect():
        return (
            k(
                config,
                "exec",
                "terminal",
                "-c",
                "tools",
                "--",
                "/usr/bin/setpriv",
                "--reuid=10002",
                "--regid=10002",
                "--clear-groups",
                "/bin/sh",
                "-c",
                "if test -e /home/agent/serial-start; then echo started; fi",
            )
            == "started"
        )

    wait_for("first serialized process", inspect, timeout=5)


def http_networks(config):
    cases = (
        ("http://9.9.9.9:18084/", True),
        ("http://[2606:4700:4700::1111]:18085/", True),
        ("http://9.9.9.9:18084/redirect-private", False),
        ("http://9.9.9.9:18084/redirect-private-v6", False),
        ("http://qualification.invalid:18084/?resolve=9.9.9.9", True),
        ("http://qualification.invalid:18084/?resolve=127.0.0.1", False),
        ("http://qualification.invalid:18085/?resolve=::1", False),
        ("https://example.com/", True),
    )
    for url, expected in cases:
        result = invoke(
            config,
            "/usr/local/bin/isolation-probe connect " + shlex.quote(url),
            lease_seconds=5,
        )
        assert result["status"] == "exited", result
        assert json.loads(result["output"])["connected"] == expected, (url, result)
    for url in (
        "http://qualification.invalid:18084/?resolve=127.0.0.1",
        "http://qualification.invalid:18085/?resolve=::1",
    ):
        result = k(
            config,
            "exec",
            "terminal",
            "-c",
            "runtime",
            "--",
            "/usr/local/bin/isolation-probe",
            "connect",
            url,
        )
        assert json.loads(result)["connected"], result


def networks(config):
    for address, allowed in (
        ("127.0.0.1:18080", False),
        ("[::1]:18081", False),
        ("9.9.9.9:18080", True),
        ("[2606:4700:4700::1111]:18081", True),
    ):
        result = invoke(
            config, "/usr/local/bin/isolation-probe connect '" + address + "'"
        )
        assert result["status"] == "exited", result
        assert json.loads(result["output"])["connected"] == allowed, (address, result)


def retained(config, manifest):
    request = {
        "id": str(uuid.uuid4()),
        "authority": str(uuid.uuid4()),
        "command": "echo once >> effects; cat effects",
    }
    first = invoke(config, "", identity=request)
    assert first["output"] == "once\n", first
    k(config, "delete", "pod", "terminal", "--wait=true", timeout=60)
    create_pod(config, manifest)
    assert invoke(config, "", identity=request) == first
    assert invoke(config, "cat effects; cat retained")["output"] == "once\nretained"


def qualify(config, name, fixture, artifacts, images):
    admission.namespaces(config, fixture, ((NAMESPACE, "privileged"),))
    references = startup.load_images(name, images)
    startup.install_csi(config, artifacts)
    dns = json.loads(
        admission.kubectl(
            config, "-n", "kube-system", "get", "service", "kube-dns", "-o", "json"
        )
    )["spec"]["clusterIP"]
    claim = {
        "apiVersion": "v1",
        "kind": "PersistentVolumeClaim",
        "metadata": {"name": "workspace"},
        "spec": {
            "accessModes": ["ReadWriteOncePod"],
            "resources": {"requests": {"storage": "1Gi"}},
        },
    }
    k(config, "create", "-f", "-", input=json.dumps(claim))
    manifest = pod(references["qualification"], dns)
    try:
        create_pod(config, manifest)
        commands(config)
        continued = invoke(
            config, "sleep 65; printf continued", lease_seconds=1, renew=True
        )
        assert continued["code"] == 0 and continued["output"] == "continued", continued
        networks(config)
        http_networks(config)
        queued_authority(config)
        hostile(config)
        hostile_output(config)
        half_closed_authority(config)
        process_limit(config)
        retained(config, manifest)
        (artifacts / "mechanisms.json").write_text(
            json.dumps({"commands": True, "network": True, "retained": True})
        )
    finally:
        for role in ("tools", "runtime"):
            try:
                (artifacts / (role + ".log")).write_text(
                    k(config, "logs", "terminal", "-c", role, timeout=10)
                )
            except RuntimeError:
                pass
        for resource in ("pods", "events"):
            (artifacts / (resource + ".json")).write_text(
                k(config, "get", resource, "-o", "json")
            )


def main():
    artifacts = evidence_directory(os.environ["CUBEBOT_QUALIFICATION_DIRECTORY"])
    name = "cubebot-terminal-" + uuid.uuid4().hex[:12]
    supplied = os.environ.get("CUBEBOT_QUALIFICATION_BUILDER")
    builder = supplied or name + "-build"
    config = artifacts / "kubeconfig"
    baseline = admission.fingerprint()
    try:
        if not supplied:
            isolation.docker(
                "buildx", "create", "--name", builder, "--driver", "docker-container"
            )
        images = isolation.build(builder, name, targets=("qualification",))
        if admission.fingerprint() != baseline:
            raise RuntimeError("source changed during build")
        admission.cluster(config, name, artifacts, partial(qualify, images=images))
    finally:
        config.unlink(missing_ok=True)
        failures = isolation.cleanup(name, None if supplied else builder)
        if failures:
            raise RuntimeError("owned cleanup failed: " + "; ".join(failures))
    if admission.fingerprint() != baseline:
        raise RuntimeError("source changed during qualification")
    print(
        "Owned terminal mechanism checks completed; "
        "website/provider and full G1-G3 remain required"
    )


if __name__ == "__main__":
    try:
        main()
    except KeyboardInterrupt:
        sys.exit(130)
