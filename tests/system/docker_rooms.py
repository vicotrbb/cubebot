"""Rooms, delegation, messaging, steering, cancellation and lanes on real agents.

Same host needs as docker_sandbox.py. Three agents share a room: Lead delegates, Analyst
runs shell commands, Scribe writes prose. Every check reads the ledger or the API (turn
status, children, steering, tool events, room routing), never only the model's words;
markers that only a real tool run or a real delivery can produce tie the words to the
work.
"""

import json
import os
import platform
import secrets
import socket
import ssl
import subprocess
import sys
import tempfile
import threading
import time
import urllib.request
import uuid
from pathlib import Path

from cluster_chat_setup import wait_for
from docker_desktop import CONSOLE, surroundings
from docker_dreams import psql
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
from docker_skills import show, tools
from integration import run_database
from smoke import running

MARK = secrets.token_hex(3).upper()
MODEL = os.environ.get("CUBEBOT_ROOMS_MODEL", "gpt-4.1-mini")
FINAL = ("complete", "failed", "cancelled")
ROLES = {
    "Lead": (
        "You lead a small team. When asked to delegate or message a teammate, use the "
        "delegate or send_message tool exactly as asked, then end your turn with a "
        "short note. When delegated outcomes come back, report them faithfully and "
        "quote outputs exactly."
    ),
    "Analyst": (
        "You run shell commands with your bash tool whenever asked and report their "
        "output exactly. Follow any steering you receive."
    ),
    "Scribe": "You write short prose answers. You never run shell commands.",
}


def agent(api, name, role=None):
    created = command(name=name, instructions=role or ROLES[name])
    identity = api("/api/agents", created, expected=201)["id"]
    status = api(f"/api/agents/{identity}/status")
    wake = command(expected_revision=status["revision"], action="wake")
    api(f"/api/agents/{identity}/lifecycle", wake, expected=202)
    return identity


def deputy(admin, room, ids, name):
    """A fresh agent in Lead's role, added to the room.

    A long chat history teaches a small model to answer "I have asked Analyst" without
    calling the tool, so each check that needs a real delegate call starts afresh.
    """
    identity = agent(admin, name, ROLES["Lead"])
    ids[name] = identity
    current = next(r for r in admin("/api/rooms") if r["id"] == room)["members"]
    body = {"agents": [m["id"] for m in current] + [identity]}
    admin(f"/api/rooms/{room}/members", body, method="PUT")
    return identity


def turns(api, agent_id):
    return api(f"/api/conversations/{agent_id}/messages")


def turn(api, agent_id, turn_id):
    return next(row for row in turns(api, agent_id) if row["id"] == turn_id)


def settled(api, agent_id, turn_id, label, timeout=420):
    def done():
        row = turn(api, agent_id, turn_id)
        return row if row["status"] in FINAL else None

    return wait_for(label, done, timeout=timeout)


def send(api, agent_id, message):
    path = f"/api/conversations/{agent_id}/messages"
    return api(path, command(message=message), expected=202)["id"]


def post(api, room, message, request=None):
    body = {"request_id": request or str(uuid.uuid4()), "message": message}
    return api(f"/api/rooms/{room}/messages", body, expected=202)


def room_message(api, room, message_id):
    rows = api(f"/api/rooms/{room}/messages")
    return next(row for row in rows if row["id"] == message_id)


def routed(api, room, message_id, label):
    def done():
        row = room_message(api, room, message_id)
        open_work = any(d["status"] not in FINAL for d in row["deliveries"])
        return row if row["routing"] != "routing" and not open_work else None

    return wait_for(label, done, timeout=420)


def steer(api, agent_id, turn_id, message):
    path = f"/api/conversations/{agent_id}/messages/{turn_id}/steer"
    return api(path, command(message=message), expected=202)


def cancel(api, agent_id, turn_id):
    path = f"/api/conversations/{agent_id}/messages/{turn_id}/cancel"
    api(path, expected=204, method="POST")


def child(api, agent_id, turn_id, label):
    def found():
        children = turn(api, agent_id, turn_id)["children"]
        return children[0] if children else None

    return wait_for(label, found, timeout=300)


