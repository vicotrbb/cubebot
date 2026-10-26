"""Exercise scoped team grants and command replay through the real HTTP service."""

import uuid

from smoke import request_json as request


def check_access(base, admin, member):
    status, account = request(base, "/api/identity/me", member)
    assert status == 200
    command = {"request_id": str(uuid.uuid4()), "name": "Engineering"}
    status, team = request(base, "/api/identity/teams", admin, command)
    assert status == 201
    check_team_listing(base, admin, command, team)
    change = {
        "request_id": str(uuid.uuid4()),
        "team": team["id"],
        "account_id": account["id"],
        "expected_revision": 1,
        "present": True,
    }
    assert request(base, "/api/identity/teams/membership", admin, change)[0] == 200
    scope = {"kind": "agent", "id": str(uuid.uuid4())}
    grant = {
        "request_id": str(uuid.uuid4()),
        "scope": scope,
        "capability": "memory_read",
        "recipient": {"kind": "team", "id": team["id"]},
    }
    assert request(base, "/api/identity/access/grants", admin, grant)[0] == 201
    check_effective_access(base, member, scope, team["id"])
    change |= {
        "request_id": str(uuid.uuid4()),
        "expected_revision": 2,
        "present": False,
    }
    assert request(base, "/api/identity/teams/membership", admin, change)[0] == 200
    status, explanation = request(
        base,
        "/api/identity/access/explain",
        member,
        {"scope": scope, "capability": "memory_read"},
    )
    assert status == 200 and explanation["allowed"] is False
    status, events = request(base, "/api/identity/access/events", admin)
    assert status == 200 and len(events) >= 4
    assert events[-1]["input"]["account_id"] == account["id"]
    assert events[-1]["input"]["present"] is False
    return command, team


def check_effective_access(base, member, scope, team):
    query = {"scope": scope, "capability": "memory_read"}
    status, explanation = request(base, "/api/identity/access/explain", member, query)
    assert status == 200 and explanation["allowed"] is True
    assert explanation["grants"][0]["team_id"] == team
    query["capability"] = "memory_edit"
    assert (
        request(base, "/api/identity/access/explain", member, query)[1]["allowed"]
        is False
    )
    query["capability"] = "memory_read"
    query["scope"] = {"kind": "agent", "id": str(uuid.uuid4())}
    assert (
        request(base, "/api/identity/access/explain", member, query)[1]["allowed"]
        is False
    )
    assert (
        request(
            base,
            "/api/identity/teams",
            member,
            {"request_id": str(uuid.uuid4()), "name": "Escalated"},
        )[0]
        == 403
    )


def check_access_restart(base, admin, checkpoint):
    command, team = checkpoint
    path = f"/api/identity/commands/team.create/{command['request_id']}"
    status, receipt = request(base, path, admin)
    assert status == 200 and receipt == team
    assert request(base, "/api/identity/teams", admin, command)[1] == team
    changed = command | {"name": "Changed"}
    assert request(base, "/api/identity/teams", admin, changed)[0] == 409


def check_team_listing(base, admin, command, team):
    assert request(base, "/api/identity/teams", admin, command)[1] == team
    status, listed = request(base, "/api/identity/teams", admin)
    assert status == 200 and listed[0]["id"] == team["id"]
    assert request(base, f"/api/identity/teams?after={team['id']}", admin)[1] == []
