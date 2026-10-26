"""Compare network controls in disposable, owned Linux containers; no cluster claims."""

import hashlib
import json
import select
import subprocess
import sys
import time
import uuid
from contextlib import contextmanager
from datetime import UTC, datetime
from pathlib import Path


def run(*args, input=None):
    result = subprocess.run(
        args, input=input, text=True, capture_output=True, check=False
    )
    if result.returncode:
        raise RuntimeError(result.stderr.strip())
    return result.stdout.strip()


def docker(*args, input=None):
    return run("docker", *args, input=input)


def build(builder, prefix):
    images = {}
    for target in ("runtime", "tools", "qualification"):
        print(f"Building isolated {target} image", flush=True)
        image = f"{prefix}-{target}"
        docker(
            "buildx",
            "build",
            "--builder",
            builder,
            "--load",
            "--target",
            target,
            "--label",
            f"org.cubebot.fixture={prefix}",
            "--tag",
            image,
            "--file",
            "apps/agent/Dockerfile",
            ".",
        )
        images[target] = image
    return images


def container(image, prefix, *args, command=(), input=None):
    return docker(
        "run",
        "--rm",
        "--label",
        f"org.cubebot.fixture={prefix}",
        "--read-only",
        "--cap-drop",
        "ALL",
        "--security-opt",
        "no-new-privileges",
        *(["--interactive"] if input is not None else []),
        *args,
        image,
        *command,
        input=input,
    )


def initialize(images, prefix, volume):
    mount = f"type=volume,src={volume},dst=/storage"
    container(
        images["runtime"],
        prefix,
        "--user",
        "0:0",
        "--cap-add",
        "CHOWN",
        "--mount",
        mount,
        "--entrypoint",
        "/usr/bin/install",
        command=("-d", "-m", "2770", "-o", "0", "-g", "20000", "/storage"),
    )
    for role, uid in (("runtime", "10001"), ("tools", "10002")):
        container(
            images["runtime"],
            prefix,
            "--user",
            f"{uid}:{uid}",
            "--group-add",
            "20000",
            "--mount",
            mount,
            command=("--initialize-storage", role),
        )


def start_runtime(image, prefix, volume):
    return docker(
        "run",
        "--detach",
        "--name",
        prefix,
        "--label",
        f"org.cubebot.fixture={prefix}",
        "--read-only",
        "--cap-drop",
        "ALL",
        "--security-opt",
        "no-new-privileges",
        "--network",
        "none",
        "--sysctl",
        "net.ipv6.conf.all.disable_ipv6=0",
        "--mount",
        f"type=volume,src={volume},dst=/var/lib/cubebot,volume-subpath=runtime",
        image,
        "serve",
    )


def ready(name):
    for _ in range(100):
        output = docker("logs", name)
        if output:
            return json.loads(output.splitlines()[0])
        time.sleep(0.1)
    raise RuntimeError("isolation fixture did not start")


def probe(image, prefix, runtime, *command, extra=()):
    return json.loads(
        container(
            image,
            prefix,
            "--user",
            "10002:10002",
            "--group-add",
            "20000",
            "--network",
            f"container:{runtime}",
            *extra,
            command=command,
        )
    )


def helper(image, prefix, runtime, entrypoint, *command, input=None):
    return container(
        image,
        prefix,
        "--user",
        "0:0",
        "--cap-add",
        "NET_ADMIN",
        "--network",
        f"container:{runtime}",
        "--entrypoint",
        entrypoint,
        command=command,
        input=input,
    )


def addresses(image, prefix, runtime):
    helper(
        image,
        prefix,
        runtime,
        "/usr/sbin/ip",
        "address",
        "add",
        "8.8.4.4/32",
        "dev",
        "lo",
    )
    helper(
        image,
        prefix,
        runtime,
        "/usr/sbin/ip",
        "-6",
        "address",
        "add",
        "2001:4860::8888/128",
        "dev",
        "lo",
        "nodad",
    )