def running_tool(postgres, turn_id, label):
    query = (
        f"SELECT count(*) FROM turn_events c WHERE c.turn_id='{turn_id}' AND "
        "c.payload->>'kind'='tool_call' AND NOT EXISTS(SELECT 1 FROM turn_events r "
        "WHERE "
        "r.turn_id=c.turn_id AND r.payload->>'kind'='tool_result' AND "
        "r.payload->>'id'=c.payload->>'id')"
    )
    wait_for(label, lambda: psql(postgres, query) != ["0"], timeout=300)


def report(label, api, postgres, agent_id, turn_id):
    row = turn(api, agent_id, turn_id)
    show(
        label,
        turn_id,
        {k: row[k] for k in ("status", "response", "error")},
        tools(postgres, turn_id),
    )
    return row


def mentions(admin, room, ids):
    request = str(uuid.uuid4())
    text = "@Analyst and @Scribe: each of you reply with only your own name."
    first = post(admin, room, text, request)
    assert post(admin, room, text, request)["id"] == first["id"], "retry is one message"
    assert first["routing"] == "routed"
    row = routed(admin, room, first["id"], "both mentioned agents to answer")
    by_agent = {d["agent"]: d for d in row["deliveries"]}
    assert sorted(by_agent) == ["Analyst", "Scribe"], row
    for name, delivery in by_agent.items():
        assert delivery["status"] == "complete", delivery
        # Each knows its own name: it answers with that name alone, not another's.
        reply = delivery["response"].strip().strip(".").lower()
        assert reply == name.lower(), delivery
    assert not any(d["agent_id"] == ids["Lead"] for d in row["deliveries"])
    print("PASS mentions: one delivery per mentioned member, retry deduplicated")


def coordinated(admin, postgres, room, ids):
    text = (
        f"Someone please run the shell command `echo ROUTE-{MARK}` "
        "and tell me its output."
    )
    message = post(admin, room, text)
    assert message["routing"] == "routing" and message["deliveries"] == []
    row = routed(admin, room, message["id"], "the coordinator to route the request")
    assert row["routing"] == "routed", row
    [delivery] = row["deliveries"]
    calls = tools(postgres, delivery["turn"])
    show("coordinated", delivery["turn"], delivery, calls)
    assert delivery["agent"] == "Analyst", f"routed to {delivery['agent']}"
    assert any(
        f"ROUTE-{MARK}" in c.get("outcome", "") for c in calls if c["name"] == "bash"
    )
    assert f"ROUTE-{MARK}" in delivery["response"]
    chatter = routed(
        admin, room, post(admin, room, "Thanks, everyone!")["id"], "chatter"
    )
    assert (
        chatter["routing"] in ("no_action", "clarify") and not chatter["deliveries"]
    ), chatter
    print(f"PASS coordinator: one owner chosen, chatter -> {chatter['routing']}")


def room_context(admin, postgres, room):
    ask = post(
        admin,
        room,
        "@Scribe what exactly did the shell command print earlier in this room?",
    )
    row = routed(admin, room, ask["id"], "Scribe to answer from room context")
    [delivery] = row["deliveries"]
    assert not tools(postgres, delivery["turn"]), "Scribe ran no tool"
    assert f"ROUTE-{MARK}" in delivery["response"], delivery
    stored = turn(admin, delivery["agent_id"], delivery["turn"])["message"]
    assert "ROUTE-" not in stored, "room history is context, not stored in the request"
    print("PASS room context: earlier messages and replies reach the delivery only")


def delegation(admin, postgres, ids):
    lead = send(
        admin,
        ids["Lead"],
        f"Use the delegate tool to ask Analyst to run `echo DELEG-{MARK}` with bash "
        "and "
        "report the output. When the outcome comes back, tell me exactly what it "
        "printed.",
    )
    task = child(admin, ids["Lead"], lead, "Lead to delegate")
    assert task["awaited"] and task["agent"] == "Analyst", task
    row = settled(admin, ids["Lead"], lead, "Lead to resume and finish")
    report("lead delegation", admin, postgres, ids["Lead"], lead)
    analyst = report("analyst task", admin, postgres, ids["Analyst"], task["id"])
    assert (
        analyst["origin"]["kind"] == "delegation"
        and analyst["origin"]["name"] == "Lead"
    )
    assert any(c["name"] == "bash" for c in tools(postgres, task["id"]))
    assert row["status"] == "complete" and f"DELEG-{MARK}" in row["response"], row
    calls = tools(postgres, lead)
    assert [c["name"] for c in calls].count("delegate") == 1, calls
    print(
        "PASS delegation: Lead yielded, Analyst ran it in a task, Lead resumed with it"
    )


