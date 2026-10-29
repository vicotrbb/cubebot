"""Build the kvm guest: a kernel and a read-only ext4 root image with the agent as init.

Writes only target/guest/bzImage and target/guest/root.ext4, the sandbox image's `guest`
build context; sources and intermediate files go to target/guest-build. Needs x86_64
Linux with make, a C compiler, flex, bison, bc and libelf headers for the kernel, and
Docker for the root image, which is apps/agent/Dockerfile exported to ext4.
"""

import hashlib
import os
import shutil
import subprocess
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
HERE = Path(__file__).resolve().parent
OUT = ROOT / "target/guest"
BUILD = ROOT / "target/guest-build"
VERSION = "6.12.114"
SHA256 = "7a8589912bd7a8adb07fed074283652cf482a35facef8954554654f0154b4af3"
URL = f"https://cdn.kernel.org/pub/linux/kernel/v6.x/linux-{VERSION}.tar.xz"
IMAGE = "cubebot-guest-root"


def run(*command, **options):
    subprocess.run([str(part) for part in command], check=True, **options)


def source() -> Path:
    tree = BUILD / f"linux-{VERSION}"
    if tree.is_dir():
        return tree
    archive = BUILD / f"linux-{VERSION}.tar.xz"
    partial = archive.with_suffix(".part")
    urllib.request.urlretrieve(URL, partial)
    if hashlib.sha256(partial.read_bytes()).hexdigest() != SHA256:
        partial.unlink()
        raise SystemExit(f"{URL}: checksum mismatch")
    partial.rename(archive)
    run("tar", "-xf", archive, "-C", BUILD)
    archive.unlink()
    return tree


def kernel(tree: Path):
    build = BUILD / "kernel"
    fragment = HERE / "kernel.config"
    shutil.copy(fragment, tree / "arch/x86/configs/cubebot.config")
    make = ["make", "-C", tree, f"O={build}", "ARCH=x86_64"]
    run(*make, "tinyconfig", "cubebot.config")
    # Kconfig silently drops symbols whose dependencies are unmet.
    config = set((build / ".config").read_text().splitlines())
    wanted = [
        line for line in fragment.read_text().splitlines() if line.startswith("CONFIG_")
    ]
    missing = [line for line in wanted if line not in config]
    if missing:
        raise SystemExit(f"kernel config dropped {missing}")
    run(*make, f"-j{os.cpu_count()}", "bzImage")
    shutil.copy(build / "arch/x86/boot/bzImage", OUT / "bzImage")


def root_image():
    run(
        "docker",
        "build",
        "--platform",
        "linux/amd64",
        "-t",
        IMAGE,
        "-f",
        ROOT / "apps/agent/Dockerfile",
        ROOT,
    )
    created = subprocess.run(
        ["docker", "create", "--platform", "linux/amd64", IMAGE],
        check=True,
        capture_output=True,
        text=True,
    )
    container = created.stdout.strip()
    archive = BUILD / "root.tar"
    try:
        with open(archive, "wb") as tar:
            run("docker", "export", container, stdout=tar)
    finally:
        run("docker", "rm", container, stdout=subprocess.DEVNULL)
    # Half again the tree, plus room for ext4 metadata; the guest never writes its root.
    size = archive.stat().st_size * 3 // 2 + (64 << 20)
    # Extracting as root in a container keeps file ownership without root on the host.
    # Docker leaves its resolv.conf; guest init binds the kernel's over an empty one.
    script = (
        "mkdir /rootfs && tar -xpf /build/root.tar -C /rootfs"
        " && rm -f /rootfs/etc/resolv.conf && : > /rootfs/etc/resolv.conf"
        " && mkfs.ext4 -q -F -L root -E root_owner=0:0 -d /rootfs"
        f" /out/root.ext4.partial {size // 1024}k"
    )
    run(
        "docker",
        "run",
        "--rm",
        "-v",
        f"{OUT}:/out",
        "-v",
        f"{BUILD}:/build",
        "debian:bookworm-slim",
        "sh",
        "-euc",
        script,
    )
    (OUT / "root.ext4.partial").rename(OUT / "root.ext4")
    archive.unlink()


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    BUILD.mkdir(parents=True, exist_ok=True)
    kernel(source())
    root_image()
    print(f"Guest kernel and root image in {OUT}")


if __name__ == "__main__":
    main()
