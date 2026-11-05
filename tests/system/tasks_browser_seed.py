"""Seeds a disposable stack (docker container cubebot-dev-pg, database `app`) for tasks_browser.mjs."""
# ruff: noqa: E501

import http.cookiejar
import json
import subprocess
import sys
import urllib.request
import uuid

B = "http://127.0.0.1:3100"
jar = http.cookiejar.CookieJar()
opener = urllib.request.build_opener(urllib.request.HTTPCookieProcessor(jar))


def call(method, path, body=None, raw=None):
    headers = {"origin": B, "cubebot-csrf": "1"}
    data = None
    if body is not None:
        data = json.dumps(body).encode()
        headers["content-type"] = "application/json"
    if raw is not None:
        data = raw
        headers["content-type"] = "application/octet-stream"
    request = urllib.request.Request(
        B + path, data=data, method=method, headers=headers
    )
    with opener.open(request) as response:
        text = response.read()
        return json.loads(text) if text[:1] in b"{[" else text


def sql(query):
    return subprocess.run(
        [
            "docker",
            "exec",
            "-i",
            "cubebot-dev-pg",
            "psql",
            "-U",
            "cubebot",
            "-d",
            "app",
            "-v",
            "ON_ERROR_STOP=1",
            "-q",
            "-t",
            "-A",
        ],
        input=query,
        text=True,
        capture_output=True,
        check=True,
    ).stdout.strip()


def quote(text):
    return "'" + text.replace("'", "''") + "'"


call("POST", "/api/identity/login", {"login": "victor", "password": "dev-password-123"})
me = sql("select id from human_accounts where login='victor'")
ids = {}
for name, paint, accessory in [
    ("Ayla", "sky", "bow"),
    ("Kind guy", "sage", "antenna"),
    ("Test Agent", "orange", "none"),
]:
    ids[name] = call(
        "POST",
        "/api/agents",
        {
            "request_id": str(uuid.uuid4()),
            "name": name,
            "instructions": "Be useful.",
            "avatar": {"paint": paint, "eyes": "round", "accessory": accessory},
        },
    )["id"]

CODE = """#!/usr/bin/env node
/**
 * Find and download movie subtitles from OpenSubtitles.com.
 */
type SubtitleFile = { file_id: number; file_name?: string };
const API_BASE = 'https://api.opensubtitles.com/api/v1';

export async function search(query: string): Promise<SubtitleFile[]> {
  const response = await fetch(`${API_BASE}/subtitles?query=${query}`);
  if (!response.ok) throw new Error(`HTTP ${response.status}`);
  return (await response.json()).data;
}
""" + "\n".join(f"// line {n}" for n in range(60))


def events(turn, items):
    rows = []
    for number, (name, tool_input, outcome) in enumerate(items, start=1):
        call_id = str(uuid.uuid4())
        rows.append(
            (
                number * 2 - 1,
                {"kind": "tool_call", "id": call_id, "name": name, "input": tool_input},
            )
        )
        if outcome is not None:
            rows.append(
                (number * 2, {"kind": "tool_result", "id": call_id, "outcome": outcome})
            )
    for sequence, payload in rows:
        sql(
            f"INSERT INTO turn_events (turn_id, sequence, payload) VALUES ('{turn}', {sequence}, {quote(json.dumps(payload))}::jsonb)"
        )


def turn_in_main(agent, message, response):
    return sql(
        f"INSERT INTO chat_turns (conversation_id,actor_id,request_id,message,status,response,completed_at) VALUES ('{agent}','{me}',gen_random_uuid(),{quote(message)},'complete',{quote(response)},clock_timestamp()) RETURNING id;"
    ).split("\n")[0]


