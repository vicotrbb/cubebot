"""Skills (ADRs 0055 to 0057) on real agents through the Docker sandbox backend.

Same host needs as docker_sandbox.py. An administrator and a member share one library.
The Operator agent loads an installed skill and runs its script as the tool user, stays
pinned to its revision while a newer one is published, moves only when updated, and
loses the skill when uninstalled. The Bystander agent never sees it. The Learner agent
is taught a recurring procedure, dreams it into a skill installed only for itself, and
keeps that revision while a member edits it. Assertions read the ledger and tool
events, never only the model's words.
"""

import json
import os
import platform
import secrets
import socket
import subprocess
import sys
import tempfile
import uuid
from pathlib import Path

from cluster_chat_setup import wait_for
from docker_dreams import dream_log, psql
from docker_sandbox import (
    ROOT,
    Api,
    bridge,
    build,
    command,
    configure,
    live,
    remove,
    sign_in,
)
from integration import run_database
from smoke import running

INSTRUCTIONS = (
    "You are an operations assistant with a bash workspace. Use your tools and skills "
    "when a request calls for them, and report command output exactly."
)
MARKER = secrets.token_hex(4)


def probe(revision, steps="Run the script and reply with exactly the line it prints."):
    entry = (
        "---\nname: probe-code\ndescription: Produces the team's probe code. Use it "
        f"whenever someone asks for the probe code.\n---\n{steps}\n\n"
        "Script: scripts/probe.sh (run it with bash).\n"
    )
    script = f'echo "PROBE-{MARKER}-r{revision} uid=$(id -u)"\n'
    return {"SKILL.md": entry, "scripts/probe.sh": script}


def agent(api, name, idle_delay_seconds):
    created = command(name=name, instructions=INSTRUCTIONS)
    identity = api("/api/agents", created, expected=201)["id"]
    current = api(f"/api/agents/{identity}")
    update = command(
        expected_revision=current["revision"],
        name=name,
        instructions=INSTRUCTIONS,
        idle_policy=current["idle_policy"],
        idle_delay_seconds=idle_delay_seconds,
        dreaming_model=None,
    )
    api(f"/api/agents/{identity}/definition", update)
    status = api(f"/api/agents/{identity}/status")
    wake = command(expected_revision=status["revision"], action="wake")
    api(f"/api/agents/{identity}/lifecycle", wake, expected=202)
    return identity


def chat(api, identity, message):
    path = f"/api/conversations/{identity}/messages"
    turn = api(path, command(message=message), expected=202)["id"]

    def complete():
        row = next((row for row in api(path) if row["id"] == turn), None)
        return row if row and row["status"] in ("complete", "failed") else None

    row = wait_for(f"the chat turn {message[:40]!r}", complete, timeout=300)
    assert row["status"] == "complete", row
    return turn, row


def tools(postgres, turn):
    """The turn's tool calls in order, each with its name, input and outcome text."""
    query = (
        f"SELECT payload::text FROM turn_events WHERE turn_id='{turn}' "
        "AND payload->>'kind' IN ('tool_call','tool_result') ORDER BY sequence"
    )
    rows = subprocess.run(
        ["docker", "exec", postgres, "psql", "-U", "cubebot", "-d", "cubebot_test"]
        + ["-Atc", query],
        capture_output=True,
        text=True,
        check=True,
    ).stdout.splitlines()
    calls = {}
    for event in map(json.loads, rows):
        if event["kind"] == "tool_call":
            calls[event["id"]] = {"name": event["name"], "input": event["input"]}
        else:
            calls[event["id"]]["outcome"] = json.dumps(event["outcome"])
    return list(calls.values())


def show(label, turn, row, calls):
    print(f"--- {label} turn {turn}")
    for call in calls:
        print(f"  {call['name']} {json.dumps(call['input'])[:160]}")
        print(f"    -> {call.get('outcome', '')[:240]}")
    print(f"  reply: {json.dumps(row)[:400]}")


def ask_probe(api, postgres, identity, label):
    turn, row = chat(
        api,
        identity,
        "What is the probe code right now? Use your probe-code skill if you have one; "
        "otherwise say you cannot.",
    )
    calls = tools(postgres, turn)
    show(label, turn, row, calls)
    return calls, row


