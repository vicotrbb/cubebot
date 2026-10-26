"""Exercise operator bootstrap and identity HTTP across real process restarts."""

import json
import secrets
import subprocess
import urllib.error
import urllib.request

from smoke import BINARY, ROOT, await_ready, running, unused_port


def request(base, path, token="", value=None):
    headers = {"Authorization": f"Bearer {token}"}
    data = None
    if value is not None:
        data = json.dumps(value).encode()
        headers["Content-Type"] = "application/json"
    req = urllib.request.Request(base + path, data=data, headers=headers)
    try:
        with urllib.request.urlopen(req, timeout=5) as response:
            return response.status, json.loads(response.read())
    except urllib.error.HTTPError as error:
        return error.code, json.loads(error.read())


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
    with running([str(BINARY)], ROOT, server_env) as process:
        await_ready(process, base + "/api/health/ready")
        assert request(base, "/api/identity/me", token)[1] == account
        assert request(base, "/api/identity/logout", token, {})[0] == 200
        assert request(base, "/api/identity/me", token)[0] == 401
    print("Identity workflow passed: CLI bootstrap, HTTP login, restart, logout")
