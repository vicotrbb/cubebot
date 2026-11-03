"""deploy/install.sh against a local registry, as a user runs it from a release.

Builds the orchestrator, web and agent images from this checkout, pushes them to a
disposable registry under one tag, and runs install.sh with that registry and this
checkout as its file source into a temporary home under its own Compose project. Then
it creates the administrator through the setup endpoint, checks setup closed and the
declared runtime, wakes one agent until its runtime authenticates over the development
control CA, runs install.sh again to check it keeps the database password, and stops
the installation with `cubebot down`. With CUBEBOT_PLAYWRIGHT_MODULE set, it then
empties the database and creates the administrator from the console's setup form in
Chromium. It removes that installation with `cubebot uninstall --yes`, then installs
again through a pty, answering every prompt with keystrokes into a temporary $HOME,
signs in with the administrator created there and uninstalls it, checking the
directory, the volumes and the images are gone each time. No provider is called.
Needs Docker only; the control port 8443 must be free. The runtime is container unless
CUBEBOT_SANDBOX_RUNTIME=kvm, which also builds the guest and sandbox image and needs
what `just guest-boot` needs.
"""

import os
import secrets
import socket
import subprocess
import sys
import tempfile
import time
from pathlib import Path

from cluster_chat_setup import wait_for
from docker_sandbox import INSTRUCTIONS, Api, agent, command, remove
from install_widgets import Terminal

ROOT = Path(__file__).resolve().parents[2]
TAG = "install-check"
# Unlike the docker-* suites, container unless asked: CI runners may have /dev/kvm.
RUNTIME = os.environ.get("CUBEBOT_SANDBOX_RUNTIME") or "container"
PASSWORD = secrets.token_urlsafe(24)


def free_port():
    with socket.socket() as probe:
        probe.bind(("127.0.0.1", 0))
        return probe.getsockname()[1]


def docker(*args, check=True):
    return subprocess.run(["docker", *args], check=check, cwd=ROOT)


def publish(registry):
    for image in ("orchestrator", "web", "agent"):
        reference = f"{registry}/cubebot-{image}:{TAG}"
        docker("build", "--tag", reference, "--file", f"apps/{image}/Dockerfile", ".")
        docker("push", reference)
    if RUNTIME == "kvm":
        subprocess.run([sys.executable, ROOT / "tooling/guest/build.py"], check=True)
        reference = f"{registry}/cubebot-sandbox:{TAG}"
        docker(
            "build",
            "--tag",
            reference,
            "--build-context",
            "guest=target/guest",
            "--file",
            "apps/sandbox/Dockerfile",
            ".",
        )
        docker("push", reference)


def install(environment):
    subprocess.run(
        ["bash", str(ROOT / "deploy/install.sh")],
        env=environment | {"CUBEBOT_YES": "1"},
        stdin=subprocess.DEVNULL,
        check=True,
        cwd=ROOT,
    )


def password(home):
    for line in (home / ".env").read_text().splitlines():
        if line.startswith("POSTGRES_PASSWORD="):
            return line
    raise AssertionError(".env has no POSTGRES_PASSWORD")


def console(environment):
    return f"http://localhost:{environment['CUBEBOT_PORT']}"


def sign_in(environment, home, path="/api/identity/login"):
    api = Api(console(environment), home / "dev-ca/ca.crt")
    api.token = api(path, {"login": "admin", "password": PASSWORD})["token"]
    return api


def set_up(environment, home):
    api = Api(console(environment), home / "dev-ca/ca.crt")
    assert api("/api/identity/setup") == {"open": True}
    api = sign_in(environment, home, "/api/identity/setup")
    assert api("/api/identity/setup") == {"open": False}
    assert api("/api/identity/me")["administrator"]
    return api


def project_volumes(name):
    return subprocess.run(
        [
            "docker",
            "volume",
            "ls",
            "--quiet",
            "--filter",
            f"label=com.docker.compose.project={name}",
        ],
        capture_output=True,
        text=True,
        check=True,
    ).stdout.split()


def remove_volumes(name):
    volumes = project_volumes(name)
    if volumes:
        docker("volume", "rm", *volumes, check=False)


def uninstall(environment, home, registry, name):
    subprocess.run(
        [str(home / "cubebot"), "uninstall", "--yes"], env=environment, check=True
    )
    assert not home.exists(), f"{home} is still there"
    assert project_volumes(name) == [], project_volumes(name)
    for image in ("orchestrator", "web", "agent"):
        reference = f"{registry}/cubebot-{image}:{TAG}"
        found = subprocess.run(
            ["docker", "image", "inspect", reference], capture_output=True, check=False
        )
        assert found.returncode != 0, f"{reference} is still there"
    print(f"PASS cubebot uninstall removed {home}, the volumes and the images")


def interactive(environment, directory, registry, name):
    """Answers every prompt the installer shows, as a person at a terminal would."""
    user = directory / "user"
    user.mkdir()
    home = user / ".cubebot"
    environment = {
        key: value
        for key, value in environment.items()
        if key not in ("CUBEBOT_HOME", "CUBEBOT_RUNTIME")
    } | {
        "HOME": str(user),
        # Docker's contexts stay where the CLI found them.
        "DOCKER_CONFIG": os.environ.get("DOCKER_CONFIG", str(Path.home() / ".docker")),
    }
    try:
        answer(environment, home)
    except BaseException:
        if (home / "cubebot").exists():
            subprocess.run(
                [str(home / "cubebot"), "uninstall", "--yes"],
                env=environment,
                check=False,
            )
        raise
    uninstall(environment, home, registry, name)