def network(image, prefix, runtime):
    targets = {
        "private_v4": "127.0.0.1:18080",
        "public_v4": "8.8.4.4:18080",
        "private_v6": "[::1]:18081",
        "public_v6": "[2001:4860::8888]:18081",
    }
    return {
        key: probe(image, prefix, runtime, "connect", value)["connected"]
        for key, value in targets.items()
    }


def policies(image, prefix, runtime):
    # These are mechanism probes, not the complete production destination classifier.
    rules = """table inet cubebot_probe {
 chain output { type filter hook output priority 0; policy accept;
  ip daddr 127.0.0.0/8 drop
  ip6 daddr ::1 drop
 }
}
"""
    helper(image, prefix, runtime, "/usr/sbin/nft", "-f", "-", input=rules)
    shared = json.loads(
        docker(
            "exec",
            runtime,
            "/usr/local/bin/isolation-probe",
            "connect",
            "127.0.0.1:18080",
        )
    )["connected"]
    helper(
        image,
        prefix,
        runtime,
        "/usr/sbin/nft",
        "delete",
        "table",
        "inet",
        "cubebot_probe",
    )
    rules = rules.replace("  ip daddr", "  meta skuid 10001 accept\n  ip daddr")
    helper(image, prefix, runtime, "/usr/sbin/nft", "-f", "-", input=rules)
    scoped = network(image, prefix, runtime)
    runtime_private = json.loads(
        docker(
            "exec",
            runtime,
            "/usr/local/bin/isolation-probe",
            "connect",
            "127.0.0.1:18080",
        )
    )["connected"]
    return {
        "shared_filter_runtime_private": shared,
        "uid_filter_tools": scoped,
        "uid_filter_runtime_private": runtime_private,
    }


def finite_policy(image, prefix, runtime):
    # Never accept conntrack-established packets ahead of current grant membership.
    rules = """delete table inet cubebot_probe
table inet cubebot_probe {
 set private4 { type ipv4_addr . inet_service; flags timeout; }
 set private6 { type ipv6_addr . inet_service; flags timeout; }
 chain output { type filter hook output priority 0; policy accept;
  meta skuid 10001 accept
  meta skuid 10002 ip daddr . tcp dport @private4 accept
  meta skuid 10002 ip6 daddr . tcp dport @private6 accept
  ip daddr 127.0.0.0/8 drop
  ip6 daddr ::1 drop
 }
}
"""
    helper(image, prefix, runtime, "/usr/sbin/nft", "-f", "-", input=rules)


def grant_element(image, prefix, runtime, family, operation, timeout=None, port=None):
    address, default_port = {4: ("127.0.0.1", 18080), 6: ("::1", 18081)}[family]
    port = port or default_port
    lifetime = f" timeout {timeout}s" if timeout else ""
    rule = (
        f"{operation} element inet cubebot_probe private{family} "
        f"{{ {address} . {port}{lifetime} }}\n"
    )
    helper(image, prefix, runtime, "/usr/sbin/nft", "-f", "-", input=rule)


def session_event(process, request=None):
    if request:
        process.stdin.write((request + "\n").encode())
        process.stdin.flush()
    readable, _, _ = select.select([process.stdout], [], [], 10)
    if not readable:
        raise RuntimeError("persistent socket probe did not respond")
    event = process.stdout.readline()
    if not event:
        raise RuntimeError("persistent socket probe exited before response")
    return json.loads(event)["connected"]


@contextmanager
def socket_session(runtime, address):
    process = subprocess.Popen(
        [
            "docker",
            "exec",
            "--interactive",
            "--user",
            "10002:10002",
            runtime,
            "/usr/local/bin/isolation-probe",
            "session",
            address,
        ],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        bufsize=0,
    )
    try:
        assert session_event(process), "private socket did not open under grant"
        yield process
    finally:
        process.stdin.close()
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait(timeout=5)
        process.stdout.close()
        process.stderr.close()


