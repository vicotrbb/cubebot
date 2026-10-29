"""Run shipped layout/binaries with disposable CSI storage; no fencing claim."""

import hashlib
import json
import os
import re
import secrets
import sys
import tempfile
import urllib.request
import uuid
from datetime import UTC, datetime
from functools import partial
from pathlib import Path

import cluster_admission as admission
import images as fixture_images
import integration
from cluster_chat_setup import wait_for

CSI_SOURCE = "https://raw.githubusercontent.com/kubernetes-csi/csi-driver-host-path/6fab18ae4fc4f486193b618114eeb817d9929242/"
SNAPSHOT = (
    "https://raw.githubusercontent.com/kubernetes-csi/external-snapshotter/v8.7.0/"
)
PLUGIN = (
    "docker.io/squat/generic-device-plugin:a11f990"
    "@sha256:5c48f7a16cfb37c4e07731bfa384b9dba6e12d2b0fb4350d8a321d49f12ac494"
)


def download(directory, name, url):
    with urllib.request.urlopen(url, timeout=30) as response:
        data = response.read(4 * 1024 * 1024 + 1)
    if len(data) > 4 * 1024 * 1024:
        raise RuntimeError("upstream CSI fixture manifest exceeds bound")
    path = directory / name
    path.write_bytes(data)
    return {"file": name, "url": url, "sha256": hashlib.sha256(data).hexdigest()}


def install_csi(config, artifacts):
    directory = artifacts / "csi"
    directory.mkdir()
    sources = []
    for name in ("volumesnapshotclasses", "volumesnapshotcontents", "volumesnapshots"):
        source = download(
            directory,
            f"{name}.yaml",
            SNAPSHOT + f"client/config/crd/snapshot.storage.k8s.io_{name}.yaml",
        )
        admission.kubectl(config, "apply", "-f", str(directory / source["file"]))
        sources.append(source)
    for name, repository, tag, path in (
        ("provisioner", "external-provisioner", "v6.4.0", "rbac.yaml"),
        ("attacher", "external-attacher", "v4.13.0", "rbac.yaml"),
        ("resizer", "external-resizer", "v2.3.0", "rbac.yaml"),
        (
            "snapshotter",
            "external-snapshotter",
            "v8.7.0",
            "csi-snapshotter/rbac-csi-snapshotter.yaml",
        ),
    ):
        url = f"https://raw.githubusercontent.com/kubernetes-csi/{repository}/{tag}/deploy/kubernetes/{path}"
        source = download(directory, f"{name}-rbac.yaml", url)
        admission.kubectl(config, "apply", "-f", str(directory / source["file"]))
        sources.append(source)
    for name in ("rbac-snapshot-controller", "setup-snapshot-controller"):
        source = download(
            directory,
            f"{name}.yaml",
            SNAPSHOT + f"deploy/kubernetes/snapshot-controller/{name}.yaml",
        )
        admission.kubectl(config, "apply", "-f", str(directory / source["file"]))
        sources.append(source)
    sources.extend(install_driver(config, directory))
    storageclass = {
        "apiVersion": "storage.k8s.io/v1",
        "kind": "StorageClass",
        "metadata": {
            "name": "cubebot-fixture-csi",
            "annotations": {"storageclass.kubernetes.io/is-default-class": "true"},
        },
        "provisioner": "hostpath.csi.k8s.io",
        "reclaimPolicy": "Delete",
        "volumeBindingMode": "Immediate",
    }
    admission.kubectl(
        config,
        "annotate",
        "storageclass",
        "standard",
        "storageclass.kubernetes.io/is-default-class=false",
        "--overwrite",
    )
    admission.kubectl(config, "create", "-f", "-", input=json.dumps(storageclass))
    admission.kubectl(
        config,
        "rollout",
        "status",
        "statefulset/csi-hostpathplugin",
        "-n",
        "default",
        "--timeout=180s",
        timeout=200,
    )
    return sources


