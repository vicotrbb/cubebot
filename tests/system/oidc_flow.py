"""Verify signed OIDC tokens and browser sessions through actual HTTP processes."""

import http.cookiejar
import json
import secrets
import subprocess
import tempfile
import time
import urllib.error
import urllib.request
import uuid
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

from oidc_issuer import Issuer
from smoke import BINARY, ROOT, await_ready, running, unused_port


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *_args):
        return None


class Browser:
    def __init__(self, base):
        self.base = base
        self.opener = urllib.request.build_opener(
            NoRedirect(), urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar())
        )

    def request(self, path, value=None, origin=True, method=None):
        url = path if path.startswith("http") else self.base + path
        headers = {}
        data = None
        if value is not None:
            data = json.dumps(value).encode()
            headers["Content-Type"] = "application/json"
        if (value is not None or method) and origin:
            headers["Origin"] = self.base
        request = urllib.request.Request(url, data=data, headers=headers, method=method)
        try:
            response = self.opener.open(request, timeout=10)
        except urllib.error.HTTPError as error:
            response = error
        with response:
            body = response.read()
            return response.code, response.headers, json.loads(body) if body else {}

    def begin(self, path="/api/identity/oidc/start"):
        status, _, body = self.request(path, {})
        assert status == 200, (status, body)
        status, headers, _ = self.request(body["authorization_url"])
        assert status == 302
        return headers["Location"]

    def sign_in(self):
        callback = self.begin()
        assert self.request(callback)[0] == 303
        status, _, account = self.request("/api/identity/me")
        assert status == 200
        return account


def rejected_tokens(base, issuer):
    for override in (
        {"nonce": "wrong"},
        {"aud": "another-client"},
        {"exp": int(time.time()) - 60},
        {"iss": "https://wrong.example"},
        {"at_hash": "wrong"},
    ):
        browser = Browser(base)
        callback = browser.begin()
        issuer.overrides = override
        assert browser.request(callback)[0] == 401, override
        issuer.overrides = {}


def check_browser(base, issuer, password):
    browser = Browser(base)
    callback = browser.begin()
    assert Browser(base).request(callback)[0] == 401
    issuer.rotate()
    assert browser.request(callback)[0] == 303
    account = browser.request("/api/identity/me")[2]
    assert account["administrator"] is False
    assert browser.request(callback)[0] == 401
    assert Browser(base).sign_in() == account
    assert browser.request("/api/identity/logout", {}, origin=False)[0] == 403
    assert browser.request("/api/identity/logout", {})[0] == 200
    assert browser.request("/api/identity/me")[0] == 401
    rejected_tokens(base, issuer)
    issuer.subject = "unlisted-subject"
    assert Browser(base).request(Browser(base).begin())[0] == 401
    denied = Browser(base)
    assert denied.request(denied.begin())[0] == 403
    check_linking(base, issuer, password)
    issuer.subject = "employee-1"
    return Browser(base), account


def check_linking(base, issuer, password):
    issuer.subject = "linked-operator"
    operator = Browser(base)
    assert (
        operator.request(
            "/api/identity/login", {"login": "operator", "password": password}
        )[0]
        == 200
    )
    expected = operator.request("/api/identity/me")[2]
    callback = operator.begin("/api/identity/oidc/link")
    assert operator.request(callback)[0] == 303
    assert operator.request("/api/identity/me")[2] == expected
    assert Browser(base).sign_in() == expected
    issuer.subject = "abandoned-link"
    callback = operator.begin("/api/identity/oidc/link")
    assert operator.request("/api/identity/logout", {})[0] == 200
    assert operator.request(callback)[0] == 401
    issuer.subject = "linked-operator"
    check_revocation(base, issuer, password)


def check_revocation(base, issuer, password):
    operator = Browser(base)
    assert (
        operator.request(
            "/api/identity/login", {"login": "operator", "password": password}
        )[0]
        == 200
    )
    status, _, bindings = operator.request("/api/identity/oidc/bindings")
    assert status == 200
    binding = next(item for item in bindings if item["subject"] == "linked-operator")
    command = {
        "id": binding["id"],
        "expected_revision": binding["revision"],
        "request_id": str(uuid.uuid4()),
    }
    signed_in = Browser(base)
    signed_in.sign_in()
    result = operator.request("/api/identity/oidc/revoke", command)
    assert result[0] == 200
    assert operator.request("/api/identity/oidc/revoke", command)[2] == result[2]
    assert signed_in.request("/api/identity/me")[0] == 401
    assert operator.request("/api/identity/me")[0] == 200
    denied = Browser(base)
    assert denied.request(denied.begin())[0] == 403


