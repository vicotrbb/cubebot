"""Repeatable real cluster text-chat qualification, with evidence outside the repo.

Supply CUBEBOT_QUALIFICATION_DIRECTORY outside the checkout and OPENAI_API_KEY_FILE.
Needs x86_64 Linux with /dev/kvm and the kernel build tools of tooling/guest: the
sandbox image always carries the guest, and every agent runs in a KVM guest.
Images are built from this checkout. An explicitly supplied disposable, caller-owned
CUBEBOT_QUALIFICATION_BUILDER can reuse its cache; otherwise a new builder is removed.
Only a fresh owned kind cluster is used; finally removes it even on failure.
"""

import json
import os
import subprocess
import sys
import uuid
from functools import partial
from pathlib import Path

import cluster_admission as admission
import cluster_startup as startup
from cluster_chat_setup import NAMESPACE, Fixture, available, forward, setup, wait_for


def evidence_directory(value):
    directory = Path(value).resolve()
    checkout = Path(__file__).resolve().parents[2]
    if directory == checkout or checkout in directory.parents:
        raise ValueError("qualification evidence must be outside the repository")
    directory.mkdir(parents=True, exist_ok=True, mode=0o700)
    return directory


def diagnostics(f):
    runtime_diagnostics(f)
    for resource in ("pods", "persistentvolumeclaims", "events"):
        try:
            f.save(f"{resource}.json", json.loads(f.k("get", resource, "-o", "json")))
        except Exception as error:
            print(f"Diagnostic {resource} unavailable: {error}", file=sys.stderr)
    for name in ("orchestrator", "gateway"):
        try:
            result = f.k("logs", name, "--tail=200")
            (f.artifacts / f"{name}.log").write_text(result)
        except Exception as error:
            print(f"Diagnostic {name} unavailable: {error}", file=sys.stderr)


def runtime_diagnostics(f):
    try:
        pods = json.loads(f.k("get", "pods", "-o", "json"))["items"]
        for pod in pods:
            name = pod["metadata"]["name"]
            if not name.startswith("cb-"):
                continue
            try:
                output = f.k("logs", name, "-c", "sandbox", "--tail=100")
                (f.artifacts / (name + "-sandbox.log")).write_text(output)
            except Exception as error:
                print(f"Agent log unavailable: {error}", file=sys.stderr)
    except Exception as error:
        print(f"Agent inventory unavailable: {error}", file=sys.stderr)


def workflow(
    config,
    name,
    fixture,
    artifacts,
    images,
    qualify=None,
    **controller,
):
    if qualify is None:
        from cluster_chat_flow import qualify

    # The sandbox runs as root with NET_ADMIN (ADR 0084), which only privileged admits.
    admission.namespaces(config, fixture, ((NAMESPACE, "privileged"),))
    references = startup.load_images(name, images)
    f = Fixture(config, name, artifacts, references)
    f.save("images.json", references)
    dns = json.loads(
        admission.kubectl(
            config,
            "get",
            "service",
            "kube-dns",
            "-n",
            "kube-system",
            "-o",
            "json",
        )
    )["spec"]["clusterIP"]
    process = None
    try:
        startup.install_csi(config, artifacts)
        startup.install_devices(config, name)
        setup(f, dns, **controller)
        process = forward(f)
        wait_for("verified HTTPS API", lambda: available(f))
        f.token = f.request(
            "/api/identity/login",
            {
                "login": "admin",
                "password": f.password,
            },
        )["token"]
        qualify(f)
    finally:
        try:
            diagnostics(f)
        finally:
            if process is not None:
                process.terminate()
                process.wait(timeout=10)


def main(prefix="cubebot-chat-", **options):
    """`options` go to workflow."""
    artifacts = evidence_directory(os.environ["CUBEBOT_QUALIFICATION_DIRECTORY"])
    name = prefix + uuid.uuid4().hex[:12]
    config = artifacts / "kubeconfig"
    baseline = admission.fingerprint()
    supplied = os.environ.get("CUBEBOT_QUALIFICATION_BUILDER")
    builder = supplied or f"{name}-build"
    try:
        if not supplied:
            admission.run(
                "docker",
                "buildx",
                "create",
                "--name",
                builder,
                "--driver",
                "docker-container",
            )
        with (artifacts / "build-guest.log").open("w") as log:
            subprocess.run(
                [sys.executable, "tooling/guest/build.py"],
                stdout=log,
                stderr=log,
                check=True,
                timeout=3600,
            )
        images = build_images(builder, name, artifacts, baseline)
        if admission.fingerprint() != baseline:
            raise RuntimeError("source changed during image build; repeat")
        admission.cluster(
            config, name, artifacts, partial(workflow, images=images, **options)
        )
    finally:
        config.unlink(missing_ok=True)
        cleanup_images(name, builder if not supplied else None)
    if admission.fingerprint() != baseline:
        raise RuntimeError(
            "source changed during qualification; repeat on final source"
        )
    print("Owned qualification and cluster cleanup completed", flush=True)


def build_images(builder, name, artifacts, source):
    images = {}
    for role in ("sandbox", "orchestrator", "web"):
        image = f"{name}-{role}"
        args = [
            "docker",
            "buildx",
            "build",
            "--builder",
            builder,
            "--load",
            "--label",
            f"org.cubebot.fixture={name}",
            "--label",
            f"org.cubebot.source={source}",
            "--tag",
            image,
            "--file",
            f"apps/{role}/Dockerfile",
        ]
        if role == "sandbox":
            args += ["--build-context", "guest=target/guest"]
        with (artifacts / f"build-{role}.log").open("w") as log:
            subprocess.run(
                [*args, "."], stdout=log, stderr=log, check=True, timeout=1200
            )
        images[role] = image
    return images


def cleanup_images(name, builder):
    failures = []
    for role in ("sandbox", "orchestrator", "web"):
        try:
            admission.run("docker", "image", "rm", f"{name}-{role}")
        except RuntimeError as error:
            if "No such image" not in str(error):
                failures.append(str(error))
    if builder:
        try:
            admission.run("docker", "buildx", "rm", "--force", builder)
        except RuntimeError as error:
            failures.append(str(error))
    if failures:
        raise RuntimeError("owned image cleanup failed: " + "; ".join(failures))


if __name__ == "__main__":
    try:
        main()
    except KeyboardInterrupt:
        sys.exit(130)
