"""Failures against shipped processes on the owned Kubernetes/CSI fixture."""

import json
import os
import subprocess
import time
import urllib.error
import urllib.request
import uuid
from contextlib import contextmanager
from pathlib import Path

import cluster_admission as admission
from cluster_chat_setup import NAMESPACE, available, wait_for


def command(**values):
    return dict(request_id=str(uuid.uuid4()), **values)


def agent(f, name):
    result = f.request(
        "/api/agents", command(name=name, instructions="Answer briefly."), expected=201
    )
    return result["id"]


def send(f, identity, text, token=None):
    body = command(message=text)
    path = f"/api/conversations/{identity}/messages"
    turn = f.request(path, body, token=token, expected=202)
    assert f.request(path, body, token=token, expected=202)["id"] == turn["id"]
    return turn


def history(f, identity):
    return f.request(f"/api/conversations/{identity}/messages")


def outcome(f, identity, turn, states=("complete",), timeout=100):
    def inspect():
        return next(
            (
                row
                for row in history(f, identity)
                if row["id"] == turn["id"] and row["status"] in states
            ),
            None,
        )

    return wait_for(f"turn {turn['id']} in {states}", inspect, timeout)


def pod(f, identity):
    items = json.loads(
        f.k("get", "pods", "-l", f"cubebot.dev/agent={identity}", "-o", "json")
    )["items"]
    return items[0] if len(items) == 1 else None


def running(f, identity):
    def inspect():
        current = pod(f, identity)
        status = f.request(f"/api/agents/{identity}/status")
        return current if current and status["runtime_available"] else None

    return wait_for("authenticated runtime", inspect)


def lifecycle(f, identity, action):
    status = f.request(f"/api/agents/{identity}/status")
    return f.request(
        f"/api/agents/{identity}/lifecycle",
        command(
            expected_revision=status["revision"],
            action=action,
        ),
        expected=202,
    )


def signal(f, current, value):
    statuses = current["status"]["containerStatuses"]
    container = statuses[0]["containerID"].split("://", 1)[1]
    admission.run(
        "docker",
        "exec",
        f"{f.name}-control-plane",
        "ctr",
        "-n",
        "k8s.io",
        "tasks",
        "kill",
        "--signal",
        value,
        container,
    )


def replacement(f, identity, old):
    def inspect():
        current = pod(f, identity)
        if not current or current["metadata"]["uid"] == old["metadata"]["uid"]:
            return None
        return (
            current
            if f.request(f"/api/agents/{identity}/status")["runtime_available"]
            else None
        )

    return wait_for("safe Pod replacement", inspect)


def configure(f, key, access=None, model="gpt-4.1-mini"):
    previous = f.request("/api/provider")
    body = command(
        expected_revision=previous["revision"] if previous else 0,
        api_key=key,
        model=model,
        provider="openai",
        context_window=128000,
    )
    if access is not None:
        body["access"] = access
    result = f.request("/api/provider", body)
    assert "api_key" not in result and "credential" not in result
    return result


def retry(f, identity, turn):
    path = f"/api/conversations/{identity}/messages/{turn['id']}/retry"
    body = command()
    result = f.request(path, body)
    assert f.request(path, body)["id"] == result["id"]
    return result


@contextmanager
def partition(f, current, provider=False):
    """Actual node packet drops, without changing the compiled workload spec."""
    gateway = json.loads(f.k("get", "pod", "gateway", "-o", "json"))["status"]["podIP"]
    source = current["status"]["podIP"]
    rule = ["-s", source, "-p", "tcp", "--dport", "443"]
    rule += ["!", "-d", gateway] if provider else ["-d", gateway]
    rule += ["-j", "DROP"]
    prefix = ["docker", "exec", f"{f.name}-control-plane", "iptables"]
    admission.run(*prefix, "-I", "FORWARD", "1", *rule)
    try:
        yield
    finally:
        counters = admission.run(*prefix, "-L", "FORWARD", "-nvx")
        (
            f.artifacts / f"partition-{current['metadata']['name']}-{provider}.txt"
        ).write_text(counters)
        admission.run(*prefix, "-D", "FORWARD", *rule)


