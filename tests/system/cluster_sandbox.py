"""docs/adr/sandbox.md#agent-centric-execution-in-a-sandbox-boundary's sandbox scenarios
on the kvm backend, in an owned kind cluster.

Needs x86_64 Linux with /dev/kvm, the kernel build tools of tooling/guest, the
cluster-chat prerequisites (Docker, the pinned kind, kubectl, OpenSSL), a fresh
CUBEBOT_QUALIFICATION_DIRECTORY outside the checkout and OPENAI_API_KEY_FILE.

The chart installs the sandboxes' egress and ingress NetworkPolicy. A real model chooses
when to run the commands it is given inside each guest; every assertion reads the
ledger's tool calls and results, Kubernetes or the node, never the model's reply.
Evidence goes to cluster-sandbox.json in the qualification directory.
"""

import base64
import json
import os
import time
from pathlib import Path

import cluster_admission as admission
import cluster_chat as chat
from cluster_chat_flow import (
    command,
    configure,
    lifecycle,
    outcome,
    pod,
    replacement,
    running,
    send,
    signal,
)
from cluster_chat_setup import CONTROL_PORT, NAMESPACE, available, wait_for

INSTRUCTIONS = (
    "You operate a test sandbox. Run each command you are given with one bash tool "
    "call, exactly as written and unchanged, in the order given, then reply with the "
    "output. Never rerun a command."
)
WORKSPACE_DISK = "/var/lib/cubebot/workspace.ext4"
# An OpenAI model that takes the `computer` tool; CUBEBOT_COMPUTER_MODEL overrides it.
COMPUTER_MODEL = "gpt-5.5"
COMPUTER_INSTRUCTIONS = (
    "You operate a Linux desktop through the computer tool. Look at the screen before "
    "and after you act. Answer only from what the screen shows."
)
COMPUTER_TASK = (
    "Chromium is open on your desktop. Click its address bar, go to "
    "https://example.com, then reply with the title on that page's browser tab "
    "exactly as it appears."
)
# As the tool user: the browser's profile, the old DevTools port, and the file
# descriptors of each process of the browser user (10002), which hold its DevTools pipe.
BROWSER_PROBE = (
    "ls /var/lib/cubebot/browser; echo profile=$?; "
    "bash -c 'exec 3<>/dev/tcp/127.0.0.1/9222' 2>/dev/null; echo devtools=$?; "
    "n=0; for p in /proc/[0-9]*; do grep -qs '^Uid:\\s10002\\s' $p/status || continue; "
    "n=$((n+1)); ls $p/fd >/dev/null 2>&1 && echo pipe=$p; done; echo browser=$n"
)


def operator(f, name, instructions=INSTRUCTIONS):
    created = f.request(
        "/api/agents", command(name=name, instructions=instructions), expected=201
    )
    lifecycle(f, created["id"], "start")
    return created["id"], running(f, created["id"])


def tools(f, turn):
    """The turn's tool calls in order, with command and outcome (None if unreported)."""
    rows = f.sql(
        "SELECT payload::text FROM turn_events WHERE turn_id='{}' AND payload->>'kind' "
        "IN ('tool_call','tool_result') ORDER BY sequence".format(turn["id"])
    )
    calls = {}
    for event in map(json.loads, rows.splitlines()):
        if event["kind"] == "tool_call":
            calls[event["id"]] = {
                "command": event["input"].get("command", ""),
                "outcome": None,
            }
        else:
            calls[event["id"]]["outcome"] = event["outcome"]
    return list(calls.values())


def called(f, turn, text, finished=None):
    """The one call whose command contains `text`, once it has (or lacks) an outcome."""
    matching = [call for call in tools(f, turn) if text in call["command"]]
    assert len(matching) <= 1, f"{text!r} ran {len(matching)} times"
    if not matching or (
        finished is not None and bool(matching[0]["outcome"]) != finished
    ):
        return None
    return matching[0]


