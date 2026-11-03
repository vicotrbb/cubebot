#!/bin/sh
# Installs Cubebot with Docker Compose on Linux or macOS:
#   curl -fsSL https://raw.githubusercontent.com/vicotrbb/cubebot/main/deploy/install.sh | sh
# Agents run as KVM guests where /dev/kvm exists on x86_64 Linux, else as containers. Optional
# environment: CUBEBOT_VERSION (a release tag; default the latest), CUBEBOT_DOMAIN (serve it on
# 80 and 443 with a public certificate), CUBEBOT_RUNTIME (kvm, container or off), CUBEBOT_PORT,
# CUBEBOT_HOME (default ~/.cubebot), CUBEBOT_REGISTRY and CUBEBOT_SOURCE (where images and files come from).
set -eu

die() {
    echo "cubebot: $*" >&2
    exit 1
}

command -v curl >/dev/null || die "curl is required"
command -v docker >/dev/null || die "Docker is required: https://docs.docker.com/engine/install/"
docker info >/dev/null 2>&1 || die "cannot reach the Docker daemon; is it running, and may this user use it?"
# Compose files carry inline configs, which need Compose 2.23.
version=$(docker compose version --short 2>/dev/null) || die "Docker Compose v2 is required"
major=${version%%.*}
minor=${version#*.}
minor=${minor%%.*}
[ "$major" -gt 2 ] || { [ "$major" -eq 2 ] && [ "$minor" -ge 23 ]; } || die "Docker Compose 2.23 or later is required; this is $version"

system=$(uname -s)
case "$system" in
    Linux | Darwin) ;;
    *) die "Cubebot installs on Linux or macOS; this is $system" ;;
esac
runtime=${CUBEBOT_RUNTIME:-}
if [ -z "$runtime" ]; then
    if [ "$system/$(uname -m)" = Linux/x86_64 ] && [ -c /dev/kvm ] && [ -c /dev/net/tun ]; then
        runtime=kvm
    else
        runtime=container
    fi
fi
case "$runtime" in
    kvm | container | off) ;;
    *) die "CUBEBOT_RUNTIME is kvm, container or off" ;;
esac

release=${CUBEBOT_VERSION:-}
if [ -z "$release" ]; then
    release=$(curl -fsSIo /dev/null -w '%{redirect_url}' https://github.com/vicotrbb/cubebot/releases/latest) || die "could not find the latest release"
    release=${release##*/}
    # Without a release, GitHub redirects to the releases page.
    case "$release" in "" | releases) die "could not find the latest release" ;; esac
fi
home=${CUBEBOT_HOME:-$HOME/.cubebot}
source=${CUBEBOT_SOURCE:-https://raw.githubusercontent.com/vicotrbb/cubebot/$release}

umask 077
mkdir -p "$home"
curl -fsSL "$source/deploy/cubebot" -o "$home/cubebot.new" || die "could not download $source/deploy/cubebot"
chmod 755 "$home/cubebot.new"
mv "$home/cubebot.new" "$home/cubebot"

# An existing .env holds the database password of existing data: keep it.
fresh=""
if [ ! -f "$home/.env" ]; then
    fresh=1
    {
        echo "POSTGRES_PASSWORD=$(od -An -N16 -tx1 /dev/urandom | tr -d ' \n')"
        echo "CUBEBOT_RUNTIME=$runtime"
        for name in CUBEBOT_DOMAIN CUBEBOT_PORT CUBEBOT_REGISTRY CUBEBOT_SOURCE; do
            eval "value=\${$name:-}"
            [ -z "$value" ] || echo "$name=$value"
        done
    } >"$home/.env"
    echo "Wrote $home/.env (agents: $runtime)."
fi

CUBEBOT_HOME="$home" "$home/cubebot" update "$release"
if [ -n "$fresh" ]; then
    echo "Create the administrator:"
    CUBEBOT_HOME="$home" "$home/cubebot" admin ||
        echo "Create it later with: $home/cubebot admin"
fi
echo "Manage this installation with $home/cubebot up | down | update | logs | admin."
