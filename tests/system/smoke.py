"""Smoke-test built binaries and the actual production Bun server without a cluster."""

import json
import os
import re
import socket
import subprocess
import tempfile
import time
import urllib.error
import urllib.request
from contextlib import contextmanager
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
BINARY = ROOT / "target/debug/cubebot-orchestrator"


def unused_port() -> int:
    with socket.socket() as listener:
        listener.bind(("127.0.0.1", 0))
        return listener.getsockname()[1]


def read_url(url: str) -> bytes:
    with urllib.request.urlopen(url, timeout=2) as response:
        return response.read()


def await_ready(process: subprocess.Popen, url: str) -> bytes:
    deadline = time.monotonic() + 20
    while time.monotonic() < deadline:
        if process.poll() is not None:
            raise RuntimeError(f"Service exited before responding at {url}")
        try:
            return read_url(url)
        except urllib.error.URLError:
            time.sleep(0.1)
    raise TimeoutError(f"Service did not respond at {url}")


@contextmanager
def running(command: list[str], cwd: Path, environment: dict[str, str]):
    with tempfile.TemporaryFile(mode="w+") as log:
        process = subprocess.Popen(
            command,
            cwd=cwd,
            env={**os.environ, **environment},
            stdout=log,
            stderr=subprocess.STDOUT,
        )
        try:
            yield process
        except Exception:
            log.seek(0)
            print(log.read())
            raise
        finally:
            process.terminate()
            try:
                process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait()


def check_api() -> None:
    port = unused_port()
    address = f"127.0.0.1:{port}"
    with running(
        [str(BINARY)], ROOT, {"CUBEBOT_BIND_ADDR": address, "RUST_LOG": "info"}
    ) as process:
        body = await_ready(process, f"http://{address}/api/health/live")
        assert json.loads(body) == {"status": "alive"}
        process.terminate()
        assert process.wait(timeout=5) == 0, "Orchestrator must shut down gracefully"


def check_invalid_configuration() -> None:
    for settings in ({"CUBEBOT_BIND_ADDR": "invalid"}, {"RUST_LOG": "invalid["}):
        result = subprocess.run(
            [str(BINARY)],
            cwd=ROOT,
            env={**os.environ, **settings},
            capture_output=True,
            timeout=5,
            check=False,
        )
        assert result.returncode != 0, f"Invalid configuration accepted: {settings}"


def check_web() -> None:
    port = unused_port()
    url = f"http://127.0.0.1:{port}"
    with running(
        ["bun", ".output/server/index.mjs"],
        ROOT / "apps/web",
        {"PORT": str(port), "HOST": "127.0.0.1", "NODE_ENV": "production"},
    ) as process:
        html = await_ready(process, url).decode()
        assert "A home for your agents." in html
        assert '<html lang="en"' in html
        assets = re.findall(r'(?:src|href)="(/assets/[^"\s]+)"', html)
        assert any(asset.endswith(".js") for asset in assets)
        assert any(asset.endswith(".css") for asset in assets)
        for asset in assets:
            assert read_url(url + asset), f"Empty asset: {asset}"


if __name__ == "__main__":
    check_invalid_configuration()
    check_api()
    check_web()
    print(
        "Smoke passed: invalid config, API liveness, graceful shutdown, "
        "Bun SSR and assets"
    )
