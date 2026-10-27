"""Owned Kubernetes/CSI text-chat fixture. Never reads the default kubeconfig."""

import base64
import json
import secrets
import ssl
import subprocess
import time
import urllib.error
import urllib.request

import cluster_admission as admission

NAMESPACE = "cubebot-chat"
ORIGIN = "https://127.0.0.1:33443"
CONTROL = f"https://gateway.{NAMESPACE}.svc.cluster.local"


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

    def create(self, kind, name, **fields):
        version = "v1"
        if kind in ("Role", "RoleBinding", "ClusterRole", "ClusterRoleBinding"):
            version = "rbac.authorization.k8s.io/v1"
        document = dict(apiVersion=version, kind=kind, metadata={"name": name})
        document.update(fields)
        self.k("create", "-f", "-", input=json.dumps(document))

    def pod(self, name, containers, **fields):
        self.create("Pod", name, spec=dict(containers=containers, **fields))

    def service(self, name, port):
        self.create(
            "Service",
            name,
            spec={"selector": {"app": name}, "ports": [{"port": port}]},
        )

    def labeled_pod(self, name, container, **fields):
        self.k(
            "create",
            "-f",
            "-",
            input=json.dumps(
                {
                    "apiVersion": "v1",
                    "kind": "Pod",
                    "metadata": {"name": name, "labels": {"app": name}},
                    "spec": {"containers": [container], **fields},
                }
            ),
        )

    def ready(self, name):
        self.k(
            "wait",
            "--for=condition=Ready",
            f"pod/{name}",
            "--timeout=240s",
            timeout=250,
        )

    def sql(self, statement):
        return self.k(
            "exec",
            "postgres",
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
        f"subjectAltName=IP:127.0.0.1,DNS:gateway.{NAMESPACE}.svc.cluster.local\n"
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
    f.create(
        "Secret",
        "tls",
        data={
            "tls.key": base64.b64encode(key.read_bytes()).decode(),
            "tls.crt": base64.b64encode(cert.read_bytes()).decode(),
        },
    )
    f.create("ConfigMap", "control-ca", data={"ca.crt": ca.read_text()})


def rbac(f):
    f.k("create", "serviceaccount", "orchestrator")
    f.create(
        "Role",
        "orchestrator",
        rules=[
            {
                "apiGroups": [""],
                "resources": ["pods", "persistentvolumeclaims", "configmaps"],
                "verbs": ["get", "create", "patch", "delete"],
            }
        ],
    )
    subject = {"kind": "ServiceAccount", "name": "orchestrator", "namespace": NAMESPACE}
    f.create(
        "RoleBinding",
        "orchestrator",
        subjects=[subject],
        roleRef={
            "apiGroup": "rbac.authorization.k8s.io",
            "kind": "Role",
            "name": "orchestrator",
        },
    )
    f.create(
        "ClusterRole",
        f.name,
        rules=[
            {
                "apiGroups": ["authentication.k8s.io"],
                "resources": ["tokenreviews"],
                "verbs": ["create"],
            }
        ],
    )
    f.create(
        "ClusterRoleBinding",
        f.name,
        subjects=[subject],
        roleRef={
            "apiGroup": "rbac.authorization.k8s.io",
            "kind": "ClusterRole",
            "name": f.name,
        },
    )


def database(f):
    f.create("Secret", "database", stringData={"password": f.password})
    f.create(
        "PersistentVolumeClaim",
        "postgres",
        spec={
            "accessModes": ["ReadWriteOncePod"],
            "resources": {"requests": {"storage": "2Gi"}},
        },
    )
    container = {
        "name": "postgres",
        "image": "postgres:18-bookworm",
        "env": [
            {"name": "POSTGRES_USER", "value": "cubebot"},
            {"name": "POSTGRES_DB", "value": "cubebot"},
            {
                "name": "POSTGRES_PASSWORD",
                "valueFrom": {"secretKeyRef": {"name": "database", "key": "password"}},
            },
        ],
        "volumeMounts": [{"name": "data", "mountPath": "/var/lib/postgresql"}],
        "readinessProbe": {"exec": {"command": ["pg_isready", "-U", "cubebot"]}},
    }
    f.labeled_pod(
        "postgres",
        container,
        automountServiceAccountToken=False,
        volumes=[{"name": "data", "persistentVolumeClaim": {"claimName": "postgres"}}],
    )
    f.service("postgres", 5432)
    f.ready("postgres")


def orchestrator(f, dns):
    config = {
        "connection": {"kind": "in_cluster"},
        "runtime_image": f.images["runtime"],
        "tools_image": f.images.get("tools", f.images["runtime"]),
        "terminal_enabled": "tools" in f.images,
        "runtime_service_account": "cubebot-runtime",
        "orchestrator_origin": CONTROL,
        "network": {"dns": [dns], "infrastructure": []},
        "control_ca_config_map": "control-ca",
    }
    f.create("ConfigMap", "controller", data={"config.json": json.dumps(config)})
    f.create(
        "Secret",
        "credentials",
        data={
            "key": base64.b64encode(secrets.token_bytes(32)).decode(),
        },
    )
    f.create(
        "Secret",
        "application",
        stringData={
            "database_url": f"postgres://cubebot:{f.password}@postgres:5432/cubebot",
            "bootstrap_password": f.password,
        },
    )
    container = {
        "name": "orchestrator",
        "image": f.images["orchestrator"],
        "env": orchestrator_environment(),
        "volumeMounts": [
            {"name": "controller", "mountPath": "/etc/controller", "readOnly": True},
            {"name": "credentials", "mountPath": "/etc/credentials", "readOnly": True},
        ],
        "securityContext": {"runAsUser": 65532, "runAsGroup": 65532},
    }
    f.labeled_pod(
        "orchestrator",
        container,
        serviceAccountName="orchestrator",
        securityContext={"fsGroup": 65532},
        initContainers=[
            {
                "name": "private-key",
                "image": f.images["orchestrator"],
                "command": ["/bin/sh", "-c", "umask 077; cp /source/key /private/key"],
                "securityContext": {"runAsUser": 65532, "runAsGroup": 65532},
                "volumeMounts": [
                    {"name": "source-key", "mountPath": "/source", "readOnly": True},
                    {"name": "credentials", "mountPath": "/private"},
                ],
            }
        ],
        volumes=[
            {"name": "controller", "configMap": {"name": "controller"}},
            {"name": "credentials", "emptyDir": {"medium": "Memory"}},
            {
                "name": "source-key",
                "secret": {"secretName": "credentials", "defaultMode": 288},
            },
        ],
    )
    f.service("orchestrator", 8080)
    f.ready("orchestrator")
    f.k("exec", "orchestrator", "--", "cubebot-admin", "bootstrap")


def orchestrator_environment():
    values = {
        "CUBEBOT_BIND_ADDR": "0.0.0.0:8080",
        "CUBEBOT_PUBLIC_ORIGIN": ORIGIN,
        "CUBEBOT_CONTROLLER_CONFIG": "/etc/controller/config.json",
        "CUBEBOT_CREDENTIAL_KEY_FILE": "/etc/credentials/key",
        "CUBEBOT_BOOTSTRAP_LOGIN": "admin",
    }
    env = [{"name": key, "value": value} for key, value in values.items()]
    for key, field in (
        ("CUBEBOT_POD_NAMESPACE", "metadata.namespace"),
        ("CUBEBOT_POD_NAME", "metadata.name"),
        ("CUBEBOT_POD_UID", "metadata.uid"),
    ):
        env.append({"name": key, "valueFrom": {"fieldRef": {"fieldPath": field}}})
    for key, field in (
        ("CUBEBOT_DATABASE_URL", "database_url"),
        ("CUBEBOT_BOOTSTRAP_PASSWORD", "bootstrap_password"),
    ):
        env.append(
            {
                "name": key,
                "valueFrom": {"secretKeyRef": {"name": "application", "key": field}},
            }
        )
    return env


def gateway(f):
    config = """events {}\nhttp { access_log off; server { listen 443 ssl;
ssl_certificate /tls/tls.crt; ssl_certificate_key /tls/tls.key;
location /api/ { proxy_pass http://orchestrator:8080; }
location / { proxy_pass http://web:3000; }
}}"""
    f.create("ConfigMap", "gateway", data={"nginx.conf": config})
    f.labeled_pod(
        "gateway",
        {
            "name": "gateway",
            "image": "nginx:1.28-bookworm",
            "volumeMounts": [
                {
                    "name": "config",
                    "mountPath": "/etc/nginx/nginx.conf",
                    "subPath": "nginx.conf",
                },
                {"name": "tls", "mountPath": "/tls", "readOnly": True},
            ],
        },
        automountServiceAccountToken=False,
        volumes=[
            {"name": "config", "configMap": {"name": "gateway"}},
            {"name": "tls", "secret": {"secretName": "tls"}},
        ],
    )
    f.service("gateway", 443)
    f.ready("gateway")


def forward(f):
    log = (f.artifacts / "port-forward.log").open("w")
    process = subprocess.Popen(
        [
            "kubectl",
            "--kubeconfig",
            str(f.config),
            "-n",
            NAMESPACE,
            "port-forward",
            "service/gateway",
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


def setup(f, dns):
    certificate(f)
    rbac(f)
    database(f)
    orchestrator(f, dns)
    f.labeled_pod(
        "web",
        {"name": "web", "image": f.images["web"]},
        automountServiceAccountToken=False,
    )
    f.service("web", 3000)
    f.ready("web")
    gateway(f)
