"""The vault (ADR 0086) on real agents through Docker.

Same host needs as docker_login.py, Node with Playwright included. Keeper and Other run
guest desktops with OpenAI's model. An administrator, a controller (`browser.view` and
`browser.control` on Keeper), a viewer (`browser.view`) and an outsider each manage
what their grants allow; installation items follow ADR 0081 access. The agents list,
fill and save items, and every fill is proved where it lands, on a login page this
script serves, and refused fills in the arbiter's log and the ledger. No secret is in
the database dump, the orchestrator's log or the sandboxes' logs. CUBEBOT_VAULT_EVIDENCE
keeps the evidence directory; CUBEBOT_VAULT_HOLD keeps a failed run for 15 minutes.
"""

import json
import os
import re
import secrets
import socket
import subprocess
import sys
import tempfile
import time
import traceback
import urllib.parse
import uuid
from pathlib import Path

from cluster_chat_setup import wait_for
from docker_browser import calls, chat, lifecycle, ready
from docker_desktop import (
    CONSOLE,
    MODEL,
    Person,
    arbiter_events,
    container,
    grant,
    key,
    logs,
    person,
    surroundings,
    visit,
)
from docker_login import MARKER as PAGE
from docker_login import Site, modes, pending
from docker_sandbox import (
    ROOT,
    Api,
    bridge,
    build,
    command,
    configure,
    live,
    remove,
    unsupported,
)
from docker_sandbox import sign_in as bootstrap
from integration import run_database

SHARED = f"sh {secrets.token_hex(6)}!"
KEEPER_ONLY = f"ko-{secrets.token_hex(6)}"
OWN = f"own-{secrets.token_hex(6)}"
SAVED = f"sv-{secrets.token_hex(6)}"
SECRETS = [SHARED, KEEPER_ONLY, OWN, SAVED]
PENDING = "a person is logging in for you"
INSTRUCTIONS = (
    "You use the browser_* tools for web pages, the computer tool for the screen and "
    "the vault_* tools for logins. Do exactly the steps you are asked for, in order, "
    "never retry a refused call, never write a secret in your reply, and reply with a "
    "short note of what happened."
)


def login_page(site):
    """Docker_login's sign-in page, whose user field has the focus once it loads."""
    return site.url(f"login-{PAGE}")


def submitted(site, before):
    """The user and password of the first sign-in after `before` requests."""
    path, _ = wait_for(
        "the guest to sign in",
        lambda: site.requests(f"session-{PAGE}")[before:],
        120,
    )[0]
    query = urllib.parse.parse_qs(urllib.parse.urlparse(path).query)
    return query.get("user", [""])[0], query.get("password", [""])[0]


def sign_ins(site):
    return len(site.requests(f"session-{PAGE}"))


def fills(identity, before=0):
    """The arbiter's fill events since `before`, as who filled which item."""
    events = [e for e in arbiter_events(identity)[before:] if "item" in e]
    for event in events:
        event["who"] = {
            k: event[k] for k in ("source", "account", "item") if k in event
        }
    return events


def vault(who, agent, expected=200):
    return who.api(f"/api/agents/{agent}/vault", expected=expected)


def names(items):
    return sorted(item["name"] for item in items)


def item(world, name):
    return world["items"][name]


# The password Keeper makes, once a fill shows it.
world_secrets = []


def clean(text):
    for secret in SECRETS + world_secrets:
        assert secret not in text, f"a secret in {text[:300]}"


def save(who, body, expected=200, method=None, path="/api/vault"):
    saved = who.api(path, body, expected=expected, method=method)
    clean(json.dumps(saved))
    return saved


def items(world):
    """The administrator's installation items and the controller's item on Keeper."""
    admin, controller = world["people"]["admin"], world["people"]["controller"]
    keeper = world["keeper"]
    login = {"kind": "login", "site": world["site"].host}
    world["items"] = {
        "Shared": save(
            admin, login | {"name": "Shared", "username": "shared", "secret": SHARED}
        ),
        "Keeper only": save(
            admin,
            {
                "kind": "secret",
                "name": "Keeper only",
                "secret": KEEPER_ONLY,
                "access": {"mode": "explicit_grants", "agents": [keeper]},
            },
        ),
        "Own": save(
            controller,
            login
            | {"agent": keeper, "name": "Own", "username": "keeper-own", "secret": OWN},
        ),
    }
    assert item(world, "Shared")["access"] == {"mode": "all_agents"}
    assert item(world, "Own")["agent"] == keeper
    print("PASS create: an administrator saved two installation items, all agents and")
    print("     Keeper only, and the controller saved one of Keeper's own; no response")
    print("     carries a secret")


