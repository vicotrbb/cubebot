"""Shared files on real agents through Docker, from every side.

Same host needs as docker_sandbox.py, plus Node with Playwright for the console.
CUBEBOT_FILES_EVIDENCE keeps the evidence directory; CUBEBOT_FILES_HOLD keeps a failed
run for 15 minutes.
"""

import hashlib
import http.client
import json
import os
import re
import secrets
import socket
import ssl
import subprocess
import sys
import tempfile
import time
import traceback
import urllib.error
import urllib.parse
import urllib.request
import uuid
from pathlib import Path

from cluster_chat_setup import wait_for
from docker_browser import calls, chat
from docker_desktop import CONSOLE, Person, container, logs, person, psql, surroundings
from docker_sandbox import (
    ROOT,
    Api,
    agent,
    bridge,
    build,
    command,
    configure,
    configure_provider,
    grant,
    live,
    remove,
    unsupported,
)
from docker_sandbox import sign_in as bootstrap
from integration import run_database

MODEL = os.environ.get("CUBEBOT_FILES_MODEL", "gpt-5.5")
MARKER = secrets.token_hex(4)
QUOTA = 12 * 1024 * 1024
INSTRUCTIONS = (
    "You work in a bash workspace and hand files to people with the files_* tools. Do "
    "exactly the steps you are asked for, in order, with the exact arguments given. "
    "Never retry a refused call and never work around a refusal; reply with each "
    "tool's output."
)


def digest(data):
    return hashlib.sha256(data).hexdigest()


class Raw:
    def __init__(self, world, who):
        self.world, self.who = world, who

    def connection(self):
        host, port = self.world["address"].rsplit(":", 1)
        context = ssl.create_default_context(cafile=str(self.world["ca"]))
        return http.client.HTTPSConnection(host, int(port), context=context, timeout=60)

    def headers(self):
        return {"Authorization": f"Bearer {self.who.api.token}"}

    def upload(self, name, data, parent=None, request=None, content_type=None):
        query = {"name": name, "request_id": request or str(uuid.uuid4())}
        if parent:
            query["parent"] = parent
        headers = self.headers()
        if content_type:
            headers["Content-Type"] = content_type
        connection = self.connection()
        path = f"/api/files?{urllib.parse.urlencode(query)}"
        connection.request("POST", path, body=data, headers=headers)
        response = connection.getresponse()
        body = response.read()
        connection.close()
        return response.status, json.loads(body or b"null"), dict(response.headers)

    def content(self, file, extra=None):
        connection = self.connection()
        headers = self.headers() | (extra or {})
        connection.request("GET", f"/api/files/{file}/content", headers=headers)
        response = connection.getresponse()
        body = response.read()
        connection.close()
        return (
            response.status,
            body,
            {k.lower(): v for k, v in response.headers.items()},
        )

    def started(self, name, length):
        connection = self.connection()
        query = urllib.parse.urlencode({"name": name, "request_id": str(uuid.uuid4())})
        connection.putrequest("POST", f"/api/files?{query}")
        for key, value in self.headers().items():
            connection.putheader(key, value)
        connection.putheader("Content-Length", str(length))
        connection.endheaders()
        connection.send(b"x" * 4096)
        return connection


def staging(world):
    return sorted(os.listdir(world["storage"] / "staging"))


def blobs(world):
    return sorted(os.listdir(world["storage"] / "blobs"))


def rows(world, query):
    return psql(world["postgres"], query)


def entry_of(outcome):
    return json.loads(outcome["output"].split("Uploaded: ", 1)[1])


def shell(row):
    return [c for c in row["terminal"] if not re.match(r"files_\w+ \{", c["command"])]


def shell_sha(row, path):
    for call in shell(row):
        found = re.search(
            rf"([0-9a-f]{{64}})\s+\S*{re.escape(path)}", call["outcome"]["output"]
        )
        if found:
            return found.group(1)
    raise AssertionError(f"no sha256sum of {path} in {row['terminal']}")


def refused(call, reason):
    outcome = call["outcome"]
    assert outcome.get("code") == 1, outcome
    assert reason in outcome["output"], outcome


