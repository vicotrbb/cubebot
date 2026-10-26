"""Exercise operator bootstrap and identity HTTP across real process restarts."""

import secrets
import subprocess
import uuid

from access_flow import check_access, check_access_restart
from agent_flow import check_agents, check_agents_restart
from authentication_flow import (
    check_authentication_budget,
    check_authentication_restart,
)
from request_flow import (
    check_account_commands,
    check_account_restart,
    check_invitation_receipt,
    check_redemption_replay,
)
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
        agent_checkpoint = check_agents(base, token, member)
        checkpoint = check_access(base, token, member)
        requests = check_account_commands(base, token)
        limited_login = check_authentication_budget(base, token)
    with running([str(BINARY)], ROOT, server_env) as process:
        await_ready(process, base + "/api/health/ready")
        assert request(base, "/api/identity/me", token)[1] == account
        check_access_restart(base, token, checkpoint)
        check_agents_restart(base, token, agent_checkpoint)
        check_account_restart(base, token, requests)
        check_authentication_restart(base, limited_login)
        assert request(base, "/api/identity/logout", token, {})[0] == 200
        assert request(base, "/api/identity/me", token)[0] == 401
    print(
        "Identity workflow passed: CLI bootstrap, HTTP login/invitation, "
        "denied escalation, durable replays, password limits, restart, logout"
    )


def check_invitation(base, token):
    command = {"login": "invited-http-member", "request_id": str(uuid.uuid4())}
    status, invitation = request(base, "/api/identity/invitations", token, command)
    assert status == 201
    check_invitation_receipt(base, token, command, invitation)
    member_password = secrets.token_hex(24)
    redemption = {
        "request_id": str(uuid.uuid4()),
        "token": invitation["token"],
        "password": member_password,
    }
    status, account = request(
        base, "/api/identity/invitations/accept", value=redemption
    )
    assert status == 201
    check_redemption_replay(base, redemption, account)
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
                "request_id": str(uuid.uuid4()),
                "login": "escalated",
                "password": member_password,
                "administrator": True,
            },
        )[0]
        == 403
    )
    return member["token"]