def live_chat(f):
    identity = agent(f, "Live qualification")
    result = send(f, identity, "Remember cedar. Reply with just cedar recorded.")
    committed = outcome(f, identity, result)
    assert committed["response"]
    current = running(f, identity)
    assert len(current["spec"]["containers"]) == 1
    assert current["spec"]["automountServiceAccountToken"] is False
    assert current["spec"]["serviceAccountName"] == "cubebot-runtime"
    f.save(
        "live-chat.json",
        {"agent": identity, "turn": committed, "pod_uid": current["metadata"]["uid"]},
    )
    print(
        "PASS real OpenAI text turn with projected identity and verified HTTPS",
        flush=True,
    )
    return identity, current


def runtime_crash(f, identity, current):
    marker = "/var/lib/cubebot/qualification-marker"
    f.k(
        "exec",
        current["metadata"]["name"],
        "--",
        "/bin/sh",
        "-c",
        f"printf retained > {marker}",
    )
    claim = current["spec"]["volumes"][0]["persistentVolumeClaim"]["claimName"]
    workspace = json.loads(f.k("get", "pvc", claim, "-o", "json"))
    projected = f.k(
        "exec", current["metadata"]["name"], "--", "cat", "/run/cubebot/identity/token"
    )
    signal(f, current, "SIGKILL")
    new = replacement(f, identity, current)
    assert f.k("exec", new["metadata"]["name"], "--", "cat", marker) == "retained"
    assert (
        json.loads(f.k("get", "pvc", claim, "-o", "json"))["metadata"]["uid"]
        == workspace["metadata"]["uid"]
    )
    f.request(
        "/api/runtime/bootstrap",
        {"token": projected, "instance": str(uuid.uuid4())},
        token="",
        expected=401,
    )
    assert history(f, identity)[0]["response"]
    print(
        "PASS runtime SIGKILL, CSI retention, replacement and stale identity rejection",
        flush=True,
    )
    return new


def exclusive_storage(f, current):
    claim = current["spec"]["volumes"][0]["persistentVolumeClaim"]["claimName"]
    f.pod(
        "workspace-contender",
        [
            {
                "name": "contender",
                "image": f.images["sandbox"],
                "command": ["/usr/bin/true"],
                "volumeMounts": [{"name": "workspace", "mountPath": "/workspace"}],
            }
        ],
        restartPolicy="Never",
        automountServiceAccountToken=False,
        volumes=[{"name": "workspace", "persistentVolumeClaim": {"claimName": claim}}],
    )
    try:

        def rejected():
            events = json.loads(
                f.k(
                    "get",
                    "events",
                    "--field-selector",
                    "involvedObject.name=workspace-contender",
                    "-o",
                    "json",
                )
            )
            return next(
                (
                    item["message"]
                    for item in events["items"]
                    if "ReadWriteOncePod" in item.get("message", "")
                ),
                None,
            )

        reason = wait_for("CSI exclusive mount rejection", rejected)
        f.save("exclusive-storage.json", {"claim": claim, "rejection": reason})
    finally:
        f.k("delete", "pod", "workspace-contender", "--wait=true", "--timeout=30s")
    print("PASS CSI ReadWriteOncePod excludes a second Pod", flush=True)


def in_flight_control_crash(f):
    identity = agent(f, "Outbox during control crash")
    lifecycle(f, identity, "wake")
    current = running(f, identity)
    controller = json.loads(f.k("get", "pod", "orchestrator", "-o", "json"))
    stopped = False
    try:
        with partition(f, current, provider=True):
            turn = send(f, identity, "Reply with durable result.")
            outcome(f, identity, turn, ("running",))
            signal(f, controller, "SIGSTOP")
            stopped = True
        # The reply arrives during the outage and waits in the agent's outbox.
        time.sleep(20)
    finally:
        if stopped:
            signal(f, controller, "SIGKILL")
    wait_for("crashed control plane recovery", lambda: available(f))
    assert outcome(f, identity, turn)["response"]
    assert (
        f.sql(
            f"SELECT count(*) FROM turn_events WHERE turn_id='{turn['id']}' "
            "AND payload->>'kind'='done'"
        )
        == "1"
    )
    print(
        "PASS turn events resent through a control-plane crash commit once",
        flush=True,
    )