def node(f, script):
    return admission.run(
        "docker", "exec", f"{f.name}-control-plane", "sh", "-c", script
    )


def vms(f, container=""):
    """Node PIDs holding a KVM VM, limited to one container's cgroup when given."""
    return set(
        node(
            f,
            "for p in /proc/[0-9]*; do ls -l $p/fd 2>/dev/null | grep -q kvm-vm && "
            # `true`: the loop otherwise exits with its last process's failed match.
            f'grep -qs "{container}" $p/cgroup && echo ${{p#/proc/}}; done; true',
        ).split()
    )


def container(current):
    return current["status"]["containerStatuses"][0]["containerID"].split("://", 1)[1]


def address(f, kind, name, namespace=None):
    item = json.loads(
        admission.kubectl(
            f.config,
            "-n",
            namespace or NAMESPACE,
            "get",
            kind,
            name,
            "-o",
            "json",
        )
    )
    return item["spec"]["clusterIP"] if kind == "service" else item["status"]["podIP"]


def probes(targets):
    """One bash command; each line says whether a TCP connection opened in 5s."""
    pairs = " ".join(f"{name}={target}" for name, target in targets.items())
    return (
        f"for t in {pairs}; do "
        'out=$(timeout 5 bash -c "exec 3<>/dev/tcp/${t#*=}" 2>&1); '
        'echo "probe ${t%%=*} exit=$? $out"; done'
    )


def reach(output):
    results = {}
    for line in output.splitlines():
        if line.startswith("probe "):
            name, code, *message = line.split(" ", 3)[1:]
            results[name] = (int(code.removeprefix("exit=")), " ".join(message))
    return results


def blocked(result):
    """Nothing answered: a refusal would mean the target's host was reached."""
    code, message = result
    return code != 0 and "refused" not in message.lower()


def vm(f, current, evidence):
    """The sandbox holds a KVM VM and no agent runs in the pod itself."""
    limits = current["spec"]["containers"][0]["resources"]["limits"]
    assert limits["devices.kubevirt.io/kvm"] == "1"
    assert current["spec"]["containers"][0]["args"] == ["run"]
    held = vms(f, container(current))
    assert len(held) == 1, held
    names = f.k(
        "exec", current["metadata"]["name"], "--", "sh", "-c", "cat /proc/[0-9]*/comm"
    ).split()
    assert "cubebot-sandbox" in names and "cubebot-agent" not in names, names
    log = f.k("logs", current["metadata"]["name"], "-c", "sandbox")
    assert "Linux version" in log and "Guest agent alive" in log
    evidence["vm"] = {"pod": current["metadata"]["uid"], "processes": names}
    print("PASS the agent runs inside a KVM guest, not in its pod", flush=True)


def isolation(f, identity, other, evidence):
    database = address(f, "pod", "postgres-0")
    targets = {
        "gateway": f"{f.controller()}/{CONTROL_PORT}",
        "sandbox": f"{other['status']['podIP']}/22",
        "database": f"{database}/5432",
        "database_service": f"{address(f, 'service', 'postgres')}/5432",
        "kubernetes_service": f"{address(f, 'service', 'kubernetes', 'default')}/443",
    }
    status = json.loads(
        admission.kubectl(
            f.config, "get", "node", f"{f.name}-control-plane", "-o", "json"
        )
    )
    internal = next(
        item["address"]
        for item in status["status"]["addresses"]
        if item["type"] == "InternalIP" and ":" not in item["address"]
    )
    targets["kubernetes_api"] = f"{internal}/6443"
    # Outside the policy the same addresses answer, so a block is the policy's doing.
    control = reach(
        f.k(
            "exec",
            "postgres-0",
            "--",
            "bash",
            "-c",
            probes({k: targets[k] for k in ("database", "kubernetes_api")}),
        )
    )
    assert all(code == 0 for code, _ in control.values()), control
    turn = send(f, identity, probes(targets))
    outcome(f, identity, turn)
    call = called(f, turn, "/dev/tcp", finished=True)
    assert call, "the probe command did not run"
    results = reach(call["outcome"]["output"])
    assert set(results) == set(targets), results
    assert results["gateway"][0] == 0, results
    for name in set(targets) - {"gateway"}:
        assert blocked(results[name]), (name, results[name])
    evidence["isolation"] = {"targets": targets, "guest": results, "control": control}
    print(
        "PASS a guest reaches its gateway but not another sandbox, the database "
        "or the Kubernetes API",
        flush=True,
    )


