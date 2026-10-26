"""Verify secret-bearing identity command recovery through real HTTP restarts."""

import secrets
import uuid

from smoke import request_json as request


def check_account_commands(base, admin):
    command = {
        "request_id": str(uuid.uuid4()),
        "login": "request-member",
        "password": secrets.token_hex(24),
        "administrator": False,
    }
    status, created = request(base, "/api/identity/accounts", admin, command)
    assert status == 201
    assert request(base, "/api/identity/accounts", admin, command)[1] == created
    changed = command | {"password": secrets.token_hex(24)}
    assert request(base, "/api/identity/accounts", admin, changed)[0] == 409
    status, login = request(
        base,
        "/api/identity/login",
        value={
            "login": command["login"],
            "password": command["password"],
        },
    )
    assert status == 200
    status, events = request(base, "/api/identity/access/events", admin)
    assert status == 200
    assert command["password"] not in str(events)
    assert "$argon2" not in str(events)
    return command, created, login["token"]


def check_account_restart(base, admin, checkpoint):
    command, created, member = checkpoint
    assert request(base, "/api/identity/accounts", admin, command)[1] == created
    path = f"/api/identity/commands/account.create/{command['request_id']}"
    status, receipt = request(base, path, admin)
    assert status == 200 and receipt["id"] == created["id"]
    assert request(base, "/api/identity/me", member)[0] == 200
    suspend = {
        "id": created["id"],
        "expected_revision": 1,
        "request_id": str(uuid.uuid4()),
    }
    status, outcome = request(base, "/api/identity/suspend", admin, suspend)
    assert status == 200 and outcome["revision"] == 2
    assert request(base, "/api/identity/suspend", admin, suspend)[1] == outcome
    assert request(base, "/api/identity/me", member)[0] == 401
    status, accounts = request(base, "/api/identity/accounts", admin)
    assert status == 200
    account = next(account for account in accounts if account["id"] == created["id"])
    assert account["active"] is False and account["revision"] == 2


def check_invitation_receipt(base, admin, command, issued):
    status, replay = request(base, "/api/identity/invitations", admin, command)
    assert status == 201 and replay["id"] == issued["id"]
    assert replay["token"] is None
    status, invitations = request(base, "/api/identity/invitations", admin)
    assert status == 200 and invitations[0]["revision"] == 1
    assert invitations[0]["status"] == "pending"
    assert "token" not in invitations[0]
    changed = command | {"login": "changed-invite"}
    assert request(base, "/api/identity/invitations", admin, changed)[0] == 409


def check_redemption_replay(base, command, account):
    assert (
        request(base, "/api/identity/invitations/accept", value=command)[1] == account
    )
    changed = command | {"password": secrets.token_hex(24)}
    assert request(base, "/api/identity/invitations/accept", value=changed)[0] == 409
    changed = command | {"request_id": str(uuid.uuid4())}
    assert request(base, "/api/identity/invitations/accept", value=changed)[0] == 401
