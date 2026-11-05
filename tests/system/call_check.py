"""A voice call in real Chromium through the real speech stack, with no provider key.

The model's words are scripted, so this is not agent qualification, and no real microphone or speaker is involved,
so this says nothing about echo.
"""
# ruff: noqa: E501

import os
import secrets
import socket
import struct
import subprocess
import sys
import tempfile
import time
import uuid
import wave
from pathlib import Path

from cluster_chat_setup import wait_for
from docker_desktop import container
from docker_sandbox import (
    INSTRUCTIONS,
    ROOT,
    Api,
    agent,
    bridge,
    build,
    command,
    configure,
    live,
    remove,
    sign_in,
)
from integration import run_database
from smoke import running, unused_port

PORT = 18080
# The second question proves the first turn is in the context; the first, the call's instructions.
STAND_IN = r"""
import http.server, json, sys

LONG = " ".join(f"This is sentence number {n} of a very long answer." for n in range(60))


def answer(body):
    items = body["input"]
    people = [i["content"] for i in items if i.get("role") == "user"]
    told = "voice call" in items[0]["content"]
    earlier = any("yellow lamps" in text.lower() for text in people[:-1])
    if not told:
        return "The call instructions never arrived."
    return LONG if earlier else "Hello from the cube. I can hear you clearly."


class Provider(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def do_POST(self):
        body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
        text = answer(body)
        frames = [json.dumps({"type": "response.output_text.delta", "delta": text[at:at + 24]})
                  for at in range(0, len(text), 24)]
        part = {"type": "output_text", "text": text}
        frames.append(json.dumps({"type": "response.completed", "response": {
            "status": "completed", "output": [{"type": "message", "content": [part]}]}}))
        data = "".join("data: " + frame + "\n\n" for frame in frames).encode()
        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)


http.server.HTTPServer(("127.0.0.1", int(sys.argv[1])), Provider).serve_forever()
"""


def stand_in(name):
    subprocess.run(
        ["docker", "exec", "--detach", "--user", "65534:65534", name]
        + ["python3", "-c", STAND_IN, str(PORT)],
        check=True,
    )

    def listening():
        probe = f"import socket; socket.create_connection(('127.0.0.1', {PORT}), 1)"
        found = subprocess.run(
            ["docker", "exec", name, "python3", "-c", probe], capture_output=True
        )
        return found.returncode == 0

    wait_for("the scripted provider", listening, timeout=30)


def microphone(models: Path, path: Path):
    recording = next(models.glob("sherpa-onnx-streaming-*/test_wavs/0.wav"))
    with wave.open(str(recording)) as source:
        params = source.getparams()
        speech = source.readframes(params.nframes)
    assert (params.framerate, params.nchannels, params.sampwidth) == (16000, 1, 2)
    silence = struct.pack("<h", 0) * (16000 * 9)
    with wave.open(str(path), "wb") as out:
        out.setparams(params)
        out.writeframes(speech + silence)


def turns_on_call(postgres, identity):
    query = (
        "SELECT count(*) FROM chat_turns "
        f"WHERE conversation_id='{identity}' AND call_id IS NOT NULL"
    )
    found = subprocess.run(
        ["docker", "exec", postgres, "psql", "-U", "cubebot", "-d", "cubebot_test"]
        + ["-tAc", query],
        capture_output=True,
        check=True,
        text=True,
    )
    return int(found.stdout)


def qualify(directory: Path, models: Path, evidence: Path):
    with socket.socket() as probe:
        probe.bind((bridge(), 0))
        address = f"{bridge()}:{probe.getsockname()[1]}"
    environment = configure(directory, address, "container")
    storage = directory / "storage"
    storage.mkdir()
    # The check's models survive it, so only the first run downloads them.
    (storage / "speech-models").symlink_to(models)
    origin_port = unused_port()
    console = f"http://127.0.0.1:{origin_port}"
    environment["CUBEBOT_STORAGE_DIRECTORY"] = str(storage)
    environment["CUBEBOT_PUBLIC_ORIGIN"] = console
    environment["CUBEBOT_TEST_PROVIDER_ORIGIN"] = f"http://127.0.0.1:{PORT}/"
    ca = directory / "ca/ca.crt"
    api = Api(f"https://{address}", ca)
    mic = directory / "microphone.wav"
    microphone(models, mic)
    login, password = "call-admin", secrets.token_urlsafe(24)

    def workflow(postgres, env):
        database = env["CUBEBOT_TEST_DATABASE_URL"]
        environment["CUBEBOT_DATABASE_URL"] = database
        binary = [str(ROOT / "target/debug/cubebot-orchestrator")]
        web_port = unused_port()
        server = ["bun", "run", ".output/server/index.mjs"]
        web = {"PORT": str(web_port), "HOST": "127.0.0.1"}
        with (
            running(binary, ROOT, environment),
            running(server, ROOT / "apps/web", web),
        ):
            wait_for("orchestrator HTTPS", lambda: live(api), timeout=30)
            sign_in(api, database, login, password)
            provider = command(
                expected_revision=0,
                api_key="scripted-provider-key",
                model="scripted-model",
                provider="openai",
                context_window=128000,
            )
            api("/api/provider", provider)
            identity = agent(api, "Caller", INSTRUCTIONS)
            try:
                wait_for(
                    "authenticated runtime",
                    lambda: api(f"/api/agents/{identity}/status")["runtime_available"],
                    timeout=300,
                )
                stand_in(container(identity))
                subprocess.run(
                    ["node", str(ROOT / "tests/system/call_check_browser.mjs")],
                    check=True,
                    timeout=420,
                    env=os.environ
                    | {
                        "CUBEBOT_PROXY_PORT": str(origin_port),
                        "CUBEBOT_ORCHESTRATOR": f"https://{address}",
                        "CUBEBOT_CA": str(ca),
                        "CUBEBOT_WEB_PORT": str(web_port),
                        "CUBEBOT_CONSOLE_LOGIN": login,
                        "CUBEBOT_CONSOLE_PASSWORD": password,
                        "CUBEBOT_MICROPHONE": str(mic),
                        "CUBEBOT_EVIDENCE_DIRECTORY": str(evidence),
                    },
                )
                assert turns_on_call(postgres, identity) >= 2
                print("PASS a call's sentences are ordinary turns of the conversation")
            except BaseException:
                if name := container(identity):
                    subprocess.run(["docker", "logs", "--tail", "60", name])
                raise
            finally:
                remove(identity)

    with tempfile.TemporaryDirectory(
        prefix="cubebot-postgres-", dir=ROOT / "target"
    ) as data:
        db_password = secrets.token_hex(24)
        name = f"cubebot-test-{uuid.uuid4().hex}"
        env = os.environ | {"POSTGRES_PASSWORD": db_password}
        run_database(name, env, db_password, os.path.abspath(data), workflow)


def main():
    started = time.monotonic()
    build("container")
    subprocess.run(["bun", "run", "build"], check=True, cwd=ROOT)
    models = Path(
        os.environ.get("CUBEBOT_SPEECH_MODELS", ROOT / "target/speech-models")
    )
    subprocess.run(
        ["just", "speech-check", str(models)],
        check=True,
        cwd=ROOT,
        stdout=subprocess.DEVNULL,
    )
    evidence = Path(
        os.environ.get("CUBEBOT_CALL_EVIDENCE", ROOT / "target/call-evidence")
    )
    evidence.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory() as directory:
        qualify(Path(directory), models.resolve(), evidence.resolve())
    print(f"call-check passed in {time.monotonic() - started:.0f}s")
    return 0


if __name__ == "__main__":
    sys.exit(main())
