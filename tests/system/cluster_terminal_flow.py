"""Live-provider terminal qualification through the shipped control/runtime/tool path.

Set CUBEBOT_TERMINAL_MANUAL=1 to require a person to submit the first request
through the real website. The default API submission is not website acceptance.
"""

import json
import os
import time
from pathlib import Path

import cluster_chat_flow as chat
from cluster_chat_setup import ORIGIN, wait_for

PROMPT = (
    "Use the terminal tool to create ./terminal-qualification.sh. Initially make "
    "it print 41 and exit 7. Run that script and inspect the nonzero exit status. "
    "Diagnose the failure, fix the script to print 42 and exit 0, and run it again. "
    "Verify the actual stdout is 42 and status is zero. Keep the fixed script in "
    "the workspace. Run the script each time using exactly "
    "./terminal-qualification.sh as the entire terminal command, without echoing "
    "$? or appending commands. The tool returns the exit code itself. "
    "Do not claim success before observing the terminal results."
)


def execute(f, current, script):
    return f.k(
        "exec",
        current["metadata"]["name"],
        "-c",
        "tools",
        "--",
        "/usr/bin/timeout",
        "--kill-after=1",
        "10",
        "/usr/bin/prlimit",
        "--nproc=128:128",
        "--",
        "/usr/bin/setpriv",
        "--reuid=10002",
        "--regid=10002",
        "--clear-groups",
        "/bin/sh",
        "-c",
        script,
    )


def native_turn(f, identity, text):
    turn = chat.send(f, identity, text)
    result = chat.outcome(f, identity, turn, ("complete", "waiting"))
    assert result["status"] == "complete", result
    assert result["terminal"], result
    return result


def acceptance(f):
    identity = chat.agent(f, "Terminal qualification")
    if os.environ.get("CUBEBOT_TERMINAL_MANUAL") == "1":
        path = f.artifacts / "manual-entry.json"
        path.write_text(
            json.dumps(
                {
                    "origin": ORIGIN,
                    "login": "admin",
                    "password": f.password,
                    "agent": identity,
                    "prompt": PROMPT,
                }
            )
        )
        path.chmod(0o600)
        print(
            f"Manual website submission required; private instructions: {path}",
            flush=True,
        )
        turn = wait_for(
            "manual website terminal request",
            lambda: next(iter(chat.history(f, identity)), None),
            timeout=600,
        )
        result = chat.outcome(f, identity, turn, ("complete", "waiting"))
    else:
        result = native_turn(f, identity, PROMPT)
    assert result["status"] == "complete", result
    results = [item["outcome"] for item in result["terminal"] if item["outcome"]]
    assert any(item.get("code") == 7 for item in results), results
    assert any(
        item.get("code") == 0 and "42" in item.get("output", "") for item in results
    ), results
    current = chat.running(f, identity)
    assert len(current["spec"]["containers"]) == 2
    assert execute(f, current, "sh /home/agent/terminal-qualification.sh") == "42"
    f.save("terminal-acceptance.json", result)
    return identity, current


def secret_denial(f, identity, key):
    result = native_turn(
        f,
        identity,
        "Use terminal to run exactly: for p in /proc/1/environ "
        "/var/lib/cubebot/journal/state /run/cubebot/identity/token "
        "/var/lib/cubebot-executor/journal/state; do "
        'if test -r "$p"; then echo SECRET_ACCESS_FAILURE; exit 1; fi; '
        "done; echo secret-access-denied. Do not print any file contents.",
    )
    outputs = json.dumps(result)
    assert key not in outputs
    assert all(
        "SECRET_ACCESS_FAILURE" not in (tool["outcome"] or {}).get("output", "")
        for tool in result["terminal"]
    )
    assert any(
        item["outcome"]
        and item["outcome"].get("code") == 0
        and "secret-access-denied" in item["outcome"].get("output", "")
        for item in result["terminal"]
    )


def active_tool(f, identity, turn):
    def inspect():
        result = next(
            row for row in chat.history(f, identity) if row["id"] == turn["id"]
        )
        return (
            result
            if any(tool["status"] == "running" for tool in result["terminal"])
            else None
        )

    try:
        return wait_for("issued terminal command", inspect, timeout=50)
    finally:
        f.save("terminal-history.json", chat.history(f, identity))


def cancellation(f, identity, current):
    turn = chat.send(
        f,
        identity,
        "Use terminal with a 30 second limit to run: "
        "sleep 20; echo bad > /home/agent/cancel-escaped",
    )
    active_tool(f, identity, turn)
    f.request(
        f"/api/conversations/{identity}/messages/{turn['id']}/cancel", {}, expected=204
    )
    result = chat.outcome(f, identity, turn, ("cancelled",))
    wait_for(
        "cancelled executor result",
        lambda: next(
            (
                row
                for row in chat.history(f, identity)
                if row["id"] == turn["id"]
                and all(tool["outcome"] is not None for tool in row["terminal"])
            ),
            None,
        ),
        timeout=40,
    )
    assert result["response"] is None
    assert (
        execute(f, current, "test ! -e /home/agent/cancel-escaped && echo stopped")
        == "stopped"
    )