def refusals(world):
    people, keeper, other = world["people"], world["keeper"], world["other"]
    admin, controller = people["admin"], people["controller"]
    viewer, outsider = people["viewer"], people["outsider"]
    secret = {"kind": "secret", "name": "x", "secret": "x"}
    save(controller, secret, 403)
    controller.api("/api/vault", expected=403)
    save(viewer, secret | {"agent": keeper}, 403)
    save(outsider, secret | {"agent": keeper}, 403)
    save(controller, secret | {"agent": other}, 403)
    Api(world["base"], world["ca"])("/api/vault", expected=401)
    shared, own = item(world, "Shared")["id"], item(world, "Own")["id"]
    save(controller, secret, 403, "PUT", f"/api/vault/{shared}")
    viewer.api(f"/api/vault/{own}", expected=403, method="DELETE")
    outsider.api(f"/api/vault/{own}", expected=403, method="DELETE")
    login = {"kind": "login", "name": "Bad", "site": "x.example", "username": "u"}
    save(admin, login | {"secret": "pässword"}, 400)
    save(admin, login | {"secret": "tab\there"}, 400)
    save(admin, login | {"username": "ünï", "secret": "ok"}, 400)
    save(admin, {"kind": "secret", "name": "Bad", "site": "x", "secret": "ok"}, 400)
    save(admin, secret | {"agent": keeper, "access": {"mode": "all_agents"}}, 400)
    save(admin, {"kind": "secret", "name": "Shared", "secret": "dup"}, 409)
    print("PASS refusals: the controller cannot list or save installation items or")
    print(
        "     another agent's, a viewer and an outsider change nothing, no credential"
    )
    print("     is 401, and untypable, malformed and duplicate items are refused")


def listings(world):
    people, keeper, other = world["people"], world["keeper"], world["other"]
    everything = ["Keeper only", "Own", "Shared"]
    for name in ("admin", "controller", "viewer"):
        assert names(vault(people[name], keeper)) == everything, name
    vault(people["outsider"], keeper, 403)
    vault(people["controller"], other, 403)
    assert names(vault(people["admin"], other)) == ["Shared"]
    assert names(people["admin"].api("/api/vault")) == ["Keeper only", "Shared"]
    print("PASS listings: Keeper's vault lists its own and both installation items for")
    print("     the admin, controller and viewer; Other's only the all-agents item; an")
    print("     outsider and a person without a grant on Other get 403")


def access(world):
    """Access changes on an installation item reach the agents' listings."""
    admin, keeper, other = world["people"]["admin"], world["keeper"], world["other"]
    shared = item(world, "Shared")
    body = {"kind": "login", "name": "Shared", "site": shared["site"]}
    body |= {"username": "shared", "secret": None}
    only = {"mode": "explicit_grants", "agents": [keeper]}
    save(
        admin, body | {"access": only}, method="PUT", path=f"/api/vault/{shared['id']}"
    )
    assert names(vault(admin, other)) == []
    save(
        admin,
        body | {"access": {"mode": "all_agents"}},
        method="PUT",
        path=f"/api/vault/{shared['id']}",
    )
    assert names(vault(admin, other)) == ["Shared"]
    own = item(world, "Own")
    save(
        world["people"]["controller"],
        {"kind": "login", "name": "Own", "site": own["site"], "username": "keeper-own"}
        | {"secret": None, "access": only},
        400,
        "PUT",
        f"/api/vault/{own['id']}",
    )
    print("PASS access: narrowing Shared to Keeper hid it from Other and widening it")
    print(
        "     back showed it again, keeping its secret; an agent's item takes no access"
    )


