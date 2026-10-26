"""Verify the public password-attempt budget and restart behavior."""

import secrets
import uuid

from smoke import request_json as request


def check_authentication_budget(base, admin):
    login = {"login": "limited-http-member", "password": secrets.token_hex(24)}
    create = login | {"request_id": str(uuid.uuid4())}
    assert request(base, "/api/identity/accounts", admin, create)[0] == 201
    for _ in range(10):
        wrong = login | {"password": "wrong password"}
        assert request(base, "/api/identity/login", value=wrong)[0] == 401
    status, rejected = request(base, "/api/identity/login", value=login)
    assert status == 401 and rejected["error"] == "unauthorized"
    unknown = login | {"login": "unknown-http-account"}
    assert request(base, "/api/identity/login", value=unknown) == (status, rejected)
    return login


def check_authentication_restart(base, login):
    assert request(base, "/api/identity/login", value=login)[0] == 401