def partition_expiry(f, identity, current):
    turn = chat.send(
        f,
        identity,
        "Use terminal with a 3 second limit to run: "
        "sleep 20; echo bad > /home/agent/expiry-escaped",
    )
    active_tool(f, identity, turn)
    with chat.partition(f, current):
        time.sleep(5)
        assert (
            execute(
                f,
                current,
                "test ! -e /home/agent/expiry-escaped && echo bounded",
            )
            == "bounded"
        )
    result = chat.outcome(f, identity, turn, ("complete", "waiting"))
    assert any(
        tool["outcome"] and tool["outcome"]["status"] in ("stopped", "uncertain")
        for tool in result["terminal"]
    ), result
    if result["status"] == "waiting":
        f.request(
            f"/api/conversations/{identity}/messages/{turn['id']}/cancel",
            {},
            expected=204,
        )


def revocation(f, identity, current, key):
    turn = chat.send(
        f,
        identity,
        "Use terminal with seconds 30: "
        "sleep 20; echo bad > /home/agent/revocation-escaped",
    )
    active_tool(f, identity, turn)
    chat.configure(f, key, {"mode": "explicit_grants", "agents": []})
    result = chat.outcome(f, identity, turn, ("waiting", "cancelled"))
    assert result["response"] is None
    assert (
        execute(f, current, "test ! -e /home/agent/revocation-escaped && echo revoked")
        == "revoked"
    )
    chat.configure(f, key, {"mode": "all_agents"})
    f.request(
        f"/api/conversations/{identity}/messages/{turn['id']}/cancel", {}, expected=204
    )


def in_flight_crash(f, identity, current, role):
    from cluster_admission import run

    marker = "/home/agent/" + role + "-effects"
    turn = chat.send(
        f,
        identity,
        "Use terminal with seconds 30 to run exactly: echo once >> "
        + marker
        + "; sleep 20; echo escaped >> "
        + marker,
    )
    active_tool(f, identity, turn)
    wait_for(
        "observable command effect",
        lambda: execute(f, current, "if test -f " + marker + "; then echo started; fi")
        == "started",
    )
    status = next(
        item for item in current["status"]["containerStatuses"] if item["name"] == role
    )
    run(
        "docker",
        "exec",
        f"{f.name}-control-plane",
        "ctr",
        "-n",
        "k8s.io",
        "tasks",
        "kill",
        "--signal",
        "SIGKILL",
        status["containerID"].split("://", 1)[1],
    )
    new = chat.replacement(f, identity, current)
    result = chat.outcome(f, identity, turn, ("waiting",))
    assert result["response"] is None
    assert execute(f, new, "cat " + marker) == "once"
    f.request(
        f"/api/conversations/{identity}/messages/{turn['id']}/cancel", {}, expected=204
    )
    return new


def executor_crash(f, identity, current):
    from cluster_admission import run

    statuses = current["status"]["containerStatuses"]
    tools = next(item for item in statuses if item["name"] == "tools")
    run(
        "docker",
        "exec",
        f"{f.name}-control-plane",
        "ctr",
        "-n",
        "k8s.io",
        "tasks",
        "kill",
        "--signal",
        "SIGKILL",
        tools["containerID"].split("://", 1)[1],
    )
    new = chat.replacement(f, identity, current)
    assert execute(f, new, "sh /home/agent/terminal-qualification.sh") == "42"
    return new


def qualify(f):
    key = Path(os.environ["OPENAI_API_KEY_FILE"]).read_text().strip()
    chat.verify_tls(f)
    chat.configure(f, key)
    identity, current = acceptance(f)
    secret_denial(f, identity, key)
    chat.exclusive_storage(f, current)
    cancellation(f, identity, current)
    partition_expiry(f, identity, current)
    current = chat.runtime_crash(f, identity, current)
    assert execute(f, current, "sh /home/agent/terminal-qualification.sh") == "42"
    current = executor_crash(f, identity, current)
    revocation(f, identity, current, key)
    current = in_flight_crash(f, identity, current, "runtime")
    current = in_flight_crash(f, identity, current, "tools")
    assert key not in json.dumps(chat.history(f, identity))
    f.save(
        "terminal-summary.json",
        {
            "agent": identity,
            "manual_website": os.environ.get("CUBEBOT_TERMINAL_MANUAL") == "1",
            "provider": True,
            "retention": True,
            "cancellation": True,
            "partition_expiry": True,
        },
    )
    print(
        "PASS live terminal loop, denied secrets, cancellation, partition/expiry, "
        "runtime/executor replacement and CSI retention",
        flush=True,
    )