def agent_lists(world):
    admin = world["people"]["admin"].api
    for agent, expected in (
        (world["keeper"], ["Keeper only", "Own", "Shared"]),
        (world["other"], ["Shared"]),
    ):
        row = chat(admin, agent, "Call vault_list once and reply with the item names.")
        listed = json.loads(calls(row, "vault_list")[-1]["outcome"]["output"])
        assert names(listed) == expected, listed
        clean(json.dumps(row))
    print(
        "PASS agent list: each real agent's vault_list returned what it may fill, with"
    )
    print("     no secret, from the same rules as the console")


def agent_fill(world):
    """Keeper signs in with Shared, without the model seeing the password."""
    admin, keeper, site = world["people"]["admin"].api, world["keeper"], world["site"]
    shared = item(world, "Shared")["id"]
    before, events = sign_ins(site), len(arbiter_events(keeper))
    row = chat(
        admin,
        keeper,
        f"In browser session main, browser_navigate to {login_page(site)}. Then take a "
        "screenshot with the computer tool, click the User field with the computer "
        f"tool, call vault_fill with item {shared} and field username, click the "
        f"Password field, call vault_fill with item {shared} and field secret, then "
        "press Enter with the computer tool.",
    )
    assert submitted(site, before) == ("shared", SHARED)
    filled = calls(row, "vault_fill")
    assert [call["input"] for call in filled][-2:] == [
        {"item": shared, "field": "username"},
        {"item": shared, "field": "secret"},
    ], filled
    assert all(call["outcome"]["code"] == 0 for call in filled[-2:]), filled
    typed = fills(keeper, events)
    assert [e["who"] for e in typed][-2:] == [{"source": "agent", "item": shared}] * 2
    assert all("characters" in json.dumps(e["action"]) for e in typed), typed
    clean(json.dumps(row))
    last = next(i for i in vault(world["people"]["admin"], keeper) if i["id"] == shared)
    assert last["last_used_at"], last
    print("PASS agent fill: Keeper's model clicked each field and vault_fill typed the")
    print("     username and password; the site received them, the arbiter logged the")
    print("     item and a length only, and the item shows when it was last filled")


def other_scope(world):
    """Other fills the all-agents item, never Keeper's items."""
    admin, other = world["people"]["admin"].api, world["other"]
    for name in ("Keeper only", "Own"):
        before = len(arbiter_events(other))
        row = chat(
            admin,
            other,
            f"Call vault_fill with item {item(world, name)['id']} and field secret "
            "once.",
        )
        outcome = calls(row, "vault_fill")[0]["outcome"]
        assert outcome["code"] == 1 and "no vault item" in outcome["output"], outcome
        assert fills(other, before) == [], fills(other, before)
    before = len(arbiter_events(other))
    shared = item(world, "Shared")["id"]
    row = chat(
        admin,
        other,
        f"Call vault_fill with item {shared} and field username once.",
    )
    assert calls(row, "vault_fill")[0]["outcome"]["code"] == 0, row["terminal"]
    assert [e["who"] for e in fills(other, before)] == [
        {"source": "agent", "item": shared}
    ]
    print(
        "PASS other agent: Other's fills of Keeper's explicit-grant item and Keeper's"
    )
    print("     own item were refused before the arbiter; its fill of the all-agents")
    print("     item reached its own desktop")


def agent_put(world):
    """Keeper saves a password it made; the ledger keeps only the redaction."""
    admin, keeper = world["people"]["admin"].api, world["keeper"]
    row = chat(
        admin,
        keeper,
        "Make up a random password of 20 letters and digits yourself and save it "
        "with vault_put as kind secret, name Generated, site null and username null. "
        "Do not write the password anywhere else.",
    )
    put = calls(row, "vault_put")[-1]
    assert put["outcome"]["code"] == 0, put
    assert put["input"]["secret"] == "[redacted]", put["input"]
    listed = vault(world["people"]["admin"], keeper)
    generated = next(i for i in listed if i["name"] == "Generated")
    assert generated["agent"] == keeper and generated["kind"] == "secret", generated
    world["items"]["Generated"] = generated
    assert "Generated" not in names(vault(world["people"]["admin"], world["other"]))
    row = chat(
        admin,
        keeper,
        "Call vault_put once with kind secret, name Accented, site null, username "
        "null and secret 'naïve-é'.",
    )
    outcome = calls(row, "vault_put")[-1]["outcome"]
    assert outcome["code"] == 1 and "printable ASCII" in outcome["output"], outcome
    assert "Accented" not in names(listed)
    print(
        "PASS agent put: Keeper saved a password it made in its own vault, the ledger"
    )
    print("     shows [redacted], Other cannot see it, and an untypable one is refused")
    print("     with the reason")