def orchestrator_crash(f, identity):
    before = f.sql("SELECT generation FROM orchestrator_owner")
    current = json.loads(f.k("get", "pod", "orchestrator", "-o", "json"))
    signal(f, current, "SIGKILL")
    wait_for("orchestrator recovery", lambda: available(f))
    wait_for(
        "new orchestrator ownership",
        lambda: f.sql("SELECT generation FROM orchestrator_owner") != before,
    )
    assert (
        f.sql("SELECT previous_termination_proof IS NOT NULL FROM orchestrator_owner")
        == "t"
    )
    assert history(f, identity)[0]["response"]
    print(
        "PASS orchestrator SIGKILL, retained history and Kubernetes termination proof",
        flush=True,
    )


def control_partition(f, identity, current):
    with partition(f, current):
        turn = send(f, identity, "Reply with the word queued.")
        time.sleep(12)
        assert outcome(f, identity, turn, ("queued",), timeout=2)["response"] is None
    assert outcome(f, identity, turn)["response"]
    print(
        "PASS packet partition holds durable intake; reconnect commits response",
        flush=True,
    )


def interrupted_inference(f):
    identity = agent(f, "Interrupted inference")
    lifecycle(f, identity, "wake")
    current = running(f, identity)
    with partition(f, current, provider=True):
        turn = send(f, identity, "Reply briefly.")
        outcome(f, identity, turn, ("running",))
        signal(f, current, "SIGKILL")
        replacement(f, identity, current)
        waiting = outcome(f, identity, turn, ("waiting",))
        assert (
            waiting["error"] == "execution_interrupted" and waiting["response"] is None
        )
    retry(f, identity, waiting)
    assert outcome(f, identity, turn)["response"]
    assert len(history(f, identity)) == 1
    print(
        "PASS in-flight crash exposes uncertainty; only explicit retry resumes work",
        flush=True,
    )


def provider_partition(f):
    identity = agent(f, "Provider unreachable")
    lifecycle(f, identity, "wake")
    current = running(f, identity)
    with partition(f, current, provider=True):
        turn = send(f, identity, "Reply with recovered.")
        waiting = outcome(f, identity, turn, ("waiting",))
        assert waiting["response"] is None
        assert waiting["error"] == "provider_unavailable"
    time.sleep(6)
    assert outcome(f, identity, turn, ("waiting",), timeout=2)["response"] is None
    print(
        "PASS provider packet loss waits without automatic ambiguous retry",
        flush=True,
    )


def lifecycle_retention(f, identity, current):
    claim = current["spec"]["volumes"][0]["persistentVolumeClaim"]["claimName"]
    uid = json.loads(f.k("get", "pvc", claim, "-o", "json"))["metadata"]["uid"]
    for action in ("restart", "sleep"):
        lifecycle(f, identity, action)
        if action == "restart":
            current = replacement(f, identity, current)
            retained_workspace(f, current, claim, uid)
        else:
            wait_for("sleep drain", lambda: pod(f, identity) is None)
    lifecycle(f, identity, "wake")
    current = running(f, identity)
    retained_workspace(f, current, claim, uid)
    assert history(f, identity)[0]["response"]
    lifecycle(f, identity, "retire")
    wait_for("retirement drain", lambda: pod(f, identity) is None)
    status = f.request(f"/api/agents/{identity}/status")
    assert status["desired_state"] == "retired" and status["workspace_attempt"]
    assert json.loads(f.k("get", "pvc", claim, "-o", "json"))["metadata"]["uid"] == uid
    assert history(f, identity)[0]["response"]
    assert len(history(f, identity)) == 3
    f.request(
        f"/api/agents/{identity}/lifecycle",
        command(expected_revision=status["revision"], action="wake"),
        expected=409,
    )
    print(
        "PASS restart, sleep, wake and retirement preserve history/workspace",
        flush=True,
    )


def retained_workspace(f, current, claim, uid):
    assert json.loads(f.k("get", "pvc", claim, "-o", "json"))["metadata"]["uid"] == uid
    assert (
        f.k(
            "exec",
            current["metadata"]["name"],
            "--",
            "cat",
            "/var/lib/cubebot/qualification-marker",
        )
        == "retained"
    )


def provider_changes(f, key):
    identity = agent(f, "Provider policy")
    configure(f, key, {"mode": "explicit_grants", "agents": []})
    turn = send(f, identity, "Reply with enabled.")
    waiting = outcome(f, identity, turn, ("waiting",))
    assert waiting["error"] == "provider_unavailable"
    configure(
        f, key, {"mode": "all_agents"}, model="cubebot-nonexistent-qualification-model"
    )
    retry(f, identity, waiting)
    waiting = outcome(f, identity, turn, ("waiting",))
    assert waiting["error"] == "provider_rejected"
    configure(f, key, {"mode": "all_agents"})
    retry(f, identity, waiting)
    assert outcome(f, identity, turn)["response"]
    print(
        "PASS provider access/model changes, visible rejection, explicit recovery",
        flush=True,
    )