def exercise(environment, issuer, directory):
    base = f"http://127.0.0.1:{unused_port()}"
    config = Path(directory) / "oidc.json"
    config.write_text(
        json.dumps(
            {
                "issuer": issuer.url,
                "client_id": "cubebot-test",
                "redirect_uri": base + "/api/identity/oidc/callback",
                "admit_subjects": ["employee-1"],
            }
        )
    )
    password = secrets.token_hex(24)
    env = environment | {
        "CUBEBOT_DATABASE_URL": environment["CUBEBOT_TEST_DATABASE_URL"],
        "CUBEBOT_BIND_ADDR": base.removeprefix("http://"),
        "CUBEBOT_PUBLIC_ORIGIN": base,
        "CUBEBOT_OIDC_CONFIG": str(config),
        "CUBEBOT_OIDC_CLIENT_SECRET": issuer.client_secret,
    }
    subprocess.run(
        [str(ROOT / "target/debug/cubebot-admin"), "bootstrap"],
        env=env
        | {
            "CUBEBOT_BOOTSTRAP_LOGIN": "operator",
            "CUBEBOT_BOOTSTRAP_PASSWORD": password,
        },
        check=True,
        capture_output=True,
    )
    with running([str(BINARY)], ROOT, env) as process:
        await_ready(process, base + "/api/health/ready")
        browser, account = check_browser(base, issuer, password)
        assert browser.sign_in() == account
        issuer.discovery_status = 500
        assert Browser(base).request("/api/identity/oidc/start", {})[0] == 503
        issuer.discovery_status = 200
    with running([str(BINARY)], ROOT, env) as process:
        await_ready(process, base + "/api/health/ready")
        assert browser.request("/api/identity/me")[2] == account
        assert Browser(base).sign_in() == account
    domain_config = json.loads(config.read_text())
    domain_config["admit_subjects"] = []
    domain_config["admit_email_domains"] = ["example.com"]
    config.write_text(json.dumps(domain_config))
    with running([str(BINARY)], ROOT, env) as process:
        await_ready(process, base + "/api/health/ready")
        assert browser.request("/api/identity/me")[0] == 401
        browser = check_domain_admission(base, issuer, password)
        assert browser.request("/api/identity/me")[0] == 200
    del env["CUBEBOT_OIDC_CONFIG"]
    with running([str(BINARY)], ROOT, env) as process:
        await_ready(process, base + "/api/health/ready")
        assert browser.request("/api/identity/me")[0] == 401
        assert (
            Browser(base).request(
                "/api/identity/login", {"login": "operator", "password": password}
            )[0]
            == 200
        )
    return password


def operator(base, password):
    browser = Browser(base)
    login = {"login": "operator", "password": password}
    assert browser.request("/api/identity/login", login)[0] == 200
    return browser


