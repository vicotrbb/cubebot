"""Owned Kubernetes/CSI text-chat fixture: installs deploy/chart with Helm. Never reads
the default kubeconfig."""

import base64
import json
import os
import secrets
import ssl
import subprocess
import time
import urllib.error
import urllib.request

import cluster_admission as admission

NAMESPACE = "cubebot-chat"
ORIGIN = "https://127.0.0.1:33443"
# The fixture's ingress controller; sandboxes reach the orchestrator through it.
TRAEFIK = "oci://ghcr.io/traefik/helm/traefik"
TRAEFIK_VERSION = "41.7.1"
CONTROL = "https://traefik.traefik.svc.cluster.local"
CONTROL_PORT = 8443
# As docker_sandbox.py: kvm where /dev/kvm exists, else container.
RUNTIME = os.environ.get("CUBEBOT_SANDBOX_RUNTIME") or (
    "kvm" if os.path.exists("/dev/kvm") else "container"
)


def wait_for(description, operation, timeout=180):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        result = operation()
        if result:
            return result
        time.sleep(1)
    raise RuntimeError(f"timed out waiting for {description}")


class Fixture:
    def __init__(self, config, name, artifacts, images):
        self.config, self.name, self.artifacts = config, name, artifacts
        self.images = images
        self.token = None
        self.password = secrets.token_hex(24)
        self.context = None

    def k(self, *args, **kwargs):
        return admission.kubectl(self.config, "-n", NAMESPACE, *args, **kwargs)

    def create(self, kind, name, namespace=NAMESPACE, **fields):
        document = dict(apiVersion="v1", kind=kind, metadata={"name": name})
        document.update(fields)
        admission.kubectl(
            self.config,
            "-n",
            namespace,
            "create",
            "-f",
            "-",
            input=json.dumps(document),
        )

    def pod(self, name, containers, **fields):
        self.create("Pod", name, spec=dict(containers=containers, **fields))

    def platform(self, app):
        """The chart's one pod labelled `app`, as JSON."""
        items = json.loads(self.k("get", "pods", "-l", f"app={app}", "-o", "json"))
        (item,) = items["items"]
        return item

    def controller(self):
        """The ingress controller pod's address."""
        items = json.loads(
            admission.kubectl(
                self.config,
                "-n",
                "traefik",
                "get",
                "pods",
                "-l",
                "app.kubernetes.io/name=traefik",
                "-o",
                "json",
            )
        )
        (item,) = items["items"]
        return item["status"]["podIP"]

    def sql(self, statement):
        return self.k(
            "exec",
            "postgres-0",
            "--",
            "psql",
            "-U",
            "cubebot",
            "-d",
            "cubebot",
            "-At",
            "-v",
            "ON_ERROR_STOP=1",
            "-c",
            statement,
        )

    def request(self, path, body=None, token=None, expected=200):
        headers = {"Content-Type": "application/json"}
        credential = self.token if token is None else token
        if credential:
            headers["Authorization"] = f"Bearer {credential}"
        data = None if body is None else json.dumps(body).encode()
        request = urllib.request.Request(ORIGIN + path, data=data, headers=headers)
        try:
            with urllib.request.urlopen(
                request, context=self.context, timeout=15
            ) as response:
                status, raw = response.status, response.read()
        except urllib.error.HTTPError as error:
            status, raw = error.code, error.read()
        assert status == expected, f"{path}: HTTP {status}, expected {expected}"
        return json.loads(raw) if raw else None

    def save(self, name, value):
        (self.artifacts / name).write_text(json.dumps(value, indent=2) + "\n")