def finite_family(image, prefix, runtime, family):
    address, other = {
        4: ("127.0.0.1:18080", "127.0.0.1:18082"),
        6: ("[::1]:18081", "[::1]:18083"),
    }[family]
    assert not probe(image, prefix, runtime, "connect", address)["connected"]
    # Establish each socket before timing expiry or explicitly revoking its element.
    grant_element(image, prefix, runtime, family, "add", timeout=5)
    with socket_session(runtime, address) as process:
        assert session_event(process, request="exchange")
        assert not probe(image, prefix, runtime, "connect", other)["connected"]
        time.sleep(5.2)
        expired_existing = session_event(process, request="exchange")
    expired_new = probe(image, prefix, runtime, "connect", address)["connected"]
    grant_element(image, prefix, runtime, family, "add", timeout=30)
    with socket_session(runtime, address) as process:
        assert session_event(process, request="exchange")
        grant_element(image, prefix, runtime, family, "delete")
        revoked_existing = session_event(process, request="exchange")
    revoked_new = probe(image, prefix, runtime, "connect", address)["connected"]
    assert not any((expired_existing, expired_new, revoked_existing, revoked_new))
    return {
        "granted_connection_and_exchange": True,
        "ungranted_port_denied": True,
        "expired_existing_exchange": expired_existing,
        "expired_new_connection": expired_new,
        "revoked_existing_exchange": revoked_existing,
        "revoked_new_connection": revoked_new,
    }


def finite_grants(image, prefix, runtime):
    finite_policy(image, prefix, runtime)
    # Prove control-port listeners exist, so refused ports cannot fake policy denial.
    for address in ("127.0.0.1:18082", "[::1]:18083"):
        result = json.loads(
            docker(
                "exec", runtime, "/usr/local/bin/isolation-probe", "connect", address
            )
        )
        assert result["connected"]
    return {
        f"ipv{family}": finite_family(image, prefix, runtime, family)
        for family in (4, 6)
    }


def duplex_policy(image, prefix, runtime):
    rules = """delete table inet cubebot_probe
table inet cubebot_probe {
 set private4 { type ipv4_addr . inet_service; flags timeout; }
 set private6 { type ipv6_addr . inet_service; flags timeout; }
 chain output { type filter hook output priority 0; policy accept;
  ct direction original meta skuid 10001 ct mark set 1
  ct direction original meta skuid 10002 ct mark set 2
  ct mark 1 accept
  meta skuid 10001 accept
  meta skuid 10002 ip daddr . tcp dport @private4 accept
  meta skuid 10002 ip6 daddr . tcp dport @private6 accept
  ip daddr 127.0.0.0/8 drop
  ip6 daddr ::1 drop
 }
 chain input { type filter hook input priority 0; policy accept;
  ct mark 1 accept
  ct mark 2 ct direction original ip daddr . tcp dport @private4 accept
  ct mark 2 ct direction original ip6 daddr . tcp dport @private6 accept
  ct mark 2 ct direction reply ip saddr . tcp sport @private4 accept
  ct mark 2 ct direction reply ip6 saddr . tcp sport @private6 accept
  ip saddr 127.0.0.0/8 drop
  ip6 saddr ::1 drop
 }
}
"""
    helper(image, prefix, runtime, "/usr/sbin/nft", "-f", "-", input=rules)


def pushed_reply(runtime, control, process):
    # The trusted sender emits only after this independent control request, not a timer.
    sent = json.loads(
        docker("exec", runtime, "/usr/local/bin/isolation-probe", "connect", control)
    )["connected"]
    assert sent, "trusted sender failed to emit fresh data"
    return session_event(process, request="receive")


def push_family(image, prefix, runtime, family, expected):
    address, control, port = {
        4: ("127.0.0.1:18088", "127.0.0.1:18089", 18088),
        6: ("[::1]:18090", "[::1]:18091", 18090),
    }[family]
    grant_element(image, prefix, runtime, family, "add", timeout=5, port=port)
    with socket_session(runtime, address) as process:
        assert pushed_reply(runtime, control, process)
        time.sleep(5.2)
        expired = pushed_reply(runtime, control, process)
    grant_element(image, prefix, runtime, family, "add", timeout=30, port=port)
    with socket_session(runtime, address) as process:
        assert pushed_reply(runtime, control, process)
        grant_element(image, prefix, runtime, family, "delete", port=port)
        revoked = pushed_reply(runtime, control, process)
    assert expired is expected and revoked is expected
    return {"fresh_push_after_expiry": expired, "fresh_push_after_revocation": revoked}


