"""Check durable chat intake and denied access across actual orchestrator restarts."""

import uuid

from smoke import request_json as request


def check_chat(base, token, member):
    status, created = request(
        base,
        "/api/agents",
        token,
        {
            "request_id": str(uuid.uuid4()),
            "name": "HTTP chat",
            "instructions": "Be brief",
        },
    )
    assert status == 201
    status, conversation = request(
        base,
        f"/api/agents/{created['id']}/conversation",
        token,
    )
    assert status == 200
    path = f"/api/conversations/{conversation['id']}/messages"
    message = {"request_id": str(uuid.uuid4()), "message": "Keep this after restart"}
    status, accepted = request(base, path, token, message)
    assert status == 202
    assert request(base, path, token, message)[1]["id"] == accepted["id"]
    assert request(base, path, member)[0] == 404
    assert request(base, path, member, message)[0] == 404
    return path, message, accepted["id"]


def check_chat_restart(base, token, checkpoint):
    path, message, turn = checkpoint
    status, history = request(base, path, token)
    assert status == 200 and len(history) == 1
    assert history[0]["id"] == turn and history[0]["message"] == message["message"]
    assert request(base, path, token, message)[1]["id"] == turn
    assert len(request(base, path, token)[1]) == 1