def writer_puts(world):
    admin, writer = world["people"]["admin"], world["writer"]
    name = f"report-{MARKER}.bin"
    row = chat(
        admin.api,
        writer,
        "Run this with bash: `head -c 1500017 /dev/urandom > report.bin && sha256sum "
        f"report.bin`. Then call files_put with path report.bin, name {name} and "
        "parent null.",
    )
    expected = shell_sha(row, "report.bin")
    [put] = calls(row, "files_put")
    assert put["outcome"]["code"] == 0, put
    entry = entry_of(put["outcome"])
    assert (entry["name"], entry["size"], entry["sha256"]) == (name, 1500017, expected)
    assert entry["owner_agent"] == writer and entry["owner_user"] is None, entry
    assert entry["access"] == "manage", entry
    status, data, headers = Raw(world, admin).content(entry["id"])
    assert status == 200 and digest(data) == expected, status
    assert headers["content-disposition"].startswith("attachment"), headers
    assert headers["x-content-type-options"] == "nosniff", headers
    [request] = rows(world, f"SELECT request_id FROM files WHERE id='{entry['id']}'")
    call_ids = rows(
        world,
        "SELECT payload->>'id' FROM turn_events WHERE payload->>'kind'='tool_call' "
        "AND payload->>'name'='files_put'",
    )
    assert request in call_ids, (request, call_ids)
    world["report"] = entry
    print(
        "PASS writer put: Writer's 1,500,017 bytes arrived with the sha256 bash printed"
    )
    print("     owned by Writer, under its tool call's id as the request id, and the")
    print("     administrator downloads the same bytes as an attachment")


def people_scopes(world):
    people, report = world["people"], world["report"]["id"]
    for name in ("reader", "editor", "admin"):
        status, data, _ = Raw(world, people[name]).content(report)
        assert status == 200 and digest(data) == world["report"]["sha256"], name
    status, _, _ = Raw(world, people["outsider"]).content(report)
    assert status == 403, status
    anonymous = Api(world["base"], world["ca"])
    anonymous(f"/api/files/{report}/content", expected=401)
    anonymous("/api/files", expected=401)
    unknown = uuid.uuid4()
    people["outsider"].api(f"/api/files/{unknown}/content", expected=404)
    placement = {"parent": None, "name": f"renamed-{MARKER}.bin"}
    for name in ("reader", "outsider", "admin"):
        people[name].api(f"/api/files/{report}", placement, 403, "PATCH")
        people[name].api(f"/api/files/{report}", expected=403, method="DELETE")
        people[name].api(f"/api/files/{report}/shares", [], 403, "PUT")
    people["reader"].api(f"/api/files/{report}/shares", expected=403)
    moved = people["editor"].api(f"/api/files/{report}", placement, method="PATCH")
    assert moved["name"] == placement["name"] and moved["access"] == "manage", moved
    assert people["editor"].api(f"/api/files/{report}/shares") == []
    world["report"]["name"] = placement["name"]
    for name in ("reader", "editor", "outsider"):
        mine = people[name].api("/api/files")
        assert report not in [e["id"] for e in mine], name
    print(
        "PASS people: workspace.read reads Writer's file, workspace.write also renames"
    )
    print(
        "     and manages its shares, an administrator only reads, an outsider gets 403"
    )
    print("     on every call, no credential 401 and an unknown file 404")


def agents_scoped(world):
    admin, writer, reader = world["people"]["admin"], world["writer"], world["reader"]
    report = world["report"]["id"]
    row = chat(
        admin.api,
        reader,
        f"Call files_get with file {report} and path stolen.bin.",
    )
    refused(calls(row, "files_get")[0], "forbidden")
    row = chat(admin.api, reader, "Call files_list with parent null and shared true.")
    listed = json.loads(calls(row, "files_list")[0]["outcome"]["output"])
    assert listed == [], listed
    row = chat(
        admin.api,
        reader,
        f"Call files_share with file {report}, kind agent, name Reader and permission "
        "write.",
    )
    refused(calls(row, "files_share")[0], "forbidden")

    row = chat(
        admin.api,
        writer,
        f"Call files_share with file {report}, kind agent, name Reader and permission "
        "read.",
    )
    assert calls(row, "files_share")[0]["outcome"]["code"] == 0, row["terminal"]
    row = chat(
        admin.api,
        reader,
        "Call files_list with parent null and shared true. Then call files_get with "
        f"file {report} and path copy.bin. Then run `sha256sum copy.bin` with bash.",
    )
    listed = json.loads(calls(row, "files_list")[0]["outcome"]["output"])
    assert [(e["id"], e["access"]) for e in listed] == [(report, "read")], listed
    assert calls(row, "files_get")[0]["outcome"]["code"] == 0, row["terminal"]
    assert shell_sha(row, "copy.bin") == world["report"]["sha256"]
    row = chat(
        admin.api,
        reader,
        f"Call files_share with file {report}, kind user, name files-outsider and "
        "permission read.",
    )
    refused(calls(row, "files_share")[0], "forbidden")
    status, _, _ = Raw(world, world["people"]["outsider"]).content(report)
    assert status == 403
    print(
        "PASS agents: Reader's files_get and files_share of Writer's file were refused"
    )
    print(
        "     and it saw nothing shared; once Writer shared it read-only, Reader listed"
    )
    print("     it, wrote it into its workspace with the same sha256, and still could")
    print("     not reshare it")