def cancelled_child(admin, postgres, room, ids):
    lead_agent = deputy(admin, room, ids, "Deputy")
    lead = send(
        admin,
        lead_agent,
        "Use the delegate tool to ask Analyst to run `sleep 120 && echo SLOW` with "
        "bash. "
        "When the outcome comes back, tell me plainly whether it succeeded.",
    )
    task = child(admin, lead_agent, lead, "Lead to delegate the slow task")
    running_tool(postgres, task["id"], "Analyst to start sleeping")
    cancel(admin, ids["Analyst"], task["id"])
    row = settled(admin, lead_agent, lead, "Lead to adapt to the cancelled child")
    report("lead after cancelled child", admin, postgres, lead_agent, lead)
    analyst = settled(admin, ids["Analyst"], task["id"], "the child to stop")
    assert analyst["status"] == "cancelled"
    stopped = [c for c in tools(postgres, task["id"]) if c["name"] == "bash"]
    assert stopped and '"stopped"' in stopped[0]["outcome"], stopped
    assert row["status"] == "complete", row
    assert "SLOW" not in row["response"] or "not" in row["response"].lower(), row
    print("PASS child cancel: the command stopped and Lead resumed with the outcome")


def cancelled_root(admin, postgres, room, ids):
    lead_agent = deputy(admin, room, ids, "Stopper")
    lead = send(
        admin,
        lead_agent,
        "Use the delegate tool to ask Analyst to run `sleep 120 && echo ROOT` with "
        "bash.",
    )
    task = child(admin, lead_agent, lead, "Lead to delegate the root task")
    running_tool(postgres, task["id"], "Analyst to start the root task")
    cancel(admin, lead_agent, lead)
    analyst = settled(admin, ids["Analyst"], task["id"], "the descendant to stop")
    assert analyst["status"] == "cancelled", analyst
    time.sleep(10)
    assert turn(admin, lead_agent, lead)["status"] == "cancelled", (
        "no resume after a stop"
    )
    print(
        "PASS subtree cancel: stopping Lead stopped Analyst's task and nothing resumed"
    )


def steering(admin, postgres, ids):
    work = send(
        admin,
        ids["Analyst"],
        "Run `sleep 20` with bash. After it finishes, reply with only the word APPLE.",
    )
    running_tool(postgres, work, "Analyst to start sleeping")
    note = steer(
        admin,
        ids["Analyst"],
        work,
        "Change of plan: reply with only the word BANANA instead.",
    )
    assert not note["incorporated"]
    row = settled(admin, ids["Analyst"], work, "the steered turn")
    report("steered", admin, postgres, ids["Analyst"], work)
    assert row["steering"][0]["incorporated"], row["steering"]
    assert (
        "BANANA" in row["response"].upper() and "APPLE" not in row["response"].upper()
    ), row
    print("PASS steering: incorporated at the tool boundary and followed")


def propagation(admin, postgres, ids):
    lead = send(
        admin,
        ids["Lead"],
        "Use the delegate tool to give Analyst exactly this task: 'Run `sleep 25` with "
        "bash. Then reply with the codeword you were given by steering, or NONE if "
        "none.' "
        "When the outcome comes back, tell me Analyst's answer exactly.",
    )
    task = child(admin, ids["Lead"], lead, "Lead to delegate the codeword task")
    running_tool(postgres, task["id"], "Analyst to start the codeword task")
    steer(admin, ids["Lead"], lead, f"The codeword is KIWI-{MARK}.")
    row = settled(admin, ids["Lead"], lead, "Lead to report the codeword")
    analyst = report("propagated task", admin, postgres, ids["Analyst"], task["id"])
    report("lead with codeword", admin, postgres, ids["Lead"], lead)
    copied = analyst["steering"]
    assert copied and copied[0]["propagated"] and copied[0]["incorporated"], copied
    assert f"KIWI-{MARK}" in analyst["response"], analyst
    assert f"KIWI-{MARK}" in row["response"], row
    print("PASS propagation: steering the parent reached and redirected its task")


