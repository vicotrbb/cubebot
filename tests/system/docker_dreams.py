"""Dreaming on real agents through the Docker sandbox backend.

Same host needs as docker_sandbox.py.
"""

import json
import os
import secrets
import socket
import subprocess
import sys
import tempfile
import time
import uuid
from pathlib import Path

from cluster_chat_setup import wait_for
from docker_sandbox import (
    ROOT,
    Api,
    agent,
    bridge,
    build,
    command,
    configure,
    configure_provider,
    live,
    remove,
    sign_in,
    unsupported,
)
from integration import run_database
from smoke import running

INSTRUCTIONS = "You are a project assistant. Answer briefly. Never use tools."
SEEDS = [
    "The project deadline is November 22.",
    "Deadline for the project: Nov 22",
    "Alice prefers email for status updates.",
    "Alice no longer wants email; she prefers Slack for status updates.",
    "pg conn str in vault secret/app/db",
]


def psql(postgres, query):
    return subprocess.run(
        ["docker", "exec", postgres, "psql", "-U", "cubebot", "-d", "cubebot_test"]
        + ["-Atc", query],
        capture_output=True,
        text=True,
        check=True,
    ).stdout.split()


class Watch:
    def __init__(self, api, identity):
        self.api, self.identity, self.activities = api, identity, set()

    def runs(self):
        self.activities.add(self.api(f"/api/agents/{self.identity}")["activity"])
        return self.api(f"/api/agents/{self.identity}/dreams")

    def finished(self, count, timeout):
        def ready():
            runs = self.runs()
            done = [run for run in runs if run["status"] == "finished"]
            return runs if len(done) >= count else None

        return wait_for(f"{count} finished dream(s)", ready, timeout=timeout)

    def running(self, timeout):
        # Polled faster than wait_for: a run's model call lasts only seconds.
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            runs = self.runs()
            if any(run["status"] == "running" for run in runs):
                return next(run for run in runs if run["status"] == "running")
            time.sleep(0.2)
        raise RuntimeError("timed out waiting for a running dream")


def chat(api, identity, message):
    path = f"/api/conversations/{identity}/messages"
    turn = api(path, command(message=message), expected=202)["id"]

    def complete():
        row = next((row for row in api(path) if row["id"] == turn), None)
        return row if row and row["status"] in ("complete", "failed") else None

    row = wait_for("the chat turn", complete, timeout=180)
    assert row["status"] == "complete", row
    return turn


def ledger(postgres, identity, run):
    tagged = psql(
        postgres,
        f"SELECT count(*) FROM memory_revisions WHERE dream_run_id='{run}'",
    )
    foreign = psql(
        postgres,
        "SELECT count(*) FROM memory_revisions r JOIN memories m ON m.id=r.memory_id "
        f"WHERE r.dream_run_id='{run}' AND m.agent_id<>'{identity}'",
    )
    return int(tagged[0]), int(foreign[0])


def history(postgres, identity):
    rows = psql(
        postgres,
        "SELECT count(*) FROM memory_revisions r JOIN memories m ON m.id=r.memory_id "
        f"WHERE m.agent_id='{identity}'",
    )
    return int(rows[0])


def consolidates(api, postgres, dreamer):
    watch = Watch(api, dreamer)
    [first] = watch.finished(1, timeout=480)
    print("first run:", json.dumps(first, indent=1))
    assert first["outcome"] == "completed", first
    assert first["model"] == "gpt-4.1-mini", first
    assert first["revisions"], "the seeded duplicates were not consolidated"
    tagged, foreign = ledger(postgres, dreamer, first["id"])
    assert (tagged, foreign) == (len(first["revisions"]), 0), (tagged, foreign)
    # Never deleting: every seeded revision is still there beside the dream's.
    assert history(postgres, dreamer) == len(SEEDS) + tagged
    assert "dreaming" in watch.activities, watch.activities

    # A new idle spell dreams again only if something outside dreaming changed memory:
    # the turn's own memory extraction may or may not save a fact.
    chat(api, dreamer, "Reply with the single word ok.")
    time.sleep(100)
    outside = psql(
        postgres,
        "SELECT count(*) FROM memory_revisions r JOIN memories m ON m.id=r.memory_id "
        f"WHERE m.agent_id='{dreamer}' AND r.dream_run_id IS NULL "
        f"AND r.created_at > '{first['started_at']}'",
    )
    runs = watch.runs()
    print(f"outside changes after the first run: {outside[0]}, runs: {len(runs)}")
    assert len(runs) == (2 if int(outside[0]) else 1), runs
    assert api(f"/api/agents/{dreamer}")["activity"] == "idle"
    return watch