def person_fill(world):
    """The controller fills while holding control; nobody else fills or the agent."""
    people, address, ca, keeper, site = (
        world[key] for key in ("people", "address", "ca", "keeper", "site")
    )
    own, generated = item(world, "Own")["id"], item(world, "Generated")["id"]
    path = f"/api/agents/{keeper}/vault/{own}/fill"
    username = {"field": "username"}
    people["controller"].api(path, username, expected=409)
    people["viewer"].api(path, username, expected=403)
    people["outsider"].api(path, username, expected=403)
    controller = people["controller"].desktop(address, ca, keeper)
    controller.frame()
    assert controller.ask({"kind": "take"})["mode"] == "human"
    refused = people["admin"].api(path, username, expected=409)
    assert "control" in refused["error"], refused
    before, events = sign_ins(site), len(arbiter_events(keeper))
    visit(controller, login_page(site))
    time.sleep(3)
    people["controller"].api(path, username, expected=204)
    key(controller, "tab")
    secret = f"/api/agents/{keeper}/vault/{generated}/fill"
    people["controller"].api(secret, {"field": "secret"}, expected=204)
    key(controller, "enter")
    user, password = submitted(site, before)
    assert user == "keeper-own" and re.fullmatch(r"[A-Za-z0-9]{20}", password), user
    world_secrets.append(password)
    sources = [e["who"] for e in fills(keeper, events)]
    account = people["controller"].id
    assert sources == [
        {"source": "person", "account": account, "item": own},
        {"source": "person", "account": account, "item": generated},
    ], sources
    blocked = chat(
        people["admin"].api,
        keeper,
        f"Call vault_fill with item {own} and field username once.",
    )
    outcome = calls(blocked, "vault_fill")[0]["outcome"]
    assert outcome["code"] == 1 and "person has control" in outcome["output"], outcome
    assert controller.ask({"kind": "hand_back"})["mode"] == "watch"
    controller.close()
    stale = chat(
        people["admin"].api,
        keeper,
        f"Call vault_fill with item {own} and field username once, before anything "
        "else.",
    )
    outcome = calls(stale, "vault_fill")[0]["outcome"]
    assert outcome["code"] == 1 and "screenshot" in outcome["output"], outcome
    print("PASS person fill: the controller filled Keeper's own username and the")
    print("     password Keeper made only while holding control, and the site received")
    print("     both; the viewer and outsider got 403, the admin without control 409,")
    print("     and the agent's fill was refused while a person held control and until")
    print("     it looked again after the hand-back")


def handoff(world, postgres):
    """During a login handoff the agent cannot fill; a person saves and fills."""
    people, address, ca, keeper, site = (
        world[key] for key in ("people", "address", "ca", "keeper", "site")
    )
    admin = people["admin"].api
    row = chat(
        admin,
        keeper,
        f"In browser session main, browser_navigate to {login_page(site)}, then call "
        f"request_login with site {site.host!r} and reason 'Save the login', and end "
        "your turn.",
    )
    assert calls(row, "request_login")[0]["outcome"]["code"] == 0, row["terminal"]
    # The controller takes control for the login, types the username and leaves the
    # focus in the password field.
    controller = people["controller"].desktop(address, ca, keeper)
    controller.frame()
    assert controller.ask({"kind": "take"})["mode"] == "human"
    assert modes(keeper)[-1]["handoff"] is True
    own = item(world, "Own")["id"]
    row = chat(admin, keeper, f"Call vault_fill with item {own} and field secret once.")
    outcome = calls(row, "vault_fill")[0]["outcome"]
    assert outcome["code"] == 1 and PENDING in outcome["output"], outcome
    visit(controller, login_page(site))
    time.sleep(3)
    typed = controller.ask(
        {"kind": "input", "action": {"action": "type", "text": "ada"}}
    )
    assert "error" not in typed, typed
    key(controller, "tab")
    controller.close()
    wait_for("the controller's drop", lambda: modes(keeper)[-1]["mode"] != "human", 30)
    before = sign_ins(site)
    console(world)
    saved = next(i for i in vault(people["admin"], keeper) if i["name"] == site.host)
    assert saved["agent"] == keeper and saved["username"] == "ada", saved
    assert pending(world, postgres)[-1][1] == "completed"
    # The handoff is complete; the controller takes control again to submit the form.
    controller = people["controller"].desktop(address, ca, keeper)
    controller.frame()
    assert controller.ask({"kind": "take"})["mode"] == "human"
    key(controller, "enter")
    assert submitted(site, before) == ("ada", SAVED)
    assert controller.ask({"kind": "hand_back"})["mode"] == "watch"
    controller.close()
    print("PASS handoff: the agent's fill was refused during the login handoff; in the")
    print("     console the admin took control, saved the login from the banner with")
    print("     Save and fill, and the site received the password it typed")