def relay(admin, postgres, ids):
    before = len(turns(admin, ids["Scribe"]))
    lead = send(
        admin,
        ids["Lead"],
        "Use send_message with expects_reply true to ask Scribe for one color name. "
        "Then "
        "end your turn without waiting.",
    )
    row = settled(admin, ids["Lead"], lead, "Lead to send the message")
    [message] = row["children"]
    assert not message["awaited"] and message["agent"] == "Scribe", message
    settled(admin, ids["Scribe"], message["id"], "Scribe to answer")

    def relayed():
        return next(
            (
                t
                for t in turns(admin, ids["Lead"])
                if t["origin"] and t["origin"]["parent_turn_id"] == message["id"]
            ),
            None,
        )

    reply = wait_for("the reply to reach Lead", relayed, timeout=300)
    assert reply["message"].startswith("Reply from agent Scribe to your message:"), (
        reply
    )
    settled(admin, ids["Lead"], reply["id"], "Lead to read the reply")
    time.sleep(15)
    assert len(turns(admin, ids["Scribe"])) == before + 1, (
        "the relay echoed back to Scribe"
    )
    print("PASS messaging: the reply came back once and nothing echoed")


def lanes(admin, postgres, room, ids):
    fanout = deputy(admin, room, ids, "Fanout")
    lead = send(
        admin,
        fanout,
        "In one response, call the delegate tool three times in parallel, asking "
        "Analyst "
        "to run `sleep 20 && echo LANE-1`, `sleep 20 && echo LANE-2` and "
        "`sleep 20 && echo LANE-3` with bash, one command per task. Once all three "
        "outcomes "
        "are back, list every output exactly.",
    )
    concurrent, queued_while_full = 0, False
    query = (
        f"SELECT status FROM chat_turns WHERE conversation_id='{ids['Analyst']}' "
        f"AND awaited AND parent_turn_id='{lead}'"
    )
    deadline = time.monotonic() + 420
    while time.monotonic() < deadline:
        states = psql(postgres, query)
        running = states.count("running")
        concurrent = max(concurrent, running)
        queued_while_full |= running == 2 and "queued" in states
        if turn(admin, fanout, lead)["status"] in FINAL:
            break
        time.sleep(1)
    row = report("lanes", admin, postgres, fanout, lead)
    # Each task once; the model may add a request of its own; repeats are refused.
    messages = [
        turn(admin, ids["Analyst"], c["id"])["message"] for c in row["children"]
    ]
    for n in (1, 2, 3):
        alone = [m for m in messages if m.count("LANE-") == 1 and f"LANE-{n}" in m]
        assert len(alone) == 1, (n, messages)
    assert len(row["children"]) <= 4, row["children"]
    assert concurrent == 2 and queued_while_full, (concurrent, queued_while_full)
    for n in (1, 2, 3):
        assert f"LANE-{n}" in row["response"], row
    print(
        "PASS lanes: two tasks ran at once, the third waited, Lead collected all three"
    )


def continued(admin, postgres, room):
    task = post(
        admin,
        room,
        "@Analyst run `sleep 30` with bash, then reply with the number you are told "
        "afterwards, or ZERO if you are told none.",
    )
    delivery = task["deliveries"][0]
    running_tool(postgres, delivery["turn"], "Analyst to start the room task")
    follow = post(
        admin,
        room,
        "Update for Analyst's task that is running now: the number is 4242.",
    )
    routing = routed(admin, room, follow["id"], "the coordinator to continue the task")
    assert routing["routing"] == "continued" and routing["note"] == "Analyst", routing
    row = routed(admin, room, task["id"], "the continued task")["deliveries"][0]
    assert "4242" in row["response"], row
    print("PASS coordinator continue: the follow-up steered the open task")