def answer(environment, home):
    terminal = Terminal("bash", f"exec bash {ROOT / 'deploy/install.sh'}", environment)
    terminal.read_until("Where should agents run?", timeout=120)
    terminal.read_until("Enter")
    terminal.send(("1" if RUNTIME == "kvm" else "2") + "\r")
    terminal.read_until("Install directory")
    terminal.read_until("›")
    terminal.send(f"{home}\r")
    terminal.read_until("Administrator")
    terminal.read_until("Enter")
    terminal.send("1\r")
    terminal.read_until("Administrator login")
    terminal.send("\r")
    terminal.read_until("Administrator password")
    terminal.send(PASSWORD + "\r")
    terminal.read_until("again")
    terminal.send(PASSWORD + "\r")
    terminal.read_until("Install?")
    terminal.send("\r")
    terminal.read_until("Cubebot is running", timeout=900)
    assert terminal.finish() == 0, terminal.output.decode(errors="replace")
    api = sign_in(environment, home)
    assert api("/api/identity/me")["administrator"]
    print("PASS the installer took every answer as keys; its administrator signed in")


def browser_setup(environment, home, wrapper, name):
    if not os.environ.get("CUBEBOT_PLAYWRIGHT_MODULE"):
        print("SKIP setup form: set CUBEBOT_PLAYWRIGHT_MODULE to check it in Chromium")
        return
    # A fresh database, so the console opens on the setup form again.
    subprocess.run(wrapper + ["down"], env=environment, check=True)
    remove_volumes(name)
    subprocess.run(wrapper + ["up"], env=environment, check=True)
    script = Path(__file__).with_name("install_setup_browser.mjs")
    browser = environment | {"CUBEBOT_CONSOLE": console(environment)}
    subprocess.run(["node", script], env=browser, check=True, timeout=300)
    assert Api(console(environment), home / "dev-ca/ca.crt")("/api/identity/setup") == {
        "open": False
    }
    print("PASS the setup form created the administrator and opened the console")


def qualify(environment, home, agents):
    install(environment)
    assert (home / ".env").stat().st_mode & 0o077 == 0, (
        "the installer's .env is readable by others"
    )
    api = set_up(environment, home)
    print(
        "PASS install.sh started the release; setup made the administrator and closed"
    )
    installation = api("/api/installation")
    assert installation["sandbox_runtime"] == RUNTIME, installation
    provider = command(
        expected_revision=0,
        api_key="install-check-key",
        model="install-check-model",
        provider="openai",
        context_window=128000,
    )
    api("/api/provider", provider)
    identity = agent(api, "Install check", INSTRUCTIONS)
    agents.append(identity)
    status = f"/api/agents/{identity}/status"
    wait_for(
        "authenticated runtime",
        lambda: api(status)["runtime_available"],
        timeout=300,
    )
    print("PASS one agent woke from the registry's image and authenticated")
    before = password(home)
    install(environment)
    assert password(home) == before, "install.sh replaced the database password"
    sign_in(environment, home)
    print("PASS running install.sh again kept the data and its password")


def main():
    registry_port = free_port()
    registry = f"localhost:{registry_port}"
    name = f"cubebot-install-check-{secrets.token_hex(4)}"
    started = time.monotonic()
    docker(
        "run",
        "--detach",
        "--name",
        name,
        "--publish",
        f"127.0.0.1:{registry_port}:5000",
        "registry:2",
    )
    (ROOT / "target").mkdir(exist_ok=True)
    with tempfile.TemporaryDirectory(
        prefix="cubebot-install-", dir=ROOT / "target"
    ) as directory:
        home = Path(directory) / "home"
        environment = os.environ | {
            "CUBEBOT_HOME": str(home),
            "CUBEBOT_VERSION": TAG,
            "CUBEBOT_REGISTRY": registry,
            "CUBEBOT_SOURCE": f"file://{ROOT}",
            "CUBEBOT_RUNTIME": RUNTIME,
            "CUBEBOT_PORT": str(free_port()),
            "POSTGRES_PORT": str(free_port()),
            # Its own project, so an installation on this machine is never touched.
            "COMPOSE_PROJECT_NAME": name,
        }
        wrapper = [str(home / "cubebot")]
        agents = []
        try:
            publish(registry)
            qualify(environment, home, agents)
            subprocess.run(wrapper + ["down"], env=environment, check=True)
            print("PASS cubebot down stopped the installation")
            browser_setup(environment, home, wrapper, name)
            uninstall(environment, home, registry, name)
            interactive(environment, Path(directory), registry, name)
        except BaseException:
            if (home / "cubebot").exists():
                subprocess.run(
                    wrapper + ["logs", "--tail", "80"], env=environment, check=False
                )
            raise
        finally:
            if (home / "cubebot").exists():
                subprocess.run(wrapper + ["down"], env=environment, check=False)
            # After `down`, so the orchestrator cannot start the agent again.
            for identity in agents:
                remove(identity)
            remove_volumes(name)
            docker("rm", "--force", "--volumes", name, check=False)
            for image in ("orchestrator", "web", "agent", "sandbox"):
                docker("rmi", f"{registry}/cubebot-{image}:{TAG}", check=False)
    print(f"install-check passed in {time.monotonic() - started:.0f} s")


if __name__ == "__main__":
    sys.exit(main())
