"""Run real database tests against an isolated, disposable local PostgreSQL."""

import json
import os
import secrets
import subprocess
import sys
import tempfile
import time
import uuid
from concurrent.futures import ThreadPoolExecutor

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
    container = development_container()
    if container:
        run_shared(container)
        return
    name = f"cubebot-test-{uuid.uuid4().hex}"
    password = secrets.token_hex(24)
    env = os.environ | {"POSTGRES_PASSWORD": password}
    with tempfile.TemporaryDirectory(prefix="cubebot-postgres-", dir="target") as data:
        run_database(name, env, password, os.path.abspath(data), verify)


def run_database(name, env, password, data, workflow):
    failed = False
    try:
        # PostgreSQL owns its private PGDATA child; its UID must traverse this parent.
        os.chmod(data, 0o711)
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
        wait_ready(name)
        binding = run(
            "docker", "port", name, "5432/tcp", capture_output=True
        ).stdout.strip()
        port = binding.rsplit(":", 1)[1]
        env["CUBEBOT_TEST_DATABASE_URL"] = (
            f"postgres://cubebot:{password}@127.0.0.1:{port}/cubebot_test"
        )
        workflow(name, env)
    except BaseException:
        failed = True
        raise
    finally:
        failures = cleanup_database(name, data)
        if failures:
            print(
                "Database fixture cleanup failed: " + "; ".join(failures),
                file=sys.stderr,
            )
            if not failed:
                raise RuntimeError("database fixture cleanup failed")


def cleanup_database(name, data):
    failures = []
    try:
        run("docker", "stop", name, stdout=subprocess.DEVNULL, timeout=60)
    except Exception as error:
        failures.append(str(error))
    try:
        restore_data_owner(data)
    except Exception as error:
        failures.append(str(error))
    return failures


def development_container():
    # Reuse `just db-up` when it runs; never touch its application database.
    if not os.path.exists(".env"):
        return None
    result = subprocess.run(
        ["docker", "compose", "--env-file", ".env", "ps", "--status", "running"]
        + ["--quiet", "postgres"],
        capture_output=True,
        text=True,
        check=False,
    )
    return result.stdout.strip() or None


def read_env_file():
    values = {}
    with open(".env") as file:
        for line in file:
            key, separator, value = line.strip().partition("=")
            if separator and not key.startswith("#"):
                values[key] = value
    return values


def run_shared(container):
    settings = read_env_file()
    database = f"cubebot_it_{secrets.token_hex(4)}"
    run("docker", "exec", container, "createdb", "-U", "cubebot", database)
    port = settings.get("POSTGRES_PORT", "5432")
    env = os.environ | {
        "CUBEBOT_TEST_DATABASE_URL": f"postgres://cubebot:{settings['POSTGRES_PASSWORD']}"
        f"@127.0.0.1:{port}/{database}"
    }
    try:
        verify(container, env)
    finally:
        # Each test database carries this run's prefix; the shared template stays.
        run(
            "docker",
            "exec",
            "-i",
            container,
            "psql",
            "-U",
            "cubebot",
            "-d",
            "postgres",
            "-q",
            input="SELECT format('DROP DATABASE %I WITH (FORCE)', datname) "
            f"FROM pg_database WHERE starts_with(datname, '{database}') \\gexec\n",
            stdout=subprocess.DEVNULL,
        )


def test_binaries():
    built = run(
        "cargo",
        "test",
        "-p",
        "cubebot-orchestrator",
        "--locked",
        "--no-run",
        "--message-format=json",
        capture_output=True,
    ).stdout
    artifacts = (json.loads(line) for line in built.splitlines())
    return [
        artifact["executable"]
        for artifact in artifacts
        if artifact.get("reason") == "compiler-artifact"
        and artifact["profile"]["test"]
        and artifact.get("executable")
    ]


def run_tests(env):
    # Password hashing has a fail-fast per-process cap; one process per test keeps it
    # per test while tests run in parallel, each against its own database.
    tests = [
        (binary, line.removesuffix(": test"))
        for binary in test_binaries()
        for line in run(
            binary,
            "--list",
            "--format",
            "terse",
            "--include-ignored",
            capture_output=True,
        ).stdout.splitlines()
        if line.endswith(": test")
    ]

    def one(test):
        binary, name = test
        return name, subprocess.run(
            [binary, "--exact", name, "--include-ignored"],
            cwd="apps/orchestrator",
            env=env,
            capture_output=True,
            text=True,
            check=False,
        )

    with ThreadPoolExecutor(os.cpu_count()) as pool:
        failed = [
            (name, result)
            for name, result in pool.map(one, tests)
            if result.returncode != 0
        ]
    for name, result in failed:
        print(f"---- {name}\n{result.stdout}{result.stderr}", file=sys.stderr)
    if failed:
        raise RuntimeError(f"{len(failed)} of {len(tests)} orchestrator tests failed")
    print(f"{len(tests)} orchestrator tests passed")


def verify(name, env):
    run_tests(env)
    check_identity(env)
    url = env["CUBEBOT_TEST_DATABASE_URL"]
    oidc = url.rsplit("/", 1)[1] + "_oidc"
    run("docker", "exec", name, "createdb", "-U", "cubebot", oidc)
    check_oidc(env | {"CUBEBOT_TEST_DATABASE_URL": url.rsplit("/", 1)[0] + "/" + oidc})


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
        timeout=60,
    )


if __name__ == "__main__":
    main()