def orchestrator_restart(f, identity, current, evidence):
    turn = send(
        f, identity, "sleep 30; echo survived >> survived.txt; cat survived.txt"
    )
    # The model may split the line into several calls; each check holds either way.
    wait_for("command started", lambda: called(f, turn, "sleep 30", finished=False))
    controller = f.platform("orchestrator")
    signal(f, controller, "SIGKILL")
    wait_for("orchestrator recovery", lambda: available(f))
    outcome(f, identity, turn, timeout=240)
    effect = called(f, turn, "echo survived", finished=True)
    assert effect and effect["outcome"]["status"] == "exited", tools(f, turn)
    call = called(f, turn, "cat survived.txt", finished=True)
    assert call and call["outcome"]["output"].count("survived") == 1, tools(f, turn)
    done = f.sql(
        f"SELECT count(*) FROM turn_events WHERE turn_id='{turn['id']}' "
        "AND payload->>'kind'='done'"
    )
    assert done == "1", done
    assert pod(f, identity)["metadata"]["uid"] == current["metadata"]["uid"]
    evidence["orchestrator_restart"] = {"turn": turn["id"], "result": call["outcome"]}
    print(
        "PASS a running command survives an orchestrator SIGKILL and commits once",
        flush=True,
    )


def revocation(f, evidence):
    identity, _ = operator(f, "Revocation")
    turn = send(f, identity, "sleep 40; echo after-revocation")
    wait_for("command started", lambda: called(f, turn, "sleep 40", finished=False))
    attempt = f.sql(
        "SELECT attempt_id FROM chat_turns WHERE id='{}'".format(turn["id"])
    )
    revoked = f.k(
        "exec", "deploy/orchestrator", "--", "cubebot-admin", "revoke-runtime", identity
    )
    assert revoked == "Revoked agent credentials: 1", revoked
    access = f.sql(
        "SELECT active::text || ',' || recovery::text FROM runtime_session_access "
        f"WHERE attempt_id='{attempt}'"
    )
    assert access == "false,false", access
    time.sleep(50)
    late = f.sql(
        "SELECT payload->>'kind' || ':' || COALESCE(payload->'outcome'->>'status', '') "
        f"FROM turn_events e WHERE e.turn_id='{turn['id']}' AND "
        "e.created_at > (SELECT revoked_at FROM agent_runtime_sessions "
        f"WHERE attempt_id='{attempt}')"
    ).split()
    # The revoked attempt writes nothing; a replacement may report the call uncertain.
    assert late in ([], ["tool_result:uncertain"]), late
    result = called(f, turn, "sleep 40")["outcome"]
    assert result is None or result["status"] == "uncertain", result
    assert f.sql(f"SELECT status FROM chat_turns WHERE id='{turn['id']}'") != "complete"
    evidence["revocation"] = {"attempt": attempt, "writes_after_revocation": late}
    print("PASS a revoked credential loses work delivery and ledger writes", flush=True)


def stop(f, identity, current, evidence):
    held = vms(f, container(current))
    assert held
    lifecycle(f, identity, "stop")
    wait_for("sandbox removal", lambda: pod(f, identity) is None)
    remaining = vms(f)
    assert not held & remaining, (held, remaining)
    evidence["stop"] = {"vm_processes": sorted(held), "after": sorted(remaining)}
    print("PASS stopping the agent ends its VM", flush=True)