def console(world):
    environment = os.environ | {
        "CUBEBOT_CONSOLE": CONSOLE,
        "CUBEBOT_CONSOLE_LOGIN": "vault-admin",
        "CUBEBOT_CONSOLE_PASSWORD": world["people"]["admin"].password,
        "CUBEBOT_EVIDENCE_DIRECTORY": str(world["evidence"]),
        "CUBEBOT_VAULT_SITE": world["site"].host,
        "CUBEBOT_VAULT_SECRET": SAVED,
        "CUBEBOT_VAULT_SECRETS": json.dumps(SECRETS),
    }
    script = Path(__file__).with_name("docker_vault_browser.mjs")
    result = subprocess.run(
        ["node", script], env=environment, capture_output=True, text=True, timeout=600
    )
    print(result.stdout + result.stderr)
    assert result.returncode == 0, "the console check failed"


def deleted(world):
    people, keeper = world["people"], world["keeper"]
    own = item(world, "Own")["id"]
    people["controller"].api(f"/api/vault/{own}", expected=204, method="DELETE")
    people["controller"].api(f"/api/vault/{own}", expected=404, method="DELETE")
    assert "Own" not in names(vault(people["admin"], keeper))
    row = chat(
        people["admin"].api,
        keeper,
        "Take a screenshot with the computer tool, then call vault_fill with item "
        f"{own} and field username once.",
    )
    outcome = calls(row, "vault_fill")[-1]["outcome"]
    assert outcome["code"] == 1 and "no vault item" in outcome["output"], outcome
    print(
        "PASS delete: the controller deleted Keeper's item, deleting it again is 404,"
    )
    print("     and the agent's fill of it is refused")


def secrets_kept(world, postgres):
    dump = subprocess.run(
        ["docker", "exec", postgres, "pg_dump", "-U", "cubebot", "cubebot_test"],
        capture_output=True,
        text=True,
        check=True,
    ).stdout
    assert "vault_items" in dump and "[redacted]" in dump
    texts = {"database": dump, "orchestrator": world["orchestrator_log"].read_text()}
    for agent in ("keeper", "other"):
        texts[agent] = logs(world[agent])
    for where, text in texts.items():
        for secret in SECRETS + world_secrets:
            assert secret not in text, (where, secret[:3])
    print(
        f"PASS secrets: none of the {len(SECRETS) + len(world_secrets)} secrets is in"
    )
    print("     the database dump, the orchestrator's log or either sandbox's log")


def scenarios(world, postgres):
    items(world)
    refusals(world)
    listings(world)
    access(world)
    agent_lists(world)
    agent_fill(world)
    other_scope(world)
    agent_put(world)
    person_fill(world)
    handoff(world, postgres)
    deleted(world)
    secrets_kept(world, postgres)


def agent(admin, name):
    identity = admin("/api/agents", command(name=name, instructions=INSTRUCTIONS), 201)[
        "id"
    ]
    current = admin(f"/api/agents/{identity}")
    admin(
        f"/api/agents/{identity}/definition",
        command(
            expected_revision=current["revision"],
            name=name,
            instructions=INSTRUCTIONS,
            idle_policy=current["idle_policy"],
            idle_delay_seconds=86400,
            dreaming_model=None,
        ),
    )
    lifecycle(admin, identity, "wake")
    return identity