def members(admin, people, postgres, room, ids):
    other, account = people
    asker = deputy(admin, room, ids, "Asker")
    body = {"agents": [asker, ids["Analyst"]]}
    other(f"/api/rooms/{room}/members", body, expected=404, method="PUT")
    grant(admin, account, room, "conversation_read")
    assert other(f"/api/rooms/{room}/messages")
    other(f"/api/rooms/{room}/members", body, expected=403, method="PUT")
    post_body = {"request_id": str(uuid.uuid4()), "message": "@Scribe hi"}
    other(f"/api/rooms/{room}/messages", post_body, expected=404)
    grant(admin, account, room, "room_manage")
    changed = other(f"/api/rooms/{room}/members", body, method="PUT")
    assert sorted(m["name"] for m in changed["members"]) == ["Analyst", "Asker"], (
        changed
    )
    assert changed["manage"]
    lead = send(admin, asker, "Use the delegate tool to ask Scribe to say hello.")
    settled(admin, asker, lead, "Lead to try a removed member")
    calls = tools(postgres, lead)
    refused = [c for c in calls if c["name"] == "delegate"]
    assert refused and "shares a room with you" in refused[0]["outcome"], calls
    assert not turn(admin, asker, lead)["children"]
    print("PASS membership: scoped to room.manage; a removed member is unreachable")


def grant(api, account, room, capability):
    body = command(
        recipient={"kind": "user", "id": account},
        scope={"kind": "room", "id": room},
        capability=capability,
    )
    api("/api/identity/access/grants", body, expected=201)


def stream(api, room, frames, stop):
    """Collects the room's server-sent `message` frames until stopped."""
    request = urllib.request.Request(
        f"{api.base}/api/rooms/{room}/events",
        headers={"Authorization": f"Bearer {api.token}", "Accept": "text/event-stream"},
    )
    context = ssl.create_default_context(cafile=str(api.ca))
    with urllib.request.urlopen(request, timeout=30, context=context) as response:
        event = None
        while not stop.is_set():
            line = response.readline().decode().rstrip("\n")
            if line.startswith("event:"):
                event = line.split(":", 1)[1].strip()
            elif line.startswith("data:") and event == "message":
                frames.append(json.loads(line.split(":", 1)[1]))


def streamed(admin, room):
    frames, stop = [], threading.Event()
    reader = threading.Thread(
        target=stream, args=(admin, room, frames, stop), daemon=True
    )
    reader.start()
    time.sleep(2)
    message = post(admin, room, "@Scribe reply with only the word STREAMED.")
    routed(admin, room, message["id"], "the streamed message")
    time.sleep(3)
    stop.set()
    mine = [f for f in frames if f["id"] == message["id"]]
    assert mine, f"no frame for the message among {len(frames)}"
    final = mine[-1]["deliveries"][0]
    assert final["status"] == "complete" and "STREAMED" in final["response"].upper(), (
        mine[-1]
    )
    print(
        f"PASS room stream: {len(mine)} snapshots of the message, the last with the "
        "reply"
    )


def console(password, address, ca):
    """Rooms, delegation and steering in the console's Chromium; needs Playwright."""
    if not os.environ.get("CUBEBOT_PLAYWRIGHT_MODULE"):
        print("SKIP console: set CUBEBOT_PLAYWRIGHT_MODULE to check it in Chromium")
        return
    evidence = Path(os.environ.get("CUBEBOT_ROOMS_EVIDENCE", "/tmp/cubebot-rooms"))
    evidence.mkdir(parents=True, exist_ok=True)
    names = surroundings({"evidence": evidence}, address, ca)
    try:
        wait_for("the console", lambda: live(Api(CONSOLE, ca)), timeout=60)
        environment = os.environ | {
            "CUBEBOT_CONSOLE": CONSOLE,
            "CUBEBOT_CONSOLE_LOGIN": "rooms-admin",
            "CUBEBOT_CONSOLE_PASSWORD": password,
            "CUBEBOT_ROOMS_MARK": MARK,
            "CUBEBOT_EVIDENCE_DIRECTORY": str(evidence),
        }
        script = Path(__file__).with_name("docker_rooms_browser.mjs")
        result = subprocess.run(
            ["node", script],
            env=environment,
            capture_output=True,
            text=True,
            timeout=900,
        )
        print(result.stdout + result.stderr)
        assert result.returncode == 0, "the console check failed"
    finally:
        subprocess.run(["docker", "rm", "-f", *names], check=False, capture_output=True)
    print(f"PASS console: rooms, streaming, delegation and steering views; {evidence}")


def member(admin, base, ca):
    login, password = "rooms-member", secrets.token_urlsafe(24)
    body = command(login=login, password=password, administrator=False)
    admin("/api/identity/accounts", body, expected=201)
    other = Api(base, ca)
    other.token = other("/api/identity/login", {"login": login, "password": password})[
        "token"
    ]
    return other, other("/api/identity/me")["id"]