def exercise_console(environment, issuer, directory, password):
    """Single sign-on set up in the console, applied without a restart."""
    base = f"http://127.0.0.1:{unused_port()}"
    key = Path(directory) / "credential.key"
    key.write_bytes(secrets.token_bytes(32))
    key.chmod(0o600)
    env = environment | {
        "CUBEBOT_DATABASE_URL": environment["CUBEBOT_TEST_DATABASE_URL"],
        "CUBEBOT_BIND_ADDR": base.removeprefix("http://"),
        "CUBEBOT_PUBLIC_ORIGIN": base,
        "CUBEBOT_CREDENTIAL_KEY_FILE": str(key),
    }
    settings = "/api/identity/oidc/settings"
    provider = {
        "issuer": issuer.url,
        "client_id": "cubebot-test",
        "client_secret": issuer.client_secret,
        "admit_subjects": ["console-member"],
    }
    issuer.subject = "console-member"
    with running([str(BINARY)], ROOT, env) as process:
        await_ready(process, base + "/api/health/ready")
        assert Browser(base).request("/api/identity/oidc")[2] == {"enabled": False}
        assert Browser(base).request("/api/identity/oidc/start", {})[0] == 404
        admin = operator(base, password)
        current = admin.request(settings)[2]
        assert current["managed"] is False and current["enabled"] is False
        assert current["redirect_uri"] == base + "/api/identity/oidc/callback"
        assert current["can_store_secret"] is True
        member = Browser(base)
        assert member.request(settings, provider, method="PUT")[0] == 401
        issuer.discovery_status = 500
        assert admin.request(settings, provider, method="PUT")[0] == 503
        issuer.discovery_status = 200
        assert admin.request(settings)[2]["enabled"] is False
        status, _, saved = admin.request(settings, provider, method="PUT")
        assert status == 200, saved
        assert saved["enabled"] is True and saved["has_client_secret"] is True
        assert "client_secret" not in saved
        assert Browser(base).request("/api/identity/oidc")[2] == {"enabled": True}
        account = member.sign_in()
        assert account["administrator"] is False
    with running([str(BINARY)], ROOT, env) as process:
        await_ready(process, base + "/api/health/ready")
        # The stored provider and its encrypted secret outlive a restart.
        assert member.request("/api/identity/me")[2] == account
        assert Browser(base).sign_in() == account
        admin = operator(base, password)
        assert admin.request(settings, method="DELETE")[0] == 204
        assert member.request("/api/identity/me")[0] == 401
        assert Browser(base).request("/api/identity/oidc/start", {})[0] == 404
    deployment = Path(directory) / "deployment.json"
    deployment.write_text(
        json.dumps(
            {
                "issuer": issuer.url,
                "client_id": "cubebot-test",
                "redirect_uri": base + "/api/identity/oidc/callback",
            }
        )
    )
    env["CUBEBOT_OIDC_CONFIG"] = str(deployment)
    with running([str(BINARY)], ROOT, env) as process:
        await_ready(process, base + "/api/health/ready")
        admin = operator(base, password)
        current = admin.request(settings)[2]
        assert current["managed"] is True and current["enabled"] is True
        assert admin.request(settings, provider, method="PUT")[0] == 409
        assert admin.request(settings, method="DELETE")[0] == 409
    issuer.subject = "employee-1"


def check_domain_admission(base, issuer, password):
    issuer.subject = "domain-member"
    issuer.overrides = {"email_verified": False}
    denied = Browser(base)
    assert denied.request(denied.begin())[0] == 403
    issuer.overrides = {"email": None, "email_verified": None}
    issuer.userinfo_overrides = {"sub": "another-person"}
    denied = Browser(base)
    assert denied.request(denied.begin())[0] == 401
    issuer.userinfo_overrides = {}
    with ThreadPoolExecutor(max_workers=2) as workers:
        accounts = list(workers.map(lambda _: Browser(base).sign_in(), range(2)))
    admitted = accounts[0]
    assert admitted["administrator"] is False
    assert accounts == [admitted, admitted]
    check_oidc_suspension(base, issuer, password, admitted)
    issuer.overrides = {}
    issuer.subject = "wrong-domain"
    issuer.email = "employee@another.example"
    denied = Browser(base)
    assert denied.request(denied.begin())[0] == 403
    issuer.email = "employee@example.com"
    issuer.discovery_padding = "x" * 1_048_577
    assert Browser(base).request("/api/identity/oidc/start", {})[0] == 503
    issuer.discovery_padding = ""
    issuer.subject = "active-domain-member"
    browser = Browser(base)
    browser.sign_in()
    return browser


def check_oidc_suspension(base, issuer, password, member):
    operator = Browser(base)
    assert (
        operator.request(
            "/api/identity/login", {"login": "operator", "password": password}
        )[0]
        == 200
    )
    member_browser = Browser(base)
    assert member_browser.sign_in() == member
    command = {
        "id": member["id"],
        "expected_revision": 1,
        "request_id": str(uuid.uuid4()),
    }
    assert operator.request("/api/identity/suspend", command)[0] == 200
    assert member_browser.request("/api/identity/me")[0] == 401
    denied = Browser(base)
    assert denied.request(denied.begin())[0] == 401


def check_oidc(environment):
    issuer = Issuer()
    try:
        with tempfile.TemporaryDirectory(prefix="cubebot-oidc-config-") as directory:
            password = exercise(environment, issuer, directory)
            exercise_console(environment, issuer, directory, password)
    finally:
        issuer.close()
    print(
        "OIDC workflow passed: signed tokens, PKCE, browser binding, replay, "
        "rotation, admission, linking, revocation, CSRF, restart, policy invalidation, "
        "console setup"
    )