def team_share(world):
    people, writer, report = world["people"], world["writer"], world["report"]["id"]
    admin, teammate = people["admin"], people["teammate"]
    team = admin.api("/api/identity/teams", command(name=f"Reviewers {MARKER}"), 201)
    change = command(
        team=team["id"], account_id=teammate.id, expected_revision=1, present=True
    )
    admin.api("/api/identity/teams/membership", change)
    row = chat(
        admin.api,
        writer,
        f"Call files_share with file {report}, kind team, name Reviewers {MARKER} and "
        "permission read.",
    )
    assert calls(row, "files_share")[0]["outcome"]["code"] == 0, row["terminal"]
    shared = teammate.api("/api/files/shared")
    assert [(e["id"], e["access"]) for e in shared] == [(report, "read")], shared
    status, data, _ = Raw(world, teammate).content(report)
    assert status == 200 and digest(data) == world["report"]["sha256"]
    change |= {
        "request_id": str(uuid.uuid4()),
        "expected_revision": 2,
        "present": False,
    }
    admin.api("/api/identity/teams/membership", change)
    assert teammate.api("/api/files/shared") == []
    status, _, _ = Raw(world, teammate).content(report)
    assert status == 403, status
    shares = people["editor"].api(f"/api/files/{report}/shares")
    kinds = sorted((s["kind"], s["name"], s["permission"]) for s in shares)
    assert kinds == [
        ("agent", "Reader", "read"),
        ("team", f"Reviewers {MARKER}", "read"),
    ]
    print(
        "PASS team: Writer shared its file with a team; a member listed and downloaded"
    )
    print(
        "     it, and lost it on leaving the team; the owner's grantee sees both shares"
    )


def folder_share(world):
    admin, reader = world["people"]["admin"], world["reader"]
    folder = admin.api(
        "/api/files/folders", {"parent": None, "name": f"Inbox {MARKER}"}, 201
    )
    shares = [{"kind": "agent", "name": "Reader", "permission": "write"}]
    admin.api(f"/api/files/{folder['id']}/shares", shares, method="PUT")
    name = f"notes-{MARKER}.txt"
    row = chat(
        admin.api,
        reader,
        f"Run `printf 'notes {MARKER}\\n' > notes.txt` with bash. Then call files_put "
        f"with path notes.txt, name {name} and parent {folder['id']}. Then call "
        f"files_list with parent {folder['id']} and shared false.",
    )
    put = calls(row, "files_put")[0]
    assert put["outcome"]["code"] == 0, put
    entry = entry_of(put["outcome"])
    assert entry["parent"] == folder["id"] and entry["owner_user"] == admin.id, entry
    listed = json.loads(calls(row, "files_list")[0]["outcome"]["output"])
    assert [e["name"] for e in listed] == [name], listed
    status, data, _ = Raw(world, admin).content(entry["id"])
    assert status == 200 and data == f"notes {MARKER}\n".encode(), data

    row = chat(
        admin.api,
        reader,
        f"Call files_put with path missing-{MARKER}.txt, name missing.txt and parent "
        f"{folder['id']}.",
    )
    refused(calls(row, "files_put")[0], "cannot be read as the tool user")
    shares[0]["permission"] = "read"
    admin.api(f"/api/files/{folder['id']}/shares", shares, method="PUT")
    row = chat(
        admin.api,
        reader,
        f"Call files_put with path notes.txt, name second-{MARKER}.txt and parent "
        f"{folder['id']}.",
    )
    refused(calls(row, "files_put")[0], "forbidden")
    row = chat(
        admin.api,
        reader,
        f"Call files_get with file {entry['id']} and path inbox/notes.txt. Then run "
        "`cat inbox/notes.txt` with bash.",
    )
    assert calls(row, "files_get")[0]["outcome"]["code"] == 0, row["terminal"]
    assert f"notes {MARKER}" in shell(row)[-1]["outcome"]["output"]
    names = [e["name"] for e in admin.api(f"/api/files?parent={folder['id']}")]
    assert names == [name], names
    world["inbox"] = folder
    print(
        "PASS folder: with a write share on the administrator's folder Reader uploaded"
    )
    print("     into it as the folder owner's file; a missing workspace file stored")
    print("     nothing; with the share made read-only its upload was refused while")
    print("     reading the file inside, by inheritance, still worked")


