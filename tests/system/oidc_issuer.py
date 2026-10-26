"""Disposable standards-shaped OIDC issuer with real RSA signatures and PKCE checks."""

import base64
import hashlib
import json
import secrets
import subprocess
import tempfile
import threading
import time
import urllib.parse
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path


def encoded(value):
    return base64.urlsafe_b64encode(value).rstrip(b"=").decode()


class Issuer:
    def __init__(self):
        self.directory = tempfile.TemporaryDirectory(prefix="cubebot-oidc-")
        self.key = Path(self.directory.name) / "signing.pem"
        self.client_secret = secrets.token_hex(24)
        self.codes = {}
        self.tokens = {}
        self.subject = "employee-1"
        self.email = "employee@example.com"
        self.overrides = {}
        self.userinfo_overrides = {}
        self.discovery_status = 200
        self.discovery_padding = ""
        self.rotate()
        owner = self

        class Handler(BaseHTTPRequestHandler):
            def do_GET(self):
                owner.get(self)

            def do_POST(self):
                owner.token(self)

            def log_message(self, *_args):
                pass

        self.server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.url = f"http://127.0.0.1:{self.server.server_port}"
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()

    def rotate(self):
        subprocess.run(
            [
                "openssl",
                "genpkey",
                "-algorithm",
                "RSA",
                "-pkeyopt",
                "rsa_keygen_bits:2048",
                "-out",
                str(self.key),
            ],
            check=True,
            capture_output=True,
        )
        modulus = (
            subprocess.run(
                ["openssl", "rsa", "-in", str(self.key), "-noout", "-modulus"],
                check=True,
                capture_output=True,
                text=True,
            )
            .stdout.strip()
            .split("=", 1)[1]
        )
        self.kid = secrets.token_hex(8)
        self.jwk = {
            "kty": "RSA",
            "use": "sig",
            "alg": "RS256",
            "kid": self.kid,
            "n": encoded(bytes.fromhex(modulus)),
            "e": "AQAB",
        }

    def close(self):
        self.server.shutdown()
        self.server.server_close()
        self.thread.join(timeout=5)
        self.directory.cleanup()

    def reply(self, handler, body, status=200):
        data = json.dumps(body).encode()
        handler.send_response(status)
        handler.send_header("Content-Type", "application/json")
        handler.send_header("Content-Length", str(len(data)))
        handler.end_headers()
        handler.wfile.write(data)

    def get(self, handler):
        path = urllib.parse.urlsplit(handler.path)
        routes = {
            "/.well-known/openid-configuration": self.discovery,
            "/jwks": self.jwks,
            "/authorize": self.authorize,
            "/userinfo": self.userinfo,
        }
        route = routes.get(path.path)
        if route is None:
            self.reply(handler, {}, 404)
            return
        route(handler)

    def discovery(self, handler):
        self.reply(
            handler,
            {
                "issuer": self.url,
                "authorization_endpoint": self.url + "/authorize",
                "token_endpoint": self.url + "/token",
                "jwks_uri": self.url + "/jwks",
                "userinfo_endpoint": self.url + "/userinfo",
                "response_types_supported": ["code"],
                "subject_types_supported": ["public"],
                "id_token_signing_alg_values_supported": ["RS256"],
                "padding": self.discovery_padding,
            },
            self.discovery_status,
        )

    def jwks(self, handler):
        self.reply(handler, {"keys": [self.jwk]})

    def authorize(self, handler):
        query = urllib.parse.parse_qs(urllib.parse.urlsplit(handler.path).query)
        code = secrets.token_urlsafe(24)
        self.codes[code] = {key: values[0] for key, values in query.items()}
        redirect = query["redirect_uri"][0]
        handler.send_response(302)
        handler.send_header(
            "Location",
            redirect
            + "?"
            + urllib.parse.urlencode({"code": code, "state": query["state"][0]}),
        )
        handler.end_headers()

    def token(self, handler):
        expected = (
            "Basic "
            + base64.b64encode(("cubebot-test:" + self.client_secret).encode()).decode()
        )
        if handler.headers.get("Authorization") != expected:
            self.reply(handler, {"error": "invalid_client"}, 401)
            return
        size = int(handler.headers.get("Content-Length", "0"))
        query = urllib.parse.parse_qs(handler.rfile.read(size).decode())
        code = query.get("code", [""])[0]
        grant = self.codes.pop(code, None)
        if grant is None:
            self.reply(handler, {"error": "invalid_grant"}, 400)
            return
        challenge = encoded(
            hashlib.sha256(query.get("code_verifier", [""])[0].encode()).digest()
        )
        if (
            challenge != grant.get("code_challenge")
            or grant.get("code_challenge_method") != "S256"
        ):
            self.reply(handler, {"error": "invalid_grant"}, 400)
            return
        if query.get("redirect_uri", [""])[0] != grant.get("redirect_uri"):
            self.reply(handler, {"error": "invalid_grant"}, 400)
            return
        self.issue(handler, grant)

    def issue(self, handler, grant):
        access = secrets.token_urlsafe(24)
        claims = {
            "iss": self.url,
            "aud": grant["client_id"],
            "sub": self.subject,
            "iat": int(time.time()),
            "exp": int(time.time()) + 300,
            "nonce": grant["nonce"],
            "email": self.email,
            "email_verified": True,
            "roles": ["administrator"],
            "at_hash": encoded(hashlib.sha256(access.encode()).digest()[:16]),
        } | self.overrides
        self.tokens[access] = {
            "sub": self.subject,
            "email": self.email,
            "email_verified": True,
        } | self.userinfo_overrides
        header = encoded(json.dumps({"alg": "RS256", "kid": self.kid}).encode())
        payload = encoded(json.dumps(claims).encode())
        message = f"{header}.{payload}".encode()
        signature = subprocess.run(
            ["openssl", "dgst", "-sha256", "-sign", str(self.key)],
            input=message,
            capture_output=True,
            check=True,
        ).stdout
        self.reply(
            handler,
            {
                "access_token": access,
                "token_type": "Bearer",
                "expires_in": 300,
                "id_token": f"{header}.{payload}.{encoded(signature)}",
            },
        )

    def userinfo(self, handler):
        access = handler.headers.get("Authorization", "").removeprefix("Bearer ")
        claims = self.tokens.get(access)
        self.reply(handler, claims or {}, 200 if claims else 401)
