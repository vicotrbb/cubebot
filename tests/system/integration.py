"""Run real database tests against an isolated, disposable local PostgreSQL."""

import os
import secrets
import subprocess
import tempfile
import time
import uuid

from identity_flow import check_identity
from oidc_flow import check_oidc


def run(*args, **kwargs):
    return subprocess.run(args, check=True, text=True, **kwargs)


def wait_ready(name):
    for _ in range(60):
        result = subprocess.run(
            ["docker", "exec", name, "pg_isready", "-h", "127.0.0.1", "-U", "cubebot"],
            capture_output=True,
            check=False,
        )
        if result.returncode == 0:
            return
        time.sleep(0.5)
    raise RuntimeError("isolated PostgreSQL did not become ready")


def main():
    run("cargo", "build", "--workspace", "--locked")
    name = f"cubebot-test-{uuid.uuid4().hex}"
    password = secrets.token_hex(24)
    env = os.environ | {"POSTGRES_PASSWORD": password}
    with tempfile.TemporaryDirectory(prefix="cubebot-postgres-", dir="target") as data:
        run_database(name, env, password, os.path.abspath(data))


def run_database(name, env, password, data):
    run(
        "docker",
        "run",
        "--detach",
        "--rm",
        "--name",
        name,
        "--mount",
        f"type=bind,source={data},target=/var/lib/postgresql",
        "--publish",
        "127.0.0.1::5432",
        "--env",
        "POSTGRES_PASSWORD",
        "--env",
        "POSTGRES_USER=cubebot",
        "--env",
        "POSTGRES_DB=cubebot_test",
        "postgres:18-bookworm",
        env=env,
        stdout=subprocess.DEVNULL,
    )
    try:
        wait_ready(name)
        binding = run(
            "docker", "port", name, "5432/tcp", capture_output=True
        ).stdout.strip()
        port = binding.rsplit(":", 1)[1]
        env["CUBEBOT_TEST_DATABASE_URL"] = (
            f"postgres://cubebot:{password}@127.0.0.1:{port}/cubebot_test"
        )
        run(
            "cargo",
            "test",
            "-p",
            "cubebot-orchestrator",
            "--locked",
            "--",
            "--include-ignored",
            "--test-threads=1",
            env=env,
        )
        check_identity(env)
        run("docker", "exec", name, "createdb", "-U", "cubebot", "cubebot_oidc")
        check_oidc(
            env
            | {
                "CUBEBOT_TEST_DATABASE_URL": env["CUBEBOT_TEST_DATABASE_URL"].rsplit(
                    "/", 1
                )[0]
                + "/cubebot_oidc"
            }
        )
    finally:
        try:
            run("docker", "stop", name, stdout=subprocess.DEVNULL)
        finally:
            restore_data_owner(data)


def restore_data_owner(data):
    # Return this test directory from PostgreSQL's UID to its creator on Linux.
    run(
        "docker",
        "run",
        "--rm",
        "--user",
        "root",
        "--mount",
        f"type=bind,source={data},target=/var/lib/postgresql",
        "--entrypoint",
        "chown",
        "postgres:18-bookworm",
        "-R",
        f"{os.getuid()}:{os.getgid()}",
        "/var/lib/postgresql",
    )


if __name__ == "__main__":
    main()