def lost_sandbox(f, identity, current, evidence):
    turn = send(
        f,
        identity,
        "First command: echo committed >> effects.txt\n"
        "Second command, after the first finishes: sleep 120",
    )
    wait_for(
        "committed effect",
        lambda: called(f, turn, "echo committed", finished=True)
        and called(f, turn, "sleep 120", finished=False),
    )
    # The guest's ext4 commits every 5 seconds; after that the effect is on the disk.
    time.sleep(10)
    signal(f, current, "SIGKILL")
    new = replacement(f, identity, current)
    wait_for("journaled outcome", lambda: called(f, turn, "sleep 120", finished=True))
    assert called(f, turn, "sleep 120")["outcome"]["status"] == "uncertain"
    assert called(f, turn, "echo committed")["outcome"]["status"] == "exited"
    duplicates = f.sql(
        f"SELECT count(*) FROM (SELECT payload->>'id' FROM turn_events WHERE "
        f"turn_id='{turn['id']}' GROUP BY payload->>'kind', payload->>'id' "
        "HAVING payload->>'kind' IN ('tool_call','tool_result') AND count(*) > 1) d"
    )
    assert duplicates == "0", duplicates
    waiting = outcome(f, identity, turn, ("waiting",))
    assert waiting["error"] == "execution_interrupted", waiting
    # An uncertain effect is never replayed, even on request.
    path = f"/api/conversations/{identity}/messages/{turn['id']}/retry"
    f.request(path, command(), expected=409)
    effects = f.k(
        "exec",
        new["metadata"]["name"],
        "--",
        "debugfs",
        "-c",
        "-R",
        "cat /workspace/effects.txt",
        WORKSPACE_DISK,
    )
    assert effects.split() == ["committed"], effects
    evidence["lost_sandbox"] = {
        "turn": turn["id"],
        "pods": [current["metadata"]["uid"], new["metadata"]["uid"]],
        "effects": effects,
    }
    print(
        "PASS a sandbox lost mid-task resumes from its journal without repeating "
        "its committed effect",
        flush=True,
    )