def ran(calls, revision):
    """Whether a bash call printed the code of `revision` as a non-root tool user."""
    code = f"PROBE-{MARKER}-r{revision}"
    for call in calls:
        outcome = call.get("outcome", "")
        if call["name"] == "bash" and code in outcome:
            uid = outcome.split("uid=")[1].split("\\")[0].split('"')[0]
            assert uid.isdigit() and int(uid) != 0, outcome
            assert f"probe-code/{revision}" in json.dumps(call["input"]), call
            return True
    return False


def loads(calls):
    return [call for call in calls if call["name"] == "load_skill"]


def member(api, base, ca):
    login, password = "skills-member", secrets.token_urlsafe(24)
    body = command(login=login, password=password, administrator=False)
    api("/api/identity/accounts", body, expected=201)
    other = Api(base, ca)
    other.token = other("/api/identity/login", {"login": login, "password": password})[
        "token"
    ]
    return other, other("/api/identity/me")["id"]


def grant(api, account, agent_id, capability):
    body = command(
        recipient={"kind": "user", "id": account},
        scope={"kind": "agent", "id": agent_id},
        capability=capability,
    )
    api("/api/identity/access/grants", body, expected=201)


def library(admin, people, operator):
    """Human publication, revision conflicts, validation and per-agent authority."""
    other, account = people
    published = other("/api/skills", {"files": probe(1)}, expected=201)
    skill = published["id"]
    assert (published["name"], published["revision"]) == ("probe-code", 1)
    admin("/api/skills", {"files": probe(9)}, expected=409)
    escape = probe(1) | {"scripts/../../etc/cron.d/x": "x"}
    admin("/api/skills", {"files": escape | {"SKILL.md": "x"}}, expected=400)
    admin("/api/skills", {"files": {"scripts/x.sh": "x"}}, expected=400)
    renamed = probe(2) | {
        "SKILL.md": probe(2)["SKILL.md"].replace("probe-code", "other")
    }
    admin(
        f"/api/skills/{skill}", {"files": renamed, "expected_revision": 1}, 400, "PUT"
    )
    stale = {"files": probe(2), "expected_revision": 0}
    other(f"/api/skills/{skill}", stale, expected=409, method="PUT")

    installs = f"/api/agents/{operator}/skills"
    other(installs, expected=404)
    other(f"{installs}/{skill}", {"revision": 1}, expected=404, method="PUT")
    grant(admin, account, operator, "memory_read")
    assert other(installs) == []
    other(f"{installs}/{skill}", {"revision": 1}, expected=404, method="PUT")
    grant(admin, account, operator, "agent_manage")
    other(f"{installs}/{skill}", {"revision": 9}, expected=404, method="PUT")
    other(f"{installs}/{skill}", {"revision": 1}, expected=204, method="PUT")
    [installed] = other(installs)
    assert (installed["revision"], installed["current_revision"]) == (1, 1)
    return skill


def operator_uses(admin, other, postgres, operator, skill):
    """Runs the installed revision, pinned until updated and gone when removed."""
    calls, row = ask_probe(admin, postgres, operator, "operator r1")
    assert [call["outcome"] for call in loads(calls)], "load_skill was not called"
    assert ran(calls, 1), "the skill's script did not run from revision 1"
    assert f"PROBE-{MARKER}-r1" in json.dumps(row)

    revised = other(
        f"/api/skills/{skill}",
        {"files": probe(2), "expected_revision": 1},
        method="PUT",
    )
    assert revised["revision"] == 2
    [installed] = admin(f"/api/agents/{operator}/skills")
    assert (installed["revision"], installed["current_revision"]) == (1, 2)
    calls, _ = ask_probe(admin, postgres, operator, "operator pinned")
    assert ran(calls, 1) and not ran(calls, 2), "a publication moved the installation"

    path = f"/api/agents/{operator}/skills/{skill}"
    admin(path, {"revision": 2}, expected=204, method="PUT")
    calls, _ = ask_probe(admin, postgres, operator, "operator r2")
    assert ran(calls, 2) and not ran(calls, 1), "the update did not take effect"

    admin(path, expected=204, method="DELETE")
    assert admin(f"/api/agents/{operator}/skills") == []
    calls, _ = ask_probe(admin, postgres, operator, "operator uninstalled")
    assert not ran(calls, 1) and not ran(calls, 2), "an uninstalled skill still ran"
    for call in loads(calls):
        assert "No installed skill" in call["outcome"], call
    assert [row["name"] for row in admin("/api/skills")] == ["probe-code"]