def yields(api, watch, dreamer):
    before = len(watch.runs())
    note = {"content": "Bob joined the team on October 29."}
    api(f"/api/agents/{dreamer}/memories", note, expected=201)
    chat(api, dreamer, "Reply with the single word ok.")
    run = watch.running(timeout=150)
    turn = chat(api, dreamer, "Reply with the single word ok.")
    stopped = next(item for item in watch.runs() if item["id"] == run["id"])
    print("yielded run:", json.dumps(stopped, indent=1))
    assert stopped["outcome"] == "yielded", stopped
    # The yielded run left nothing completed, so the next idle spell dreams again.
    runs = watch.finished(before + 2, timeout=150)
    assert runs[0]["outcome"] in ("completed", "no_op"), runs[0]
    return turn


def overrides(api, nano, broken):
    [run] = Watch(api, nano).finished(1, timeout=480)
    print("override run:", json.dumps(run, indent=1))
    assert run["model"] == "gpt-4o-mini", run
    assert run["outcome"] in ("completed", "no_op"), run
    [run] = Watch(api, broken).finished(1, timeout=120)
    assert (run["model"], run["outcome"]) == ("no-such-model", "failed"), run
    assert run["revisions"] == [], run


def dream_log(identity):
    label = f"label=cubebot.dev/agent={identity}"
    ids = subprocess.run(
        ["docker", "ps", "--all", "--quiet", "--filter", label],
        capture_output=True,
        text=True,
        check=False,
    ).stdout.split()
    for container in ids:
        logs = subprocess.run(
            ["docker", "logs", container], capture_output=True, text=True, check=False
        )
        lines = (logs.stdout + logs.stderr).splitlines()
        print(identity, *[line for line in lines if "ream" in line][-20:], sep="\n  ")


def scenarios(api, postgres):
    dreamer = agent(api, "Dreamer", INSTRUCTIONS, 60, memories=SEEDS)
    nano = agent(api, "Nano", INSTRUCTIONS, 60, "gpt-4o-mini", SEEDS[:2])
    broken = agent(api, "Broken", INSTRUCTIONS, 60, "no-such-model", SEEDS[:1])
    identities = [dreamer, nano, broken]
    try:
        watch = consolidates(api, postgres, dreamer)
        yields(api, watch, dreamer)
        overrides(api, nano, broken)
        listed = {row["id"]: row["activity"] for row in api("/api/agents")}
        assert all(listed[identity] in ("idle", "dreaming") for identity in identities)
    finally:
        for identity in identities:
            dream_log(identity)
            remove(identity)


def qualify(directory: Path):
    with socket.socket() as probe:
        probe.bind((bridge(), 0))
        address = f"{bridge()}:{probe.getsockname()[1]}"
    environment = configure(directory, address)
    api = Api(f"https://{address}", directory / "ca/ca.crt")

    def workflow(postgres, env):
        database = env["CUBEBOT_TEST_DATABASE_URL"]
        environment["CUBEBOT_DATABASE_URL"] = database
        binary = [str(ROOT / "target/debug/cubebot-orchestrator")]
        with running(binary, ROOT, environment):
            wait_for("orchestrator HTTPS", lambda: live(api), timeout=30)
            sign_in(api, database, "dream-admin", secrets.token_urlsafe(24))
            configure_provider(api)
            scenarios(api, postgres)
        print("PASS agents dreamed, yielded, honored overrides and kept history")

    with tempfile.TemporaryDirectory(
        prefix="cubebot-postgres-", dir=ROOT / "target"
    ) as data:
        password = secrets.token_hex(24)
        name = f"cubebot-test-{uuid.uuid4().hex}"
        env = os.environ | {"POSTGRES_PASSWORD": password}
        run_database(name, env, password, os.path.abspath(data), workflow)


def main():
    if (refused := unsupported("docker-dreams")) is not None:
        return refused
    build()
    with tempfile.TemporaryDirectory() as directory:
        qualify(Path(directory))
    return 0


if __name__ == "__main__":
    sys.exit(main())