def transfers(world):
    admin = world["people"]["admin"]
    raw = Raw(world, admin)
    data = secrets.token_bytes(300_000)
    request = str(uuid.uuid4())
    status, first, _ = raw.upload(f"bytes-{MARKER}.bin", data, request=request)
    assert status == 201, first
    status, again, _ = raw.upload(f"bytes-{MARKER}.bin", b"other", request=request)
    assert status == 200 and again["id"] == first["id"], again
    status, part, headers = raw.content(first["id"], {"Range": "bytes=1000-1999"})
    assert status == 206 and part == data[1000:2000], status
    assert headers["content-range"] == f"bytes 1000-1999/{len(data)}", headers
    status, _, _ = raw.content(first["id"], {"Range": "bytes=999999-"})
    assert status == 416, status
    status, _, _ = raw.upload(f"bytes-{MARKER}.bin", b"dup")
    assert status == 409, status

    before = (blobs(world), rows(world, "SELECT count(*) FROM files"))
    broken = raw.started(f"broken-{MARKER}.bin", 1_000_000)
    wait_for("the staged upload", lambda: staging(world), 30)
    broken.close()
    wait_for("the broken upload's cleanup", lambda: not staging(world), 30)
    assert (blobs(world), rows(world, "SELECT count(*) FROM files")) == before

    held = [raw.started(f"held-{index}-{MARKER}.bin", 1_000_000) for index in range(16)]
    wait_for("16 staged uploads", lambda: len(staging(world)) == 16, 30)
    status, busy, headers = raw.upload(f"busy-{MARKER}.bin", b"x")
    assert status == 429 and busy["error"] == "storage_busy", (status, busy)
    assert "retry-after" in {key.lower() for key in headers}, headers
    for connection in held:
        connection.close()
    wait_for("the held uploads' cleanup", lambda: not staging(world), 30)
    status, _, _ = raw.upload(f"after-{MARKER}.bin", b"fits")
    assert status == 201, status

    used = int(rows(world, "SELECT coalesce(sum(size),0) FROM blobs")[0])
    status, full, _ = raw.upload(f"full-{MARKER}.bin", b"x" * (QUOTA - used + 1))
    assert status == 507 and full["error"] == "storage_full", (status, full)
    assert not staging(world)
    status, _, _ = raw.upload(f"exact-{MARKER}.bin", b"y" * (QUOTA - used))
    assert status == 201, status
    world["exact"] = rows(
        world, f"SELECT id FROM files WHERE name='exact-{MARKER}.bin'"
    )[0]
    print("PASS transfers: a range is 206 and past the end 416, a retried request id")
    print("     returns the first file, a duplicate name 409; a dropped upload and 16")
    print(
        "     held ones left no staging, file or blob behind, the 17th upload got 429"
    )
    print("     with Retry-After, and the quota refused one byte over it with 507")


def agent_quota(world):
    admin, writer = world["people"]["admin"], world["writer"]
    before = blobs(world)
    row = chat(
        admin.api,
        writer,
        "Run `head -c 2000000 /dev/urandom > big.bin` with bash. Then call files_put "
        f"with path big.bin, name big-{MARKER}.bin and parent null.",
    )
    refused(calls(row, "files_put")[0], "storage_full")
    assert blobs(world) == before and not staging(world)
    admin.api(f"/api/files/{world['exact']}", expected=204, method="DELETE")
    row = chat(
        admin.api,
        writer,
        f"Call files_put with path big.bin, name big-{MARKER}.bin and parent null.",
    )
    put = calls(row, "files_put")[-1]
    assert put["outcome"]["code"] == 0 and entry_of(put["outcome"])["size"] == 2_000_000
    print(
        "PASS agent quota: Writer's upload over the quota was refused as storage_full"
    )
    print("     and stored nothing; after a person deleted a file the same upload fit")