def bystander_denied(admin, postgres, bystander):
    calls, _ = ask_probe(admin, postgres, bystander, "bystander")
    assert not ran(calls, 1) and not ran(calls, 2), "an uninstalled agent ran the skill"
    for call in loads(calls):
        assert "No installed skill" in call["outcome"], call
    assert admin(f"/api/agents/{bystander}/skills") == []


PROCEDURE = (
    "Our weekly staging-disk check, which you will run for me every Monday from now "
    "on: 1) run `df -P / | tail -1` and read the use percentage; 2) run "
    "`du -sh /tmp 2>/dev/null` to see what temporary files take; 3) if use is above "
    "80%, "
    "say CLEANUP NEEDED, otherwise say DISK OK, and always quote both outputs. Do the "
    "check now."
)


def dream_skill(admin, watch_runs, postgres, learner, tries=3):
    """Teaches a recurring procedure until a dream writes it down as a skill."""
    for attempt in range(tries):
        turn, row = chat(admin, learner, PROCEDURE)
        show(f"learner lesson {attempt}", turn, row, tools(postgres, turn))
        chat(admin, learner, "Thanks. Same check again next Monday, same steps.")
        # An outside memory change makes the next idle spell eligible to dream.
        note = {
            "content": f"The weekly staging-disk check happens on Mondays ({attempt})."
        }
        admin(f"/api/agents/{learner}/memories", note, expected=201)
        before = len(watch_runs())

        def finished(before=before):
            runs = watch_runs()
            done = [
                run for run in runs[: len(runs) - before] if run["status"] == "finished"
            ]
            return done[0] if done else None

        run = wait_for("a dream after the lesson", finished, timeout=420)
        print("learner dream:", json.dumps(run, indent=1)[:3000])
        if run["skills"]:
            return run
    raise AssertionError(f"no dream wrote a skill in {tries} lessons")


def learner_dreams(admin, other, postgres, learner, others):
    def runs():
        return admin(f"/api/agents/{learner}/dreams")

    run = dream_skill(admin, runs, postgres, learner)
    assert run["outcome"] == "completed", run
    written = run["skills"][0]
    tagged = psql(
        postgres,
        f"SELECT count(*) FROM skill_revisions WHERE dream_run_id='{run['id']}'",
    )
    assert int(tagged[0]) == len(run["skills"]), tagged
    revision = admin(f"/api/skills/{written['skill']}/revisions/{written['revision']}")
    assert (revision["dream_run"], revision["human"]) == (run["id"], None), revision
    assert revision["files"]["SKILL.md"] == written["content"]
    mine = {
        row["skill"]: row["revision"] for row in admin(f"/api/agents/{learner}/skills")
    }
    assert mine.get(written["skill"]) == written["revision"], mine
    for identity in others:
        installed = admin(f"/api/agents/{identity}/skills")
        assert written["skill"] not in [row["skill"] for row in installed], installed
    assert "-----BEGIN" not in json.dumps(revision["files"])

    # A member edits the dream's revision; the learner keeps the one it installed.
    current = next(s for s in admin("/api/skills") if s["id"] == written["skill"])
    # The description the system prompt lists is text, even from a YAML block value.
    assert len(current["description"]) > 10, current
    assert not current["description"].startswith(("|", ">")), current
    files = dict(revision["files"])
    files["SKILL.md"] += f"\nReviewed by a person: MEMBER-{MARKER}.\n"
    edit = {"files": files, "expected_revision": current["revision"]}
    edited = other(f"/api/skills/{written['skill']}", edit, method="PUT")
    assert edited["revision"] == current["revision"] + 1
    mine = {row["skill"]: row for row in admin(f"/api/agents/{learner}/skills")}
    pinned = mine[written["skill"]]
    assert (pinned["revision"], pinned["current_revision"]) == (
        written["revision"],
        edited["revision"],
    )
    ask = (
        f"Load your {written['name']} skill and tell me its first step. Do not run it."
    )
    turn, row = chat(admin, learner, ask)
    calls = tools(postgres, turn)
    show("learner pinned load", turn, row, calls)
    assert loads(calls), "the learner did not load its dreamed skill"
    assert all(f"MEMBER-{MARKER}" not in call["outcome"] for call in loads(calls))
    path = f"/api/agents/{learner}/skills/{written['skill']}"
    admin(path, {"revision": edited["revision"]}, expected=204, method="PUT")
    turn, row = chat(admin, learner, ask)
    calls = tools(postgres, turn)
    show("learner updated load", turn, row, calls)
    assert any(f"MEMBER-{MARKER}" in call["outcome"] for call in loads(calls)), calls