def install_devices(config, name):
    """A generic device plugin hands /dev/kvm and /dev/net/tun to sandbox pods under the
    KubeVirt resource names the orchestrator requests."""
    devices = [
        {"name": device, "groups": [{"count": 8, "paths": [{"path": path}]}]}
        for device, path in (("kvm", "/dev/kvm"), ("tun", "/dev/net/tun"))
    ]
    args = ["--domain", "devices.kubevirt.io"]
    for device in devices:
        args += ["--device", json.dumps(device)]
    plugin = {
        "apiVersion": "apps/v1",
        "kind": "DaemonSet",
        "metadata": {"name": "device-plugin", "namespace": "kube-system"},
        "spec": {
            "selector": {"matchLabels": {"app": "device-plugin"}},
            "template": {
                "metadata": {"labels": {"app": "device-plugin"}},
                "spec": {
                    "automountServiceAccountToken": False,
                    "containers": [
                        {
                            "name": "plugin",
                            "image": PLUGIN,
                            "args": args,
                            "securityContext": {"privileged": True},
                            "volumeMounts": [
                                {
                                    "name": "plugins",
                                    "mountPath": "/var/lib/kubelet/device-plugins",
                                },
                                {"name": "dev", "mountPath": "/dev"},
                            ],
                        }
                    ],
                    "volumes": [
                        {
                            "name": "plugins",
                            "hostPath": {"path": "/var/lib/kubelet/device-plugins"},
                        },
                        {"name": "dev", "hostPath": {"path": "/dev"}},
                    ],
                },
            },
        },
    }
    admission.kubectl(config, "create", "-f", "-", input=json.dumps(plugin))

    def advertised():
        node = f"{name}-control-plane"
        status = json.loads(
            admission.kubectl(config, "get", "node", node, "-o", "json")
        )
        allocatable = status["status"]["allocatable"]
        return all(
            allocatable.get(f"devices.kubevirt.io/{device}", "0") != "0"
            for device in ("kvm", "tun")
        )

    wait_for("kvm and tun device advertisement", advertised)


def install_driver(config, directory):
    files = ("csi-hostpath-driverinfo.yaml", "csi-hostpath-plugin.yaml")
    sources = [
        download(
            directory, name, CSI_SOURCE + "deploy/kubernetes-1.35/hostpath/" + name
        )
        for name in files
    ]
    customization = {
        "apiVersion": "kustomize.config.k8s.io/v1beta1",
        "kind": "Kustomization",
        "resources": list(files),
        "images": [
            {"name": "registry.k8s.io/sig-storage/hostpathplugin", "newTag": "v1.19.0"}
        ],
    }
    (directory / "kustomization.yaml").write_text(json.dumps(customization))
    admission.kubectl(config, "apply", "-k", str(directory))
    return sources


def load_images(name, images):
    admission.run(
        str(admission.KIND),
        "load",
        "docker-image",
        "--name",
        name,
        *images.values(),
        timeout=240,
    )
    nodes = admission.run(
        str(admission.KIND), "get", "nodes", "--name", name
    ).splitlines()
    if len(nodes) != 1 or nodes[0] != f"{name}-control-plane":
        raise RuntimeError("unexpected owned node inventory")
    node = nodes[0]
    rows = admission.run(
        "docker", "exec", node, "ctr", "-n", "k8s.io", "images", "ls"
    ).splitlines()[1:]
    references = {}
    for role, image in images.items():
        source = f"docker.io/library/{image}:latest"
        matching = [row.split() for row in rows if row.split()[0] == source]
        if len(matching) != 1 or not re.fullmatch(
            r"sha256:[0-9a-f]{64}", matching[0][2]
        ):
            raise RuntimeError("missing actual imported image digest")
        reference = f"docker.io/library/{image}@{matching[0][2]}"
        admission.run(
            "docker",
            "exec",
            node,
            "ctr",
            "-n",
            "k8s.io",
            "images",
            "tag",
            source,
            reference,
        )
        admission.run(
            "docker",
            "exec",
            node,
            "ctr",
            "-n",
            "k8s.io",
            "images",
            "label",
            reference,
            "io.cri-containerd.image=managed",
        )
        references[role] = reference
    return references


def verify_database(_, env):
    output = admission.run(
        "target/debug/examples/cluster-startup", env=env, timeout=600
    )
    (Path(env["CUBEBOT_CLUSTER_ARTIFACTS"]) / "startup.json").write_text(output + "\n")