def restart(world):
    admin = world["people"]["admin"]
    world["process"].kill()
    world["process"].wait(timeout=30)
    orphan = uuid.uuid4()
    (world["storage"] / "blobs" / str(orphan)).write_bytes(b"linked, never committed")
    (world["storage"] / "staging" / str(uuid.uuid4())).write_bytes(b"half an upload")
    world["process"] = world["start"]()
    wait_for("orchestrator HTTPS", lambda: live(admin.api), timeout=30)
    assert str(orphan) not in blobs(world) and not staging(world)
    recorded = set(rows(world, "SELECT id FROM blobs"))
    assert set(blobs(world)) == recorded, (blobs(world), recorded)
    status, data, _ = Raw(world, admin).content(world["report"]["id"])
    assert status == 200 and digest(data) == world["report"]["sha256"]
    status = f"/api/agents/{world['writer']}/status"
    wait_for("Writer's runtime", lambda: admin.api(status)["runtime_available"], 300)
    row = chat(
        admin.api, world["writer"], "Call files_list with parent null and shared false."
    )
    listed = json.loads(calls(row, "files_list")[0]["outcome"]["output"])
    assert world["report"]["id"] in [e["id"] for e in listed], listed
    print(
        "PASS restart: after a kill the orchestrator removed a planted orphan blob and"
    )
    print(
        "     staged upload, kept every recorded blob byte for byte, and Writer listed"
    )
    print("     its files again")


def console(world):
    people = world["people"]
    environment = os.environ | {
        "CUBEBOT_CONSOLE": CONSOLE,
        "CUBEBOT_CONSOLE_LOGIN": "files-admin",
        "CUBEBOT_CONSOLE_PASSWORD": people["admin"].password,
        "CUBEBOT_MEMBER_LOGIN": people["reader"].login,
        "CUBEBOT_MEMBER_PASSWORD": people["reader"].password,
        "CUBEBOT_FILES_MARKER": MARKER,
        "CUBEBOT_EVIDENCE_DIRECTORY": str(world["evidence"]),
    }
    script = Path(__file__).with_name("docker_files_browser.mjs")
    result = subprocess.run(
        ["node", script], env=environment, capture_output=True, text=True, timeout=600
    )
    print(result.stdout + result.stderr)
    assert result.returncode == 0, "the console check failed"
    admin = people["admin"].api
    [folder] = [e for e in admin("/api/files") if e["name"] == f"Console {MARKER}"]
    inside = {e["name"]: e for e in admin(f"/api/files?parent={folder['id']}")}
    assert sorted(inside) == [f"dropped-{MARKER}.bin", f"renamed-{MARKER}.txt"], inside
    assert inside[f"renamed-{MARKER}.txt"]["sha256"] == digest(
        f"console {MARKER}\n".encode()
    )
    shares = admin(f"/api/files/{folder['id']}/shares")
    assert shares == [
        {"kind": "user", "name": people["reader"].login, "permission": "read"}
    ]
    names = [e["name"] for e in admin("/api/files")]
    assert f"trash-{MARKER}.txt" not in names, names
    print("PASS console: in Chromium the administrator uploaded with the picker and by")
    print("     dropping a file, made a folder, dragged a file into it, renamed it,")
    print("     downloaded the same bytes, shared the folder and deleted a file; the")
    print("     reader found it under Shared with me, read only, and downloaded it")


def deleted(world):
    admin, reader = world["people"]["admin"], world["reader"]
    inbox = world["inbox"]["id"]
    inside = [e["id"] for e in admin.api(f"/api/files?parent={inbox}")]
    admin.api(f"/api/files/{inbox}", expected=204, method="DELETE")
    admin.api(f"/api/files/{inbox}", expected=404, method="DELETE")
    for file in inside:
        assert file not in blobs(world)
        assert rows(
            world, f"SELECT count(*) FROM file_shares WHERE file_id='{file}'"
        ) == ["0"]
    assert rows(world, f"SELECT count(*) FROM file_shares WHERE file_id='{inbox}'") == [
        "0"
    ]
    row = chat(
        admin.api, reader, f"Call files_get with file {inside[0]} and path gone.txt."
    )
    refused(calls(row, "files_get")[0], "not_found")
    print("PASS delete: deleting the inbox removed its file, blob and shares, a second")
    print("     delete is 404, and Reader's files_get of the file inside is not_found")