def suspended(admin, other, account):
    target = next(
        row for row in admin("/api/identity/accounts") if row["id"] == account
    )
    body = command(expected_revision=target["revision"], id=account)
    admin("/api/identity/suspend", body)
    other("/api/skills", expected=401)
    other("/api/skills", {"files": probe(5)}, expected=401)


def scenarios(admin, people, postgres):
    operator = agent(admin, "Operator", 86400)
    bystander = agent(admin, "Bystander", 86400)
    learner = agent(admin, "Learner", 60)
    identities = [operator, bystander, learner]
    other, account = people
    try:
        skill = library(admin, people, operator)
        operator_uses(admin, other, postgres, operator, skill)
        bystander_denied(admin, postgres, bystander)
        learner_dreams(admin, other, postgres, learner, [operator, bystander])
        suspended(admin, other, account)
    finally:
        for identity in identities:
            dream_log(identity)
            remove(identity)


def qualify(directory: Path):
    with socket.socket() as probe_socket:
        probe_socket.bind((bridge(), 0))
        address = f"{bridge()}:{probe_socket.getsockname()[1]}"
    environment = configure(directory, address)
    base, ca = f"https://{address}", directory / "ca/ca.crt"
    admin = Api(base, ca)

    def workflow(postgres, env):
        database = env["CUBEBOT_TEST_DATABASE_URL"]
        environment["CUBEBOT_DATABASE_URL"] = database
        binary = [str(ROOT / "target/debug/cubebot-orchestrator")]
        with running(binary, ROOT, environment):
            wait_for("orchestrator HTTPS", lambda: live(admin), timeout=30)
            sign_in(admin, database, "skills-admin", secrets.token_urlsafe(24))
            key = Path(os.environ["OPENAI_API_KEY_FILE"]).read_text().strip()
            provider = command(
                expected_revision=0,
                api_key=key,
                model="gpt-4.1-mini",
                provider="openai",
                context_window=128000,
            )
            admin("/api/provider", provider)
            scenarios(admin, member(admin, base, ca), postgres)
        print("PASS skills published, scoped, pinned, loaded, run and dreamed")

    with tempfile.TemporaryDirectory(
        prefix="cubebot-postgres-", dir=ROOT / "target"
    ) as data:
        password = secrets.token_hex(24)
        name = f"cubebot-test-{uuid.uuid4().hex}"
        env = os.environ | {"POSTGRES_PASSWORD": password}
        run_database(name, env, password, os.path.abspath(data), workflow)


def main():
    if sys.platform != "linux" or platform.machine() != "x86_64":
        print(f"docker-skills skipped: needs x86_64 Linux, this is {sys.platform}")
        return 0
    if not os.environ.get("OPENAI_API_KEY_FILE"):
        print("docker-skills failed: set OPENAI_API_KEY_FILE", file=sys.stderr)
        return 1
    build()
    with tempfile.TemporaryDirectory() as directory:
        qualify(Path(directory))
    return 0


if __name__ == "__main__":
    sys.exit(main())