def setup(admin, base, ca, password):
    admin_id = admin("/api/identity/me")["id"]
    key = Path(os.environ["OPENAI_API_KEY_FILE"]).read_text().strip()
    provider = command(
        expected_revision=0,
        api_key=key,
        model=MODEL,
        provider="openai",
        context_window=128000,
    )
    admin("/api/provider", provider)
    keeper, other = agent(admin, "Keeper"), agent(admin, "Other")
    people = {"admin": Person(admin, "vault-admin", password, admin_id)}
    for name in ["viewer", "controller", "outsider"]:
        people[name] = person(admin, base, ca, f"vault-{name}")
    grant(admin, people["viewer"].id, keeper, "browser_view")
    grant(admin, people["controller"].id, keeper, "browser_view")
    grant(admin, people["controller"].id, keeper, "browser_control")
    return {"people": people, "keeper": keeper, "other": other}


def qualify(directory: Path):
    with socket.socket() as probe:
        probe.bind((bridge(), 0))
        address = f"{bridge()}:{probe.getsockname()[1]}"
    environment = configure(directory, address)
    config = directory / "docker.json"
    config.write_text(
        json.dumps(json.loads(config.read_text()) | {"computer_use_enabled": True})
    )
    environment["CUBEBOT_PUBLIC_ORIGIN"] = CONSOLE
    base, ca = f"https://{address}", directory / "ca/ca.crt"
    admin = Api(base, ca)
    world = {"address": address, "base": base, "ca": ca, "evidence": directory}
    world["orchestrator_log"] = directory / "orchestrator.log"

    def workflow(postgres, env):
        environment["CUBEBOT_DATABASE_URL"] = env["CUBEBOT_TEST_DATABASE_URL"]
        binary = [str(ROOT / "target/debug/cubebot-orchestrator")]
        with open(world["orchestrator_log"], "w") as log:
            process = subprocess.Popen(
                binary,
                env=os.environ | environment,
                stdout=log,
                stderr=subprocess.STDOUT,
            )
        surrounding = []
        try:
            wait_for("orchestrator HTTPS", lambda: live(admin), timeout=30)
            password = secrets.token_urlsafe(24)
            bootstrap(admin, env["CUBEBOT_TEST_DATABASE_URL"], "vault-admin", password)
            world.update(setup(admin, base, ca, password))
            surrounding = surroundings(world, address, ca)
            ready(admin, world["keeper"])
            ready(admin, world["other"])
            world["site"] = Site()
            scenarios(world, postgres)
        except BaseException:
            print(world["orchestrator_log"].read_text()[-20000:])
            for name in ("keeper", "other"):
                if world.get(name) and container(world[name]):
                    print(logs(world[name])[-10000:])
            if os.environ.get("CUBEBOT_VAULT_HOLD"):
                traceback.print_exc()
                print("HOLDING for inspection", flush=True)
                time.sleep(900)
            raise
        finally:
            subprocess.run(
                ["docker", "rm", "-f", *surrounding], check=False, capture_output=True
            )
            for name in ("keeper", "other"):
                if world.get(name):
                    remove(world[name])
            process.terminate()
            process.wait(timeout=30)
        print("PASS vault on real agents, from every scope and person's side")

    with tempfile.TemporaryDirectory(
        prefix="cubebot-postgres-", dir=ROOT / "target"
    ) as data:
        password = secrets.token_hex(24)
        name = f"cubebot-test-{uuid.uuid4().hex}"
        env = os.environ | {"POSTGRES_PASSWORD": password}
        run_database(name, env, password, os.path.abspath(data), workflow)


def main():
    if (refused := unsupported("docker-vault")) is not None:
        return refused
    build()
    subprocess.run(
        ["docker", "build", "--tag", "cubebot-web:local"]
        + ["--file", "apps/web/Dockerfile", "."],
        check=True,
        cwd=ROOT,
    )
    keep = os.environ.get("CUBEBOT_VAULT_EVIDENCE")
    with tempfile.TemporaryDirectory() as directory:
        qualify(Path(keep or directory))
    return 0


if __name__ == "__main__":
    sys.exit(main())
