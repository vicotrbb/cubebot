"""Serve a disposable browser/runtime workflow; Ctrl-C removes its fixture."""

import os
import secrets
import subprocess
import tempfile
import uuid

from integration import run, run_database


def serve(name, env):
    del name
    run("cargo", "build", "--workspace", "--locked")
    run(
        "cargo",
        "build",
        "-p",
        "cubebot-orchestrator",
        "--example",
        "text-chat-workflow",
        "--locked",
    )
    try:
        subprocess.run(
            ["target/debug/examples/text-chat-workflow"], env=env, check=True
        )
    except KeyboardInterrupt:
        pass


def main():
    password = secrets.token_hex(24)
    with tempfile.TemporaryDirectory(prefix="cubebot-chat-", dir="target") as data:
        run_database(
            "cubebot-chat-" + uuid.uuid4().hex,
            os.environ | {"POSTGRES_PASSWORD": password},
            password,
            os.path.abspath(data),
            serve,
        )


if __name__ == "__main__":
    main()
