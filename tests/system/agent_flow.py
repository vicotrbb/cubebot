"""Read actual private agent definitions through team-scoped HTTP authorization."""

import uuid

from smoke import request_json as request


def check_agents(base, admin, member):
    command = {
        "request_id": str(uuid.uuid4()),
        "name": "Private researcher",
        "instructions": "Inspect private evidence",
    }
    status, receipt = request(base, "/api/agents", admin, command)
    assert status == 201
    assert request(base, "/api/agents", admin, command)[1] == receipt
    assert request(base, "/api/agents", admin, command | {"name": "Changed"})[0] == 409
    assert request(base, "/api/agents", member, command)[0] == 403
    other = command | {"request_id": str(uuid.uuid4()), "name": "Other private agent"}
    assert request(base, "/api/agents", admin, other)[0] == 201
    assert request(base, "/api/agents", member)[1] == []
    path = "/api/agents/" + receipt["id"]
    assert request(base, path, member)[0] == 404
    team, membership = grant_manager(base, admin, member, receipt["id"])
    status, definition = request(base, path, member)
    assert status == 200 and definition["instructions"] == command["instructions"]
    assert definition["desired_state"] == "sleeping"
    assert [item["id"] for item in request(base, "/api/agents", member)[1]] == [
        receipt["id"]
    ]
    revoke_membership(base, admin, member, path, membership)
    return command, receipt, team, member


def grant_manager(base, admin, member, agent):
    account = request(base, "/api/identity/me", member)[1]
    status, team = request(
        base,
        "/api/identity/teams",
        admin,
        {"name": "Agent managers", "request_id": str(uuid.uuid4())},
    )
    assert status == 201
    membership = {
        "request_id": str(uuid.uuid4()),
        "team": team["id"],
        "account_id": account["id"],
        "present": True,
        "expected_revision": 1,
    }
    assert request(base, "/api/identity/teams/membership", admin, membership)[0] == 200
    grant = {
        "request_id": str(uuid.uuid4()),
        "recipient": {"kind": "team", "id": team["id"]},
        "scope": {"kind": "agent", "id": agent},
        "capability": "agent_manage",
    }
    assert request(base, "/api/identity/access/grants", admin, grant)[0] == 201
    return team, membership


def revoke_membership(base, admin, member, path, membership):
    change = membership | {
        "request_id": str(uuid.uuid4()),
        "expected_revision": 2,
        "present": False,
    }
    assert request(base, "/api/identity/teams/membership", admin, change)[0] == 200
    assert request(base, path, member)[0] == 404
    assert request(base, "/api/agents", member)[1] == []


def check_agents_restart(base, admin, checkpoint):
    command, receipt, _team, member = checkpoint
    assert request(base, "/api/agents", admin, command)[1] == receipt
    assert (
        request(base, "/api/agents/" + receipt["id"], admin)[1]["name"]
        == command["name"]
    )
    assert request(base, "/api/agents", member)[1] == []