FENCES = """Here is the banner, and the flow behind it:

```html
<section style="font-family:system-ui;padding:28px;border-radius:16px;color:#f5f3ff;background:linear-gradient(120deg,#4338ca,#9333ea 55%,#db2777)">
  <h2 style="margin:8px 0">Version 2.0 is here</h2>
  <p id="note" style="margin:0">Scripts run in the preview.</p>
  <script>document.getElementById('note').textContent='Script ran inside the frame';window.__ranInFrame=1;try{window.parent.document.title='PWNED'}catch(e){}</script>
</section>
```

```mermaid
graph LR; Order-->Items; Items-->Total
```

```svg
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 40"><rect width="100" height="40" rx="8" fill="#f26a1b"/></svg>
```

```csv
item,qty
Book,1
```

And the code:

```typescript
export const answer: number = 42 // the answer
```
"""
ayla = ids["Ayla"]
main = turn_in_main(
    ayla, "Write me a subtitle finder", "Done. It is in `movie-subtitle-finder`."
)
events(
    main,
    [
        (
            "kb_search",
            {"query": "TypeScript script create share file workflow"},
            {"status": "exited", "code": 0, "output": "No pages match."},
        ),
        (
            "browser_navigate",
            {
                "url": "https://opensubtitles.stoplight.io/docs/opensubtitles-api",
                "session": "main",
            },
            {"status": "exited", "code": 0, "output": "Opened."},
        ),
        (
            "browser_read",
            {"session": "main"},
            {"status": "exited", "code": 0, "output": "[1] heading OpenSubtitles API"},
        ),
        (
            "write_file",
            {"path": "movie-subtitle-finder/subtitles.ts", "content": CODE},
            {"status": "exited", "code": 0, "output": "Wrote it."},
        ),
        (
            "edit_file",
            {
                "path": "movie-subtitle-finder/subtitles.ts",
                "old_string": "const API_BASE = 'old';",
                "new_string": "const API_BASE = 'https://api.opensubtitles.com/api/v1';\nconst AGENT = 'Cubebot';",
                "replace_all": False,
            },
            {"status": "exited", "code": 0, "output": "Edited."},
        ),
        (
            "bash",
            {
                "command": "cd movie-subtitle-finder && npx tsx subtitles.ts --query 'The Hobbit'"
            },
            {
                "status": "exited",
                "code": 1,
                "output": "Error: OPENSUBTITLES_API_KEY is not set\n    at main (subtitles.ts:40:11)",
            },
        ),
        (
            "files_put",
            {"path": "movie-subtitle-finder/subtitles.ts", "share": True},
            {"status": "exited", "code": 0, "output": "Saved."},
        ),
    ],
)
turn_in_main(ayla, "Show me the launch banner", FENCES)

# A task at work: the orchestrator planned, Ayla is mid-way, Kind guy waits his turn.
working = call(
    "POST",
    "/api/tasks",
    {
        "request_id": str(uuid.uuid4()),
        "request": "Compare subtitle services and write up which we should use",
    },
)
room = working["room_id"]
sql(
    f"UPDATE tasks SET title='Compare subtitle services' WHERE id='{working['id']}'; UPDATE rooms SET name='Compare subtitle services' WHERE id='{room}'; UPDATE room_messages SET routing='routed' WHERE room_id='{room}';"
)
plan = sql(
    f"INSERT INTO room_messages (room_id,actor_id,request_id,message,routing,orchestrator) VALUES ('{room}','{me}',gen_random_uuid(),'Ayla researches the services and Kind guy writes it up. I will check it against what you asked.','routed',true) RETURNING id;"
).split("\n")[0]
for agent in ("Ayla", "Kind guy"):
    sql(f"INSERT INTO room_members (room_id,agent_id) VALUES ('{room}','{ids[agent]}')")
for position, (agent, title, status, note) in enumerate(
    [
        ("Ayla", "List the main subtitle services", "done", "Found three"),
        ("Ayla", "Compare their APIs and limits", "doing", None),
        ("Kind guy", "Write the recommendation", "todo", None),
        ("Kind guy", "Save it to the knowledge base", "todo", None),
    ],
    start=1,
):
    sql(
        f"INSERT INTO task_items (task_id,agent_id,position,title,status,note) VALUES ('{working['id']}','{ids[agent]}',{position},{quote(title)},'{status}',{quote(note) if note else 'NULL'})"
    )