def scenarios(admin, people, postgres, browser):
    ids = {name: agent(admin, name) for name in ROLES}
    try:
        body = command(name="Launch", agents=list(ids.values()))
        room = admin("/api/rooms", body, expected=201)["id"]
        steps = {
            "mentions": lambda: mentions(admin, room, ids),
            "coordinated": lambda: coordinated(admin, postgres, room, ids),
            "room_context": lambda: room_context(admin, postgres, room),
            "streamed": lambda: streamed(admin, room),
            "delegation": lambda: delegation(admin, postgres, ids),
            "steering": lambda: steering(admin, postgres, ids),
            "propagation": lambda: propagation(admin, postgres, ids),
            "relay": lambda: relay(admin, postgres, ids),
            "lanes": lambda: lanes(admin, postgres, room, ids),
            "cancelled_child": lambda: cancelled_child(admin, postgres, room, ids),
            "cancelled_root": lambda: cancelled_root(admin, postgres, room, ids),
            "continued": lambda: continued(admin, postgres, room),
            "console": lambda: console(*browser),
            "members": lambda: members(admin, people, postgres, room, ids),
        }
        only = os.environ.get("CUBEBOT_ROOMS_ONLY", "").split(",")
        failures = []
        for name, step in steps.items():
            if only != [""] and name not in only:
                continue
            try:
                step()
            except Exception as error:  # noqa: BLE001 - run every scenario, then fail
                failures.append(f"{name}: {error}")
                print(f"FAIL {name}: {error}", flush=True)
        assert not failures, failures
    finally:
        for identity in ids.values():
            remove(identity)


def qualify(directory: Path):
    with socket.socket() as probe_socket:
        probe_socket.bind((bridge(), 0))
        address = f"{bridge()}:{probe_socket.getsockname()[1]}"
    environment = configure(directory, address)
    environment["CUBEBOT_PUBLIC_ORIGIN"] = CONSOLE
    base, ca = f"https://{address}", directory / "ca/ca.crt"
    admin = Api(base, ca)
    admin.ca = ca

    def workflow(postgres, env):
        database = env["CUBEBOT_TEST_DATABASE_URL"]
        environment["CUBEBOT_DATABASE_URL"] = database
        binary = [str(ROOT / "target/debug/cubebot-orchestrator")]
        with running(binary, ROOT, environment):
            wait_for("orchestrator HTTPS", lambda: live(admin), timeout=30)
            password = secrets.token_urlsafe(24)
            sign_in(admin, database, "rooms-admin", password)
            key = Path(os.environ["OPENAI_API_KEY_FILE"]).read_text().strip()
            provider = command(
                expected_revision=0,
                api_key=key,
                model=MODEL,
                provider="openai",
                context_window=128000,
            )
            admin("/api/provider", provider)
            browser = (password, address, ca)
            scenarios(admin, member(admin, base, ca), postgres, browser)
        print(
            f"PASS rooms, delegation, messaging, steering, cancel and lanes on {MODEL}"
        )

    with tempfile.TemporaryDirectory(
        prefix="cubebot-postgres-", dir=ROOT / "target"
    ) as data:
        password = secrets.token_hex(24)
        name = f"cubebot-test-{uuid.uuid4().hex}"
        env = os.environ | {"POSTGRES_PASSWORD": password}
        run_database(name, env, password, os.path.abspath(data), workflow)


def main():
    if sys.platform != "linux" or platform.machine() != "x86_64":
        print(f"docker-rooms skipped: needs x86_64 Linux, this is {sys.platform}")
        return 0
    if not os.environ.get("OPENAI_API_KEY_FILE"):
        print("docker-rooms failed: set OPENAI_API_KEY_FILE", file=sys.stderr)
        return 1
    build()
    if os.environ.get("CUBEBOT_PLAYWRIGHT_MODULE"):
        web = ["docker", "build", "-q", "-t", "cubebot-web:local"]
        subprocess.run(web + ["-f", "apps/web/Dockerfile", "."], check=True, cwd=ROOT)
    with tempfile.TemporaryDirectory() as directory:
        qualify(Path(directory))
    return 0


if __name__ == "__main__":
    sys.exit(main())
