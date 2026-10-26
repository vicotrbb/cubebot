"""Verify revisioned management and persisted lifecycle intent over real HTTP."""

import uuid

from smoke import request_json as request


def check_lifecycle(base, admin, member):
    command = {
        "request_id": str(uuid.uuid4()),
        "name": "Lifecycle agent",
        "instructions": "Original instructions",
    }
    status, created = request(base, "/api/agents", admin, command)
    assert status == 201
    path = "/api/agents/" + created["id"]
    account = request(base, "/api/identity/me", member)[1]
    grant_input = {
        "request_id": str(uuid.uuid4()),
        "recipient": {"kind": "user", "id": account["id"]},
        "scope": {"kind": "agent", "id": created["id"]},
        "capability": "agent_manage",
    }
    status, grant = request(base, "/api/identity/access/grants", admin, grant_input)
    assert status == 201
    update = {
        "request_id": str(uuid.uuid4()),
        "expected_revision": 1,
        "name": "Updated lifecycle agent",
        "instructions": "Corrected instructions",
        "idle_policy": "keep_running",
    }
    status, updated = request(base, path + "/definition", member, update)
    assert status == 200 and updated["revision"] == 2
    assert request(base, path + "/definition", member, update)[1] == updated
    assert (
        request(base, path + "/definition", member, update | {"name": "Conflicting"})[0]
        == 409
    )
    wake = check_transitions(base, member, path)
    check_definition_history(base, member, path, command["instructions"])
    assert (
        request(
            base,
            path + "/definition",
            member,
            update | {"request_id": str(uuid.uuid4()), "expected_revision": 7},
        )[0]
        == 409
    )
    revoke = {
        "request_id": str(uuid.uuid4()),
        "id": grant["id"],
        "expected_revision": 1,
    }
    assert request(base, "/api/identity/access/revoke", admin, revoke)[0] == 200
    assert request(base, path + "/definition", member, update)[0] == 404
    assert request(base, path + "/revisions", member)[0] == 404
    assert request(base, path + "/lifecycle", member, wake)[0] == 404
    return path, wake


def check_definition_history(base, token, path, original):
    status, revisions = request(base, path + "/revisions", token)
    assert status == 200 and [item["revision"] for item in revisions] == [1, 2]
    assert revisions[0]["instructions"] == original
    assert revisions[1]["instructions"] == "Corrected instructions"
    assert [
        item["revision"]
        for item in request(base, path + "/revisions?after=1", token)[1]
    ] == [2]


def check_transitions(base, token, path):
    wake = None
    for revision, action in enumerate(
        ("wake", "sleep", "wake", "restart", "retire"), start=2
    ):
        command = {
            "request_id": str(uuid.uuid4()),
            "expected_revision": revision,
            "action": action,
        }
        status, receipt = request(base, path + "/lifecycle", token, command)
        assert status == 202 and receipt["revision"] == revision + 1
        assert request(base, path + "/lifecycle", token, command)[1] == receipt
        if wake is None:
            wake = command
    definition = request(base, path, token)[1]
    assert (
        definition["desired_state"] == "retired"
        and definition["restart_generation"] == 3
    )
    assert definition["instructions_revision"] == 2
    denied = {"request_id": str(uuid.uuid4()), "expected_revision": 7, "action": "wake"}
    assert request(base, path + "/lifecycle", token, denied)[0] == 409
    return wake


def check_lifecycle_restart(base, admin, checkpoint):
    path, wake = checkpoint
    definition = request(base, path, admin)[1]
    assert definition["desired_state"] == "retired" and definition["revision"] == 7
    assert definition["instructions"] == "Corrected instructions"
    # The member's original receipt remains private to that actor, not the admin.
    assert request(base, path + "/lifecycle", admin, wake)[0] == 409