def scenarios(world):
    writer_puts(world)
    people_scopes(world)
    agents_scoped(world)
    team_share(world)
    folder_share(world)
    transfers(world)
    agent_quota(world)
    console(world)
    deleted(world)
    restart(world)


def setup(admin, base, ca, password):
    configure_provider(admin, MODEL)
    writer = agent(admin, "Writer", INSTRUCTIONS, 86400)
    reader = agent(admin, "Reader", INSTRUCTIONS, 86400)
    people = {
        "admin": Person(admin, "files-admin", password, admin("/api/identity/me")["id"])
    }
    for name in ("reader", "editor", "outsider", "teammate"):
        people[name] = person(admin, base, ca, f"files-{name}")
    grant(admin, people["reader"].id, writer, "workspace_read")
    grant(admin, people["editor"].id, writer, "workspace_write")
    for identity in (writer, reader):
        status = f"/api/agents/{identity}/status"
        wait_for(
            "the runtime", lambda status=status: admin(status)["runtime_available"], 300
        )
    return {"people": people, "writer": writer, "reader": reader}


def qualify(directory: Path):
    with socket.socket() as probe:
        probe.bind((bridge(), 0))
        address = f"{bridge()}:{probe.getsockname()[1]}"
    environment = configure(directory, address)
    storage = directory / "files"
    environment |= {
        "CUBEBOT_PUBLIC_ORIGIN": CONSOLE,
        "CUBEBOT_STORAGE_DIRECTORY": str(storage),
        "CUBEBOT_STORAGE_QUOTA_BYTES": str(QUOTA),
    }
    base, ca = f"https://{address}", directory / "ca/ca.crt"
    admin = Api(base, ca)
    world = {"address": address, "base": base, "ca": ca, "evidence": directory}
    world["storage"] = storage
    log_path = directory / "orchestrator.log"

    def workflow(postgres, env):
        environment["CUBEBOT_DATABASE_URL"] = env["CUBEBOT_TEST_DATABASE_URL"]
        world["postgres"] = postgres

        def start():
            binary = [str(ROOT / "target/debug/cubebot-orchestrator")]
            with open(log_path, "a") as log:
                return subprocess.Popen(
                    binary,
                    env=os.environ | environment,
                    stdout=log,
                    stderr=subprocess.STDOUT,
                )

        world["start"] = start
        world["process"] = start()
        surrounding = []
        try:
            wait_for("orchestrator HTTPS", lambda: live(admin), timeout=30)
            password = secrets.token_urlsafe(24)
            bootstrap(admin, env["CUBEBOT_TEST_DATABASE_URL"], "files-admin", password)
            world.update(setup(admin, base, ca, password))
            surrounding = surroundings(world, address, ca)
            scenarios(world)
            log = log_path.read_text()
            for person_ in world["people"].values():
                assert person_.password not in log
            assert "panicked" not in log, "the orchestrator panicked"
        except BaseException:
            print(log_path.read_text()[-20000:])
            for name in ("writer", "reader"):
                if world.get(name) and container(world[name]):
                    print(logs(world[name])[-10000:])
            if os.environ.get("CUBEBOT_FILES_HOLD"):
                traceback.print_exc()
                print("HOLDING for inspection", flush=True)
                time.sleep(900)
            raise
        finally:
            subprocess.run(
                ["docker", "rm", "-f", *surrounding], check=False, capture_output=True
            )
            for name in ("writer", "reader"):
                if world.get(name):
                    remove(world[name])
            world["process"].terminate()
            world["process"].wait(timeout=30)
        print("PASS shared files on real agents, from every scope and person's side")

    with tempfile.TemporaryDirectory(
        prefix="cubebot-postgres-", dir=ROOT / "target"
    ) as data:
        password = secrets.token_hex(24)
        name = f"cubebot-test-{uuid.uuid4().hex}"
        env = os.environ | {"POSTGRES_PASSWORD": password}
        run_database(name, env, password, os.path.abspath(data), workflow)


def main():
    if (refused := unsupported("docker-files")) is not None:
        return refused
    build()
    subprocess.run(
        [
            "docker",
            "build",
            "--tag",
            "cubebot-web:local",
            "--file",
            "apps/web/Dockerfile",
            ".",
        ],
        check=True,
        cwd=ROOT,
    )
    keep = os.environ.get("CUBEBOT_FILES_EVIDENCE")
    with tempfile.TemporaryDirectory() as directory:
        qualify(Path(keep or directory))
    return 0


if __name__ == "__main__":
    sys.exit(main())