def bidirectional_grants(image, prefix, runtime):
    output_only = {
        f"ipv{family}": push_family(image, prefix, runtime, family, expected=True)
        for family in (4, 6)
    }
    duplex_policy(image, prefix, runtime)
    existing = {
        f"ipv{family}": finite_family(image, prefix, runtime, family)
        for family in (4, 6)
    }
    duplex = {
        f"ipv{family}": push_family(image, prefix, runtime, family, expected=False)
        for family in (4, 6)
    }
    reachable = network(image, prefix, runtime)
    assert reachable == {
        "private_v4": False,
        "public_v4": True,
        "private_v6": False,
        "public_v6": True,
    }
    return {
        "output_only_leaks_incoming_data": output_only,
        "duplex_request_reply_grants": existing,
        "duplex_blocks_incoming_data": duplex,
        "duplex_default_paths": reachable,
    }


def cannot_flush(image, prefix, runtime):
    try:
        container(
            image,
            prefix,
            "--user",
            "10002:10002",
            "--network",
            f"container:{runtime}",
            "--entrypoint",
            "/usr/sbin/nft",
            command=("flush", "ruleset"),
        )
    except RuntimeError as error:
        if "Operation not permitted" not in str(error):
            raise
        return True
    raise AssertionError("tool UID changed kernel network policy")


def image_metadata(images):
    metadata = {
        role: json.loads(docker("image", "inspect", image, "--format", "{{json .}}"))
        for role, image in images.items()
    }
    assert metadata["runtime"]["Config"]["User"] == "10001:10001"
    assert metadata["tools"]["Config"]["User"] == "10002:10002"
    return {
        role: {
            "id": data["Id"],
            "architecture": data["Architecture"],
            "os": data["Os"],
            "user": data["Config"]["User"],
            "entrypoint": data["Config"]["Entrypoint"],
        }
        for role, data in metadata.items()
    }


def fingerprint():
    files = [
        Path(name)
        for name in (
            "Cargo.toml",
            "Cargo.lock",
            "rust-toolchain.toml",
            ".dockerignore",
            "tests/system/container_isolation.py",
        )
    ]
    for root in ("apps/agent", "apps/orchestrator", "modules"):
        files.extend(
            path
            for path in Path(root).rglob("*")
            if path.is_file()
            and (path.suffix in {".rs", ".sql", ".toml"} or path.name == "Dockerfile")
        )
    digest = hashlib.sha256()
    for path in sorted(files):
        digest.update(str(path).encode() + b"\0" + path.read_bytes())
    return digest.hexdigest()


def profile(images):
    platform = json.loads(
        docker(
            "info",
            "--format",
            '{"os":{{json .OSType}},"architecture":{{json .Architecture}},'
            '"cpus":{{json .NCPU}},"memory_bytes":{{json .MemTotal}},'
            '"kernel":{{json .KernelVersion}}}',
        )
    )
    return {
        "timestamp": datetime.now(UTC).isoformat(),
        "base_commit": run("git", "rev-parse", "HEAD"),
        "source_sha256": fingerprint(),
        "docker_version": docker("version", "--format", "{{.Server.Version}}"),
        "platform": platform,
        "images": image_metadata(images),
        "nftables": container(
            images["qualification"],
            "version-probe",
            "--entrypoint",
            "/usr/sbin/nft",
            command=("--version",),
        ),
    }