running = sql(
    f"INSERT INTO chat_turns (conversation_id,actor_id,request_id,message,status,room_message_id) VALUES ('{ayla}','{me}',gen_random_uuid(),'Your part','running','{plan}') RETURNING id;"
).split("\n")[0]
events(
    running,
    [
        (
            "browser_navigate",
            {"url": "https://www.opensubtitles.com/docs", "session": "main"},
            {"status": "exited", "code": 0, "output": "Opened."},
        ),
        (
            "task_item",
            {"item": 1, "status": "done", "note": "Found three"},
            {"status": "exited", "code": 0, "output": "Item #1 is done."},
        ),
        (
            "write_file",
            {
                "path": "research/services.md",
                "content": "# Services\n\n- OpenSubtitles\n",
            },
            None,
        ),
    ],
)
sql(
    f"INSERT INTO turn_events (turn_id,sequence,payload) VALUES ('{running}',50,{quote(json.dumps({'kind': 'text_delta', 'text': 'So far OpenSubtitles looks **strongest**, '}))}::jsonb),('{running}',51,{quote(json.dumps({'kind': 'text_delta', 'text': 'still checking its limits.'}))}::jsonb)"
)
sql(
    f"INSERT INTO chat_turns (conversation_id,actor_id,request_id,message,status,room_message_id) VALUES ('{ids['Kind guy']}','{me}',gen_random_uuid(),'Your part','queued','{plan}')"
)
sql(
    f"INSERT INTO room_messages (room_id,actor_id,request_id,message,routing,author_agent_id) VALUES ('{room}','{me}',gen_random_uuid(),'I have the list; @Kind guy you will want the limits table when I finish.','routed','{ayla}')"
)

# Other agents' browser work, switched to running by the test that stacks desktops.
desktops = []
for agent in ("Kind guy", "Test Agent"):
    turn = sql(
        f"INSERT INTO chat_turns (conversation_id,actor_id,request_id,message,status) VALUES ('{ids[agent]}','{me}',gen_random_uuid(),'Look something up','cancelled') RETURNING id;"
    ).split("\n")[0]
    events(
        turn,
        [("browser_navigate", {"url": "https://example.com", "session": "main"}, None)],
    )
    desktops.append(turn)

asking = call(
    "POST",
    "/api/tasks",
    {"request_id": str(uuid.uuid4()), "request": "Plan the team offsite"},
)
sql(
    f"UPDATE tasks SET title='Plan the team offsite', status='needs_you' WHERE id='{asking['id']}'; UPDATE room_messages SET routing='routed' WHERE room_id='{asking['room_id']}'; INSERT INTO room_messages (room_id,actor_id,request_id,message,routing,orchestrator) VALUES ('{asking['room_id']}','{me}',gen_random_uuid(),'Which month should I plan around?','routed',true);"
)
finished = call(
    "POST",
    "/api/tasks",
    {"request_id": str(uuid.uuid4()), "request": "Summarize the launch notes"},
)
sql(
    f"UPDATE tasks SET title='Summarize the launch notes', status='done' WHERE id='{finished['id']}'; UPDATE room_messages SET routing='routed' WHERE room_id='{finished['room_id']}';"
)

teams = call(
    "POST",
    "/api/identity/teams",
    {"request_id": str(uuid.uuid4()), "name": "Test Team"},
)
call(
    "POST",
    "/api/identity/accounts",
    {
        "request_id": str(uuid.uuid4()),
        "login": "maria",
        "password": "member-password-123",
        "administrator": False,
    },
)

files = {
    "subtitles.ts": CODE.encode(),
    "page.html": b'<!doctype html><h1 id=x>Page</h1><script>document.getElementById("x").textContent="SCRIPT RAN";try{window.parent.document.title="PWNED"}catch(e){}</script>',
    "flow.mmd": b"graph TD; A-->B",
    "notes.md": b"# Notes\n\nSome **bold** text.\n",
    "orders.csv": b"item,qty\nBook,1\n",
    "logo.svg": b'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"><rect width="10" height="10" fill="#f26a1b"/></svg>',
    "config.json": b'{"a":1,"b":[1,2]}',
    "mystery.weird": b"plain words in a file with an unknown name\n",
    "archive.bin": b"\x00\x01\x02binary",
}
for name, data in files.items():
    call("POST", f"/api/files?name={name}&request_id={uuid.uuid4()}", raw=data)

json.dump(
    {
        "ids": ids,
        "working": working["id"],
        "asking": asking["id"],
        "running": running,
        "desktops": desktops,
        "team": str(teams),
    },
    open(sys.argv[1], "w"),
)
print("seeded")