def certificate(f):
    directory = f.artifacts / "private"
    directory.mkdir(mode=0o700)
    key, cert = directory / "tls.key", directory / "tls.crt"
    ca_key, ca = directory / "ca.key", directory / "ca.crt"
    admission.run(
        "openssl",
        "req",
        "-x509",
        "-newkey",
        "rsa:2048",
        "-nodes",
        "-keyout",
        str(ca_key),
        "-out",
        str(ca),
        "-days",
        "1",
        "-subj",
        "/CN=cubebot-disposable-ca",
        "-addext",
        "basicConstraints=critical,CA:TRUE",
        "-addext",
        "keyUsage=critical,keyCertSign,cRLSign",
    )
    csr, extensions = directory / "server.csr", directory / "server.cnf"
    admission.run(
        "openssl",
        "req",
        "-new",
        "-newkey",
        "rsa:2048",
        "-nodes",
        "-keyout",
        str(key),
        "-out",
        str(csr),
        "-subj",
        "/CN=cubebot-disposable",
    )
    extensions.write_text(
        "basicConstraints=critical,CA:FALSE\nextendedKeyUsage=serverAuth\n"
        "keyUsage=critical,digitalSignature,keyEncipherment\n"
        "subjectKeyIdentifier=hash\nauthorityKeyIdentifier=keyid,issuer\n"
        f"subjectAltName=IP:127.0.0.1,DNS:{CONTROL.removeprefix('https://')}\n"
    )
    admission.run(
        "openssl",
        "x509",
        "-req",
        "-in",
        str(csr),
        "-CA",
        str(ca),
        "-CAkey",
        str(ca_key),
        "-CAcreateserial",
        "-out",
        str(cert),
        "-days",
        "1",
        "-extfile",
        str(extensions),
    )
    key.chmod(0o600)
    ca_key.chmod(0o600)
    f.context = ssl.create_default_context(cafile=str(ca))
    f.create("Namespace", "traefik")
    data = {
        "tls.key": base64.b64encode(key.read_bytes()).decode(),
        "tls.crt": base64.b64encode(cert.read_bytes()).decode(),
    }
    # The Ingress names it; Traefik also serves it to clients that send no SNI.
    for namespace in (NAMESPACE, "traefik"):
        f.create("Secret", "tls", namespace, type="kubernetes.io/tls", data=data)
    return ca.read_text()


def helm(f, *args):
    admission.run(
        "helm",
        "--kubeconfig",
        str(f.config),
        *args,
        "--wait",
        "--timeout",
        "5m",
        timeout=330,
    )


def ingress_controller(f):
    values = {
        "service": {"type": "ClusterIP"},
        "tlsStore": {"default": {"defaultCertificate": {"secretName": "tls"}}},
        # The agents' control stream and the console's live updates stay open.
        "ports": {
            "websecure": {"transport": {"respondingTimeouts": {"readTimeout": 0}}}
        },
    }
    path = f.artifacts / "traefik-values.json"
    path.write_text(json.dumps(values))
    helm(
        f,
        "install",
        "traefik",
        TRAEFIK,
        "--version",
        TRAEFIK_VERSION,
        "--namespace",
        "traefik",
        "--values",
        str(path),
    )


def setup(f, dns, **sandbox):
    """`sandbox` adds to the chart's sandbox values, e.g. terminalEnabled."""
    ca = certificate(f)
    ingress_controller(f)
    image = f.images["sandbox" if RUNTIME == "kvm" else "agent"]
    values = {
        "images": {
            "orchestrator": f.images["orchestrator"],
            "web": f.images["web"],
            "sandbox" if RUNTIME == "kvm" else "agent": image,
        },
        "publicOrigin": ORIGIN,
        "controlOrigin": CONTROL,
        "controlCA": ca,
        "ingress": {"tlsSecret": "tls"},
        "storage": {"size": "2Gi"},
        "postgres": {"size": "2Gi"},
        "sandbox": {
            "runtime": RUNTIME,
            "dns": [dns],
            "networkPolicy": {
                "ingressController": {
                    "namespaceSelector": {
                        "matchLabels": {"kubernetes.io/metadata.name": "traefik"}
                    },
                    "podSelector": {
                        "matchLabels": {"app.kubernetes.io/name": "traefik"}
                    },
                    "port": CONTROL_PORT,
                }
            },
            **sandbox,
        },
    }
    path = f.artifacts / "values.json"
    path.write_text(json.dumps(values, indent=2))
    helm(
        f,
        "install",
        "cubebot",
        "deploy/chart",
        "--namespace",
        NAMESPACE,
        "--values",
        str(path),
    )
    f.k(
        "exec",
        "-i",
        "deploy/orchestrator",
        "--",
        "/bin/sh",
        "-c",
        "read -r CUBEBOT_BOOTSTRAP_PASSWORD && export CUBEBOT_BOOTSTRAP_PASSWORD && "
        "CUBEBOT_BOOTSTRAP_LOGIN=admin exec cubebot-admin bootstrap",
        input=f.password + "\n",
    )


def forward(f):
    log = (f.artifacts / "port-forward.log").open("w")
    process = subprocess.Popen(
        [
            "kubectl",
            "--kubeconfig",
            str(f.config),
            "-n",
            "traefik",
            "port-forward",
            "service/traefik",
            "33443:443",
            "--address=127.0.0.1",
        ],
        stdout=log,
        stderr=log,
    )
    log.close()
    return process


def available(f):
    try:
        return f.request("/api/health/live", token="")["status"] == "alive"
    except (OSError, AssertionError):
        return False