def qualify(images, prefix, volume):
    initialize(images, prefix, volume)
    runtime = start_runtime(images["qualification"], prefix, volume)
    actor = ready(runtime)
    addresses(images["qualification"], prefix, runtime)
    filesystem = probe(
        images["qualification"],
        prefix,
        runtime,
        "filesystem",
        extra=(
            "--mount",
            f"type=volume,src={volume},dst=/storage",
            "--mount",
            f"type=volume,src={volume},dst=/home/agent,volume-subpath=tools",
        ),
    )
    assert filesystem["uid"] == 10002 and actor["uid"] == 10001
    assert filesystem["pid_namespace"] != actor["pid_namespace"]
    assert not any(
        filesystem[key]
        for key in (
            "reads_runtime_token",
            "reads_runtime_journal",
            "writes_runtime_directory",
            "links_runtime_token",
        )
    )
    unrestricted = network(images["qualification"], prefix, runtime)
    proxy_ignored = probe(
        images["qualification"],
        prefix,
        runtime,
        "connect",
        "127.0.0.1:18080",
        extra=("--env", "HTTPS_PROXY=http://127.0.0.1:9"),
    )["connected"]
    results = policies(images["qualification"], prefix, runtime)
    assert all(unrestricted.values()) and proxy_ignored
    assert not results["shared_filter_runtime_private"]
    assert results["uid_filter_tools"] == {
        "private_v4": False,
        "public_v4": True,
        "private_v6": False,
        "public_v6": True,
    }
    assert results["uid_filter_runtime_private"]
    results["tool_cannot_flush_rules"] = cannot_flush(
        images["qualification"], prefix, runtime
    )
    assert not network(images["qualification"], prefix, runtime)["private_v4"]
    return {
        "filesystem": filesystem,
        "without_filter": unrestricted,
        "direct_socket_ignores_proxy": proxy_ignored,
        "finite_private_grants": finite_grants(
            images["qualification"], prefix, runtime
        ),
        "bidirectional_private_grants": bidirectional_grants(
            images["qualification"], prefix, runtime
        ),
        **results,
    }


def missing_resource(message):
    text = message.lower()
    return any(
        marker in text
        for marker in (
            "no such container:",
            "no such volume",
            "no such image:",
            "no builder ",
        )
    )


def remove_owned(command, resource):
    try:
        result = subprocess.run(command, text=True, capture_output=True, check=False)
    except OSError:
        return f"{resource}: cleanup command unavailable"
    if result.returncode and not missing_resource(result.stderr):
        return f"{resource}: cleanup exited {result.returncode}"
    return None


def cleanup(prefix, builder):
    commands = [
        (["docker", "rm", "--force", prefix], prefix),
        (["docker", "volume", "rm", prefix], f"volume:{prefix}"),
        (["docker", "buildx", "rm", "--force", builder], builder),
    ]
    commands.extend(
        (["docker", "image", "rm", f"{prefix}-{target}"], f"{prefix}-{target}")
        for target in ("runtime", "tools", "qualification")
    )
    failures = []
    for command, resource in commands:
        failure = remove_owned(command, resource)
        if failure:
            failures.append(failure)
    return failures


def write_report(path, report):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(report, indent=2) + "\n")


def main():
    prefix = f"cubebot-isolation-{uuid.uuid4().hex}"
    builder = f"{prefix}-build"
    failed = False
    try:
        docker("buildx", "create", "--name", builder, "--driver", "docker-container")
        images = build(builder, prefix)
        docker("volume", "create", "--label", f"org.cubebot.fixture={prefix}", prefix)
        metadata = profile(images)
        results = qualify(images, prefix, prefix)
        report = {
            "profile": metadata,
            "scope": "Linux container mechanism probes; G1-G3 not qualified",
            "results": results,
        }
        write_report(Path("target/container-isolation.json"), report)
        print(json.dumps(report, indent=2))
    except BaseException:
        failed = True
        raise
    finally:
        failures = cleanup(prefix, builder)
        if failures:
            print("Isolation cleanup failed: " + "; ".join(failures), file=sys.stderr)
            if not failed:
                raise RuntimeError("isolation fixture cleanup failed")


if __name__ == "__main__":
    main()
