"""Build the guest kernel and an initramfs whose init is cubebot-agent.

Writes target/guest. Needs x86_64 Linux with make, a C compiler, flex, bison, bc,
libelf headers and musl-gcc.
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
VERSION = "6.12.114"
SHA256 = "7a8589912bd7a8adb07fed074283652cf482a35facef8954554654f0154b4af3"
URL = f"https://cdn.kernel.org/pub/linux/kernel/v6.x/linux-{VERSION}.tar.xz"
TARGET = "x86_64-unknown-linux-musl"


def run(*command, **options):
    subprocess.run([str(part) for part in command], check=True, **options)


def source() -> Path:
    tree = OUT / f"linux-{VERSION}"
    if tree.is_dir():
        return tree
    archive = OUT / f"linux-{VERSION}.tar.xz"
    partial = archive.with_suffix(".part")
    urllib.request.urlretrieve(URL, partial)
    if hashlib.sha256(partial.read_bytes()).hexdigest() != SHA256:
        partial.unlink()
        raise SystemExit(f"{URL}: checksum mismatch")
    partial.rename(archive)
    run("tar", "-xf", archive, "-C", OUT)
    archive.unlink()
    return tree


def kernel(tree: Path) -> Path:
    build = OUT / "kernel"
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
    return build


def initramfs(build: Path):
    run("rustup", "target", "add", TARGET)
    run(
        "cargo",
        "build",
        "--locked",
        "--release",
        "-p",
        "cubebot-agent",
        "--target",
        TARGET,
        cwd=ROOT,
    )
    agent = ROOT / "target" / TARGET / "release/cubebot-agent"
    listing = OUT / "initramfs.list"
    # The kernel opens /dev/console for init's standard streams.
    listing.write_text(
        "dir /dev 0755 0 0\n"
        "nod /dev/console 0600 0 0 c 5 1\n"
        f"file /init {agent} 0755 0 0\n"
    )
    with open(OUT / "initramfs.cpio", "wb") as cpio:
        run(build / "usr/gen_init_cpio", listing, stdout=cpio)


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    initramfs(kernel(source()))
    print(f"Guest kernel and initramfs in {OUT}")


if __name__ == "__main__":
    main()