def permissions(f):
    identity = agent(f, "Permission revocation")
    account = f.request(
        "/api/identity/accounts",
        command(
            login="member", password="disposable member password", administrator=False
        ),
        expected=201,
    )
    token = f.request(
        "/api/identity/login",
        {"login": "member", "password": "disposable member password"},
        token="",
    )["token"]
    grants = []
    for capability in ("conversation_read", "conversation_send"):
        grants.append(
            f.request(
                "/api/identity/access/grants",
                command(
                    recipient={"kind": "user", "id": account["id"]},
                    scope={"kind": "conversation", "id": identity},
                    capability=capability,
                ),
                expected=201,
            )
        )
    lifecycle(f, identity, "wake")
    current = running(f, identity)
    with partition(f, current):
        turn = send(f, identity, "Retain after permission revocation", token)
        for grant in grants:
            f.request(
                "/api/identity/access/revoke",
                command(id=grant["id"], expected_revision=grant["revision"]),
            )
        f.request(f"/api/conversations/{identity}/messages", token=token, expected=404)
    waiting = outcome(f, identity, turn, ("waiting",))
    assert waiting["error"] == "sender_access_revoked"
    print("PASS revoked sender cannot read or start accepted queued work", flush=True)


def qualify(f):
    verify_tls(f)
    key = Path(os.environ["OPENAI_API_KEY_FILE"]).read_text().strip()
    configure(f, key)
    identity, current = live_chat(f)
    protected_credentials(f, key, current)
    browser(f)
    exclusive_storage(f, current)
    current = runtime_crash(f, identity, current)
    orchestrator_crash(f, identity)
    control_partition(f, identity, current)
    in_flight_control_crash(f)
    interrupted_inference(f)
    provider_partition(f)
    browser(f, waiting=True)
    provider_changes(f, key)
    permissions(f)
    lifecycle_retention(f, identity, current)
    assert key not in f.sql("SELECT row_to_json(p)::text FROM provider_connection p")
    f.save(
        "summary.json",
        {"cluster": f.name, "checks": "completed", "source": admission.fingerprint()},
    )


def verify_tls(f):
    from cluster_chat_setup import ORIGIN

    try:
        urllib.request.urlopen(ORIGIN + "/api/health/live", timeout=10)
    except urllib.error.URLError as error:
        assert "CERTIFICATE_VERIFY_FAILED" in str(error.reason)
    else:
        raise AssertionError("untrusted TLS certificate was accepted")
    assert available(f)
    print("PASS private CA required; verified TLS succeeds", flush=True)


def protected_credentials(f, key, current):
    assert (
        f.k(
            "exec",
            current["metadata"]["name"],
            "--",
            "stat",
            "-c",
            "%u:%a",
            "/var/lib/cubebot/runtime",
        )
        == "10001:700"
    )
    assert key not in f.k(
        "exec", current["metadata"]["name"], "--", "cat", "/proc/1/environ"
    )
    allowed = subprocess.run(
        [
            "kubectl",
            "--kubeconfig",
            str(f.config),
            "-n",
            NAMESPACE,
            "auth",
            "can-i",
            "get",
            "secrets",
            "--as",
            f"system:serviceaccount:{NAMESPACE}:cubebot-runtime",
        ],
        capture_output=True,
        text=True,
        check=False,
    )
    assert allowed.returncode == 1 and allowed.stdout.strip() == "no"
    print(
        "PASS private runtime storage, no key in environment, no Secret-read RBAC",
        flush=True,
    )


def browser(f, waiting=False):
    environment = os.environ | {
        "CUBEBOT_BROWSER_DIRECTORY": str(f.artifacts),
        "CUBEBOT_BROWSER_PASSWORD": f.password,
        "CUBEBOT_BROWSER_WAITING": "1" if waiting else "0",
    }
    subprocess.run(
        ["node", "tests/system/cluster_chat_browser.mjs"],
        env=environment,
        check=True,
        timeout=180,
    )
