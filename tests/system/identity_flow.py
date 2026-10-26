"""Exercise operator bootstrap and identity HTTP across real process restarts."""

import secrets
import subprocess

from access_flow import check_access, check_access_restart
from smoke import (
    BINARY,
    ROOT,
    await_ready,
    running,
    unused_port,
)
from smoke import (
    request_json as request,
)


def check_identity(environment):
    password = secrets.token_hex(24)
    env = environment | {
        "CUBEBOT_DATABASE_URL": environment["CUBEBOT_TEST_DATABASE_URL"],
        "CUBEBOT_BOOTSTRAP_LOGIN": "operator",
        "CUBEBOT_BOOTSTRAP_PASSWORD": password,
    }
    command = [str(ROOT / "target/debug/cubebot-admin"), "bootstrap"]
    subprocess.run(command, env=env, check=True, capture_output=True)
    repeated = subprocess.run(command, env=env, check=False, capture_output=True)
    assert repeated.returncode != 0, "bootstrap must be one-time"
    # Credentials belong only in the operator bootstrap process.
    server_env = env.copy()
    del server_env["CUBEBOT_BOOTSTRAP_PASSWORD"]
    del server_env["CUBEBOT_BOOTSTRAP_LOGIN"]
    address = f"127.0.0.1:{unused_port()}"
    server_env["CUBEBOT_BIND_ADDR"] = address
    base = f"http://{address}"
    with running([str(BINARY)], ROOT, server_env) as process:
        await_ready(process, base + "/api/health/ready")
        assert request(base, "/api/identity/me")[0] == 401
        status, result = request(
            base,
            "/api/identity/login",
            value={
                "login": "operator",
                "password": password,
            },
        )
        assert status == 200
        token = result["token"]
        status, account = request(base, "/api/identity/me", token)
        assert status == 200 and account["administrator"] is True
        member = check_invitation(base, token)
        checkpoint = check_access(base, token, member)
    with running([str(BINARY)], ROOT, server_env) as process:
        await_ready(process, base + "/api/health/ready")
        assert request(base, "/api/identity/me", token)[1] == account
        check_access_restart(base, token, checkpoint)
        assert request(base, "/api/identity/logout", token, {})[0] == 200
        assert request(base, "/api/identity/me", token)[0] == 401
    print(
        "Identity workflow passed: CLI bootstrap, HTTP login/invitation, "
        "denied escalation, restart, logout"
    )


def check_invitation(base, token):
    status, invitation = request(
        base, "/api/identity/invitations", token, {"login": "invited-http-member"}
    )
    assert status == 201
    member_password = secrets.token_hex(24)
    status, _ = request(
        base,
        "/api/identity/invitations/accept",
        value={
            "token": invitation["token"],
            "password": member_password,
        },
    )
    assert status == 201
    assert (
        request(
            base,
            "/api/identity/invitations/accept",
            value={
                "token": invitation["token"],
                "password": member_password,
            },
        )[0]
        == 401
    )
    status, member = request(
        base,
        "/api/identity/login",
        value={
            "login": "invited-http-member",
            "password": member_password,
        },
    )
    assert status == 200
    assert (
        request(
            base,
            "/api/identity/accounts",
            member["token"],
            {
                "login": "escalated",
                "password": member_password,
                "administrator": True,
            },
        )[0]
        == 403
    )
    return member["token"]