def computer_use(f, evidence):
    """The model opens Chromium and navigates; the ledger holds screenshots and clicks.

    Computer use is an installation setting, so it is switched on last: the controller
    configuration gains `computer_use_enabled`, the orchestrator restarts with it, and
    the provider moves to a model with the computer tool.
    """
    stored = json.loads(f.k("get", "configmap", "controller", "-o", "json"))
    settings = json.loads(stored["data"]["config.json"]) | {
        "computer_use_enabled": True
    }
    patch = {"data": {"config.json": json.dumps(settings)}}
    f.k("patch", "configmap", "controller", "--type", "merge", "-p", json.dumps(patch))
    wait_for(
        "the orchestrator's mounted configuration",
        lambda: "computer_use_enabled"
        in f.k(
            "exec", "deploy/orchestrator", "--", "cat", "/etc/controller/config.json"
        ),
    )
    signal(f, f.platform("orchestrator"), "SIGKILL")
    wait_for("orchestrator recovery", lambda: available(f))
    model = os.environ.get("CUBEBOT_COMPUTER_MODEL", COMPUTER_MODEL)
    configure(
        f, Path(os.environ["OPENAI_API_KEY_FILE"]).read_text().strip(), model=model
    )
    identity, current = operator(f, "Computer", COMPUTER_INSTRUCTIONS)
    environment = current["spec"]["containers"][0]["env"]
    assert {"name": "CUBEBOT_COMPUTER_ENABLED", "value": "1"} in environment
    name = current["metadata"]["name"]
    wait_for(
        "the guest desktop",
        lambda: "Sandbox desktop started" in f.k("logs", name, "-c", "sandbox"),
    )
    turn = send(f, identity, COMPUTER_TASK)
    result = outcome(f, identity, turn, timeout=900)
    calls, results = ledger(f, turn)
    screenshots = [r for r in results.values() if r["status"] == "screenshot"]
    assert screenshots, results
    for screenshot in screenshots:
        assert base64.b64decode(screenshot["image"])[:8] == b"\x89PNG\r\n\x1a\n"
    # Chromium starts focused on its address bar, so the model may type without a click.
    clicks = [call for call in calls if "click" in json.dumps(call["input"])]
    typed = [call for call in calls if "example.com" in json.dumps(call["input"])]
    assert typed, calls
    done = f.sql(
        f"SELECT payload->>'response' FROM turn_events WHERE turn_id='{turn['id']}' "
        "AND payload->>'kind'='done'"
    )
    assert "Example Domain" in done, done
    # The arbiter logged each call's input with its source and the call's ledger id.
    log = f.k("logs", name, "-c", "sandbox")
    events = [
        json.loads(line.split("Desktop event ", 1)[1])
        for line in log.splitlines()
        if "Desktop event " in line
    ]
    tagged = {event["call"] for event in events if event["source"] == "agent"}
    assert {call["id"] for call in calls} <= tagged, (calls, events)
    # The host channel, through the exec the orchestrator uses, serves the same screen.
    request = {
        "input": {
            "account": "00000000-0000-0000-0000-000000000001",
            "action": {"action": "screenshot"},
        }
    }
    host = json.loads(
        f.k(
            "exec",
            "-i",
            name,
            "-c",
            "sandbox",
            "--",
            "cubebot-sandbox",
            "desktop",
            input=json.dumps(request) + "\n",
        )
    )
    assert host["mode"] == "agent", host
    assert base64.b64decode(host["png"])[:8] == b"\x89PNG\r\n\x1a\n"
    probe = send(
        f, identity, f"Use the bash tool to run exactly, then stop: {BROWSER_PROBE}"
    )
    outcome(f, identity, probe, timeout=300)
    seen = called(f, probe, "echo browser=", finished=True)
    assert seen, tools(f, probe)
    output = seen["outcome"]["output"]
    assert "Permission denied" in output and "profile=2" in output, output
    assert "devtools=1" in output and "pipe=" not in output, output
    assert "browser=0" not in output, output
    evidence["computer_use"] = {
        "turn": turn["id"],
        "model": model,
        "status": result["status"],
        "calls": [call["input"] for call in calls],
        "screenshots": len(screenshots),
        "clicks": len(clicks),
        "response": done,
        "browser_probe": output,
    }
    print(
        "PASS the model opens Chromium and navigates; the ledger holds its "
        "screenshots and clicks; a tool command reads neither the browser's profile "
        "nor its DevTools",
        flush=True,
    )


def ledger(f, turn):
    """The turn's computer calls in order, and their outcomes by call id."""
    rows = f.sql(
        "SELECT payload::text FROM turn_events WHERE turn_id='{}' AND payload->>'kind' "
        "IN ('tool_call','tool_result') ORDER BY sequence".format(turn["id"])
    )
    calls, results = [], {}
    for event in map(json.loads, rows.splitlines()):
        if event["kind"] == "tool_call" and event["name"] == "computer":
            calls.append(event)
        elif event["kind"] == "tool_result":
            results[event["id"]] = event["outcome"]
    return calls, {id: results[id] for id in (c["id"] for c in calls) if id in results}


def qualify(f):
    configure(f, Path(os.environ["OPENAI_API_KEY_FILE"]).read_text().strip())
    evidence = {"cluster": f.name, "source": admission.fingerprint()}
    first, current = operator(f, "Sandbox one")
    second, other = operator(f, "Sandbox two")
    vm(f, current, evidence)
    isolation(f, first, other, evidence)
    orchestrator_restart(f, first, current, evidence)
    stop(f, second, other, evidence)
    revocation(f, evidence)
    lost_sandbox(f, first, current, evidence)
    computer_use(f, evidence)
    evidence["base_commit"] = admission.run("git", "rev-parse", "HEAD")
    f.save("cluster-sandbox.json", evidence)


if __name__ == "__main__":
    chat.main("cubebot-sandbox-", qualify=qualify, terminalEnabled=True)