def verify(config, name, fixture, artifacts, images):
    version = json.loads(admission.kubectl(config, "version", "-o", "json"))[
        "serverVersion"
    ]
    if version["gitVersion"] != "v1.36.4":
        raise RuntimeError("unexpected Kubernetes version")
    admission.namespaces(config, fixture, (("cubebot-startup", "privileged"),))
    references = load_images(name, images)
    print("Installing test-only CSI backend in owned cluster", flush=True)
    dns = json.loads(
        admission.kubectl(
            config, "get", "service", "kube-dns", "-n", "kube-system", "-o", "json"
        )
    )["spec"]["clusterIP"]
    password = secrets.token_hex(24)
    env = os.environ | {
        "POSTGRES_PASSWORD": password,
        "CUBEBOT_CLUSTER_KUBECONFIG": str(config),
        "CUBEBOT_CLUSTER_CONTEXT": f"kind-{name}",
        "CUBEBOT_CLUSTER_FIXTURE": fixture,
        "CUBEBOT_CLUSTER_DNS": dns,
        "CUBEBOT_CLUSTER_ARTIFACTS": str(artifacts),
        "CUBEBOT_SANDBOX_IMAGE": references["sandbox"],
    }
    try:
        sources = install_csi(config, artifacts)
        install_devices(config, name)
        with tempfile.TemporaryDirectory(prefix="postgres-", dir=artifacts) as data:
            integration.run_database(
                f"{name}-postgres", env, password, data, verify_database
            )
    finally:
        diagnostics(config, artifacts)
    csi = json.loads(
        admission.kubectl(
            config,
            "get",
            "pods",
            "-n",
            "default",
            "-l",
            "app.kubernetes.io/instance=hostpath.csi.k8s.io",
            "-o",
            "json",
        )
    )
    return {
        "timestamp": datetime.now(UTC).isoformat(),
        "node_image": admission.NODE,
        "kind": admission.run(str(admission.KIND), "version"),
        "server": version,
        "nodes": admission.node_profile(config),
        "images": fixture_images.image_metadata(images),
        "node_image_references": references,
        "csi_sources": sources,
        "csi_pods": csi,
        "scope": "Test-only CSI/layout startup and clean termination; "
        "working agent and G1-G3 remain unqualified",
        "results": json.loads((artifacts / "startup.json").read_text()),
    }


def diagnostics(config, artifacts):
    for namespace in ("cubebot-startup", "default"):
        for resource in ("pods", "persistentvolumeclaims", "events"):
            try:
                result = admission.kubectl(
                    config, "get", resource, "-n", namespace, "-o", "json", timeout=10
                )
                (artifacts / f"{namespace}-{resource}.json").write_text(result + "\n")
            except Exception as error:
                print(
                    f"Diagnostic {namespace}/{resource} unavailable: {error}",
                    file=sys.stderr,
                )


def main():
    Path("target/cluster-startup.json").unlink(missing_ok=True)
    if not admission.KIND.is_file() or "v0.33.0" not in admission.run(
        str(admission.KIND), "version"
    ):
        raise RuntimeError("verified kind v0.33.0 is required")
    baseline = admission.fingerprint()
    integration.run(sys.executable, "tooling/guest/build.py")
    integration.run(
        "cargo",
        "build",
        "-p",
        "cubebot-orchestrator",
        "--locked",
        "--example",
        "cluster-startup",
    )
    name = f"cubebot-startup-{uuid.uuid4().hex[:12]}"
    builder = f"{name}-build"
    artifacts = Path("target") / name
    artifacts.mkdir()
    failed = False
    try:
        fixture_images.docker(
            "buildx", "create", "--name", builder, "--driver", "docker-container"
        )
        images = fixture_images.build(builder, name)
        if baseline != admission.fingerprint():
            raise RuntimeError(
                "source changed during build; rerun before accepting evidence"
            )
        with tempfile.TemporaryDirectory(prefix="private-", dir=artifacts) as private:
            report = admission.cluster(
                Path(private).resolve() / "kubeconfig",
                name,
                artifacts.resolve(),
                partial(verify, images=images),
            )
    except BaseException:
        failed = True
        raise
    finally:
        failures = fixture_images.cleanup(name, builder)
        if failures:
            print(
                "Owned image cleanup failed: " + "; ".join(failures),
                file=sys.stderr,
            )
            if not failed:
                raise RuntimeError("owned image cleanup failed")
    if baseline != admission.fingerprint():
        raise RuntimeError("source changed during startup fixture; rerun")
    remaining = admission.run(
        "docker", "ps", "--all", "--filter", f"name={name}", "--format", "{{.Names}}"
    )
    if remaining:
        raise RuntimeError("owned startup fixture container remains after cleanup")
    if admission.run(
        "docker",
        "image",
        "ls",
        "--filter",
        f"label=org.cubebot.fixture={name}",
        "--format",
        "{{.ID}}",
    ):
        raise RuntimeError("owned startup image remains after cleanup")
    report.update(
        source_sha256=baseline,
        base_commit=admission.run("git", "rev-parse", "HEAD"),
        cleanup_completed=True,
    )
    Path("target/cluster-startup.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
