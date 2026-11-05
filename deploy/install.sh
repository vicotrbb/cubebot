#!/usr/bin/env bash
# Installs Cubebot with Docker Compose on Linux or macOS:
#   curl -fsSL https://cubebot.io/install.sh | bash
# Setting a question's variable skips it; CUBEBOT_YES=1, or no terminal, takes every default.
# Also: CUBEBOT_VERSION (default latest), CUBEBOT_PORT (3000), CUBEBOT_REGISTRY, CUBEBOT_SOURCE, NO_COLOR.
# Written for bash 3.2, macOS's /bin/bash; /bin/sh is not bash everywhere.
case "${BASH_VERSION:-}:${SHELLOPTS:-}" in
    :* | *posix*)
        echo "cubebot: run the installer with bash: curl -fsSL https://cubebot.io/install.sh | bash" >&2
        exit 1
        ;;
esac
set -euo pipefail

# Rows: variable|asked when|choices|default|question|explanation. Choices are value=label pairs split by ";", a
# leading "!" marking one this machine cannot run, or "text" or "secret". Rows re-expand so a default can follow an answer.
questions() {
    cat <<EOF
CUBEBOT_RUNTIME|fresh|${kvm_mark}kvm=KVM guests;container=Containers;off=No agents|$runtime_default|Where should agents run?|KVM gives every agent its own virtual machine and needs x86_64 Linux with /dev/kvm and /dev/net/tun; $kvm_here. Containers run every agent as a Docker container, a weaker boundary that works on any machine. No agents runs only the console. Agents start through the Docker socket, which makes Cubebot root on this machine.
CUBEBOT_ACCESS|fresh|local=This machine at http://localhost:${CUBEBOT_PORT:-3000};domain=A public domain with automatic certificates|local|How will people reach the console?|This machine keeps the console on loopback (CUBEBOT_PORT picks another port). A public domain serves it on ports 80 and 443 with a certificate Caddy obtains, and agents reach Cubebot through it.
CUBEBOT_DOMAIN|fresh && [ "\${CUBEBOT_ACCESS:-}" = domain ]|text||Domain name|The name must already resolve to this machine, with ports 80 and 443 open to it. Whoever opens the console first becomes the administrator, unless you create the administrator now.
CUBEBOT_HOME|true|text|$HOME/.cubebot|Install directory|It holds the compose files, the cubebot command and .env with the database password.
CUBEBOT_ADMIN|fresh|now=Create it now in this terminal;console=At the console, the first account created|$(admin_default)|Administrator|The administrator manages people, agents and providers. Created now, nobody else can claim the console; at the console, the first person to open it becomes the administrator.
CUBEBOT_BOOTSTRAP_LOGIN|[ "\${CUBEBOT_ADMIN:-}" = now ]|text|admin|Administrator login|You sign in to the console with this login.
CUBEBOT_BOOTSTRAP_PASSWORD|[ "\${CUBEBOT_ADMIN:-}" = now ]|secret||Administrator password|Typed twice and never shown.
EOF
}

die() {
    echo "cubebot: $*" >&2
    exit 1
}

# t top, f front, s side. Strokes paint the front, then the side, then the top, each bottom-up, then eyes.
CUBE=(
    "    tttttttttt"
    "  ttttttttttss"
    "ffffffffffssss"
    "ffffffffffssss"
    "ffffffffffssss"
    "ffffffffffssss"
    "ffffffffffss  "
)
FRAMES=(◰ ◳ ◲ ◱)
fancy="" interactive="" tty_state="" work="" painted=0 drawn="" cols=80
orange="" sage="" dim="" bold="" reset=""
notes=()

setup_terminal() {
    cols=$(tput cols 2>/dev/null || echo 80)
    if [ -z "${NO_COLOR:-}" ] && [ "${TERM:-dumb}" != dumb ] && [ -t 1 ] && [ "$cols" -ge 60 ]; then
        fancy=1
    fi
    # Keys come from the terminal, since `curl … | bash` makes stdin the script.
    if [ "${CUBEBOT_YES:-}" != 1 ] && { exec 3</dev/tty; } 2>/dev/null; then
        interactive=1
        tty_state=$(stty -g <&3)
    fi
    if [ -n "$fancy" ]; then
        orange=$'\033[38;5;202m' sage=$'\033[38;5;108m' dim=$'\033[2m' bold=$'\033[1m' reset=$'\033[0m'
    fi
    trap cleanup EXIT
    trap 'exit 130' INT TERM
}

cleanup() {
    [ -z "$fancy" ] || printf '\033[0m\033[?25h'
    [ -z "$tty_state" ] || stty "$tty_state" <&3 2>/dev/null || true
    [ -z "$work" ] || rm -rf "$work"
}

draw_cube() {
    local r i cell stroke color line right
    [ -z "$drawn" ] || printf '\033[7A'
    drawn=1
    for r in 0 1 2 3 4 5 6; do
        line="  "
        for ((i = 0; i < 14; i++)); do
            cell=${CUBE[r]:i:1}
            case $cell in
                f) stroke=$((6 - r)) color=223 ;;
                s) stroke=$((11 - r)) color=180 ;;
                t) stroke=$((12 - r)) color=230 ;;
                *) line+=$'\033[49m ' && continue ;;
            esac
            if [ "$painted" -gt "$stroke" ]; then
                case $cell in
                    f) color=$((stroke % 2 ? 208 : 202)) ;;
                    s) color=166 ;;
                    t) color=208 ;;
                esac
            fi
            [ "$painted" -lt 13 ] || [ "$r" -ne 3 ] || { [ "$i" -ne 3 ] && [ "$i" -ne 6 ]; } || color=235
            line+=$'\033[48;5;'"${color}m "
        done
        right=""
        [ "$r" -ne 1 ] || right="$bold${orange}cubebot$reset"
        [ "$r" -lt 2 ] || [ -z "${notes[r - 2]:-}" ] || right="${sage}✓$reset ${notes[r - 2]}"
        printf '%s\033[0m    %s\033[K\n' "$line" "$right"
    done
}

note() {
    notes+=("$1")
    painted=$2
    [ -z "$fancy" ] || draw_cube
}

step() {
    local target=$1 pid
    shift
    if [ -z "$fancy" ]; then
        "$@" >"$work/out" 2>&1
        return
    fi
    "$@" >"$work/out" 2>&1 &
    pid=$!
    while kill -0 "$pid" 2>/dev/null; do
        if [ "$painted" -lt $((target - 1)) ]; then
            painted=$((painted + 1))
            draw_cube
        fi
        sleep 0.1
    done
    wait "$pid"
}

card() {
    local line
    echo
    for line in "$@"; do
        printf '  %s▌%s %s\n' "$orange" "$reset" "$line"
    done
    echo
}

paragraph() {
    fold -s -w $((cols - 6)) <<<"$1" | sed "s/^/    $dim/;s/\$/$reset/"
}

pick_move() {
    local i=$((selected + $1))
    while [ "$i" -ge 0 ] && [ "$i" -lt "$n" ]; do
        [ "${values[i]:0:1}" = "!" ] || {
            selected=$i
            return
        }
        i=$((i + $1))
    done
}

# Draws the picker; prints how many lines it took.
pick_draw() {
    local i mark
    printf '  %s?%s %s%s%s\n' "$orange" "$reset" "$bold" "$title" "$reset"
    for ((i = 0; i < n; i++)); do
        mark=" "
        [ "$i" -ne "$selected" ] || mark="${orange}❯$reset"
        if [ "${values[i]:0:1}" = "!" ]; then
            printf '    %s %s%d  %s, not on this machine%s\n' "$mark" "$dim" $((i + 1)) "${labels[i]}" "$reset"
        else
            printf '    %s %d  %s\n' "$mark" $((i + 1)) "${labels[i]}"
        fi
    done
    paragraph "$text"
    printf '    %s↑↓ or j k, 1-%d, Enter%s\n' "$dim" "$n" "$reset"
}

pick() {
    local var=$1 title=$2 default=$3 text=$4 item key rest lines n=0 selected=0
    local -a items values labels
    IFS=';' read -ra items <<<"$5"
    for item in "${items[@]}"; do
        values[n]=${item%%=*} labels[n]=${item#*=}
        [ "${values[n]}" != "$default" ] || selected=$n
        n=$((n + 1))
    done
    if [ -z "$fancy" ]; then
        pick_plain
        return
    fi
    lines=$(($(pick_draw | wc -l)))
    printf '\033[?25l'
    stty -echo -icanon <&3
    while :; do
        pick_draw
        IFS= read -rsn1 key <&3
        case $key in
            '') break ;;
            k) pick_move -1 ;;
            j) pick_move 1 ;;
            [1-9]) [ "$key" -gt "$n" ] || [ "${values[key - 1]:0:1}" = "!" ] || selected=$((key - 1)) ;;
            $'\033')
                IFS= read -rsn2 -t 1 rest <&3 || true
                case $rest in
                    '[A' | OA) pick_move -1 ;;
                    '[B' | OB) pick_move 1 ;;
                esac
                ;;
        esac
        printf '\033[%dA\033[J' "$lines"
    done
    stty "$tty_state" <&3
    printf '\033[%dA\033[J\033[?25h  %s✓%s %s  %s%s%s\n' "$lines" "$sage" "$reset" "$title" "$bold" "${labels[selected]}" "$reset"
    printf -v "$var" %s "${values[selected]}"
}

pick_plain() {
    local i answer
    printf '%s\n' "$title" "$text"
    for ((i = 0; i < n; i++)); do
        if [ "${values[i]:0:1}" = "!" ]; then
            printf '  %d) %s, not on this machine\n' $((i + 1)) "${labels[i]}"
        else
            printf '  %d) %s\n' $((i + 1)) "${labels[i]}"
        fi
    done
    while :; do
        printf 'Choose 1-%d [%d]: ' "$n" $((selected + 1))
        IFS= read -r answer <&3
        answer=${answer:-$((selected + 1))}
        case $answer in
            [1-9]) [ "$answer" -gt "$n" ] || [ "${values[answer - 1]:0:1}" = "!" ] || break ;;
        esac
    done
    printf -v "$var" %s "${values[answer - 1]}"
}

ask() {
    local var=$1 title=$2 default=$3 text=$4 secret=${5:-} answer again lines
    printf '  %s?%s %s%s%s\n' "$orange" "$reset" "$bold" "$title" "$reset"
    paragraph "$text"
    lines=$((1 + $(paragraph "$text" | wc -l)))
    while :; do
        # Before the prompt, so keys typed early do not show; bash 3.2's read -s alone does not hide them.
        [ -z "$secret" ] || stty -echo <&3
        printf '    %s›%s %s' "$orange" "$reset" "${default:+$dim($default)$reset }"
        if [ -n "$secret" ]; then
            IFS= read -rs answer <&3
            printf '\n    %s›%s again: ' "$orange" "$reset"
            IFS= read -rs again <&3
            stty "$tty_state" <&3
            echo
            lines=$((lines + 2))
            [ "$answer" = "$again" ] || {
                echo "    They differ; type it again."
                lines=$((lines + 1))
                continue
            }
        else
            IFS= read -r answer <&3
            lines=$((lines + 1))
        fi
        answer=${answer:-$default}
        [ -z "$answer" ] || break
    done
    answer=${answer/#\~/$HOME}
    printf -v "$var" %s "$answer"
    [ -z "$fancy" ] || {
        [ -z "$secret" ] || answer="••••••••"
        printf '\033[%dA\033[J  %s✓%s %s  %s%s%s\n' "$lines" "$sage" "$reset" "$title" "$bold" "$answer" "$reset"
    }
}

confirm() {
    local key
    [ -n "$interactive" ] || {
        [ "$2" = y ]
        return
    }
    stty -echo <&3
    printf '  %s?%s %s%s%s %s ' "$orange" "$reset" "$bold" "$1" "$reset" "$([ "$2" = y ] && echo "[Y/n]" || echo "[y/N]")"
    while :; do
        IFS= read -rsn1 key <&3
        case $key in
            y | Y | n | N) break ;;
            '') key=$2 && break ;;
        esac
    done
    stty "$tty_state" <&3
    case $key in
        y | Y) echo yes ;;
        *)
            echo no
            return 1
            ;;
    esac
}

# Spins beside `label` until job `pid` exits or $work/log has a `marker` line; non-zero, in `status`, if it failed.
spinner() {
    local label=$1 marker=${2:-} frame=0
    while [ -z "$status" ] && ! { [ -n "$marker" ] && grep -qs "^$marker" "$work/log"; }; do
        if ! kill -0 "$pid" 2>/dev/null; then
            wait "$pid" && status=0 || status=$?
            break
        fi
        [ -z "$fancy" ] || printf '\r  %s%s%s %s' "$orange" "${FRAMES[frame++ % 4]}" "$reset" "$label"
        sleep 0.1
    done
    if [ "${status:-0}" -ne 0 ]; then
        [ -z "$fancy" ] || printf '\r\033[K  %s✗%s %s\n' "$orange" "$reset" "$label"
        return 1
    fi
    if [ -n "$fancy" ]; then
        printf '\r\033[K  %s✓%s %s\n' "$sage" "$reset" "$label"
    else
        echo "$label: done."
    fi
}

fresh() {
    [ ! -f "${CUBEBOT_HOME:-$HOME/.cubebot}/.env" ]
}

admin_default() {
    if [ "${CUBEBOT_ACCESS:-}" = domain ] && { [ -n "$interactive" ] || [ -n "${CUBEBOT_BOOTSTRAP_PASSWORD:-}" ]; }; then
        echo now
    else
        echo console
    fi
}

valid() {
    local item
    local -a items
    case $2 in text | secret) return ;; esac
    IFS=';' read -ra items <<<"$2"
    for item in "${items[@]}"; do
        item=${item%%=*}
        [ "${item#!}" != "${!1}" ] || return 0
    done
    die "$1 is one of: $(sed 's/=[^;]*//g;s/[!]//g;s/;/ /g' <<<"$2")"
}

ask_all() {
    local count i var when choices default title text
    [ -z "${CUBEBOT_DOMAIN:-}" ] || CUBEBOT_ACCESS=${CUBEBOT_ACCESS:-domain}
    [ -z "${CUBEBOT_PORT:-}" ] || CUBEBOT_ACCESS=${CUBEBOT_ACCESS:-local}
    # CUBEBOT_HOME installs another installation; pick its CUBEBOT_PORT. Without a domain, agents need 8443.
    fresh || CUBEBOT_HOME=${CUBEBOT_HOME:-$HOME/.cubebot}
    count=$(questions | wc -l)
    for ((i = 1; i <= count; i++)); do
        IFS='|' read -r var when choices default title text <<<"$(questions | sed -n "${i}p")"
        eval "$when" || continue
        if [ -n "${!var:-}" ]; then
            valid "$var" "$choices"
            continue
        fi
        if [ -z "$interactive" ]; then
            printf -v "$var" %s "$default"
        elif [ "$choices" = text ] || [ "$choices" = secret ]; then
            ask "$var" "$title" "$default" "$text" "${choices#text}"
        else
            pick "$var" "$title" "$default" "$text" "$choices"
        fi
        [ -n "${!var}" ] || die "set $var"
    done
}

checks() {
    local version major minor
    command -v curl >/dev/null || die "curl is required"
    command -v docker >/dev/null || die "Docker is required: https://docs.docker.com/engine/install/"
    note "curl and Docker" 1
    step 2 docker info || die "cannot reach the Docker daemon; is it running, and may this user use it?"
    note "Docker daemon" 2
    # Compose files carry inline configs, which need Compose 2.23.
    version=$(docker compose version --short 2>/dev/null) || die "Docker Compose v2 is required"
    major=${version%%.*}
    minor=${version#*.}
    minor=${minor%%.*}
    [ "$major" -gt 2 ] || { [ "$major" -eq 2 ] && [ "$minor" -ge 23 ]; } || die "Docker Compose 2.23 or later is required; this is $version"
    note "Compose $version" 3

    system=$(uname -s)
    case "$system" in
        Linux | Darwin) ;;
        *) die "Cubebot installs on Linux or macOS; this is $system" ;;
    esac
    if [ "$system/$(uname -m)" = Linux/x86_64 ] && [ -c /dev/kvm ] && [ -c /dev/net/tun ]; then
        kvm_mark="" kvm_here="this machine has them" runtime_default=kvm
    else
        kvm_mark="!" kvm_here="this $system $(uname -m) machine does not" runtime_default=container
    fi
    note "$system $(uname -m)" 4

    release=${CUBEBOT_VERSION:-}
    if [ -z "$release" ]; then
        step 12 curl -fsSIo /dev/null -w '%{redirect_url}' https://github.com/vicotrbb/cubebot/releases/latest || die "could not find the latest release"
        release=$(cat "$work/out")
        release=${release##*/}
        # Without a release, GitHub redirects to the releases page.
        case "$release" in "" | releases) die "could not find the latest release" ;; esac
    fi
    note "release $release" 13
}

setting() {
    if [ -f "$home/.env" ]; then
        sed -n "s/^$1=//p" "$home/.env"
    else
        echo "${!1:-}"
    fi
}

console_url() {
    local domain port
    domain=$(setting CUBEBOT_DOMAIN) port=$(setting CUBEBOT_PORT)
    if [ -n "$domain" ]; then echo "https://$domain"; else echo "http://localhost:${port:-3000}"; fi
}

summary() {
    local admin="the first account created at the console" kept=""
    [ "${CUBEBOT_ADMIN:-}" != now ] || admin="created now as ${CUBEBOT_BOOTSTRAP_LOGIN:-admin}"
    fresh || kept=" (keeps its .env and data)" admin="unchanged"
    local network=""
    [ "$(setting CUBEBOT_RUNTIME)" = off ] || network="Agent network  no firewall: agents can reach this machine's private networks and cloud metadata; only the Helm chart blocks them"
    card "${bold}Cubebot $release$reset" \
        "Agents         $(setting CUBEBOT_RUNTIME)" \
        ${network:+"$network"} \
        "Console        $(console_url)" \
        "Directory      ${home/#$HOME/\~}$kept" \
        "Administrator  $admin"
}

start() {
    local source=${CUBEBOT_SOURCE:-https://raw.githubusercontent.com/vicotrbb/cubebot/$release} name
    umask 077
    mkdir -p "$home"
    status=""
    {
        curl -fsSL "$source/deploy/cubebot" -o "$home/cubebot.new" || die "could not download $source/deploy/cubebot"
        chmod 755 "$home/cubebot.new"
        mv "$home/cubebot.new" "$home/cubebot"
        # An existing .env holds the database password of existing data: keep it.
        if [ -n "$fresh_install" ]; then
            {
                echo "POSTGRES_PASSWORD=$(od -An -N16 -tx1 /dev/urandom | tr -d ' \n')"
                echo "CUBEBOT_RUNTIME=$CUBEBOT_RUNTIME"
                # Compose names a project's volumes after it, so another directory needs its own.
                [ "$home" = "$HOME/.cubebot" ] || echo "COMPOSE_PROJECT_NAME=cubebot-$(printf %s "$home" | cksum | cut -d' ' -f1)"
                for name in CUBEBOT_DOMAIN CUBEBOT_PORT CUBEBOT_REGISTRY CUBEBOT_SOURCE; do
                    [ -z "${!name:-}" ] || echo "$name=${!name}"
                done
            } >"$home/.env"
        fi
        CUBEBOT_HOME="$home" "$home/cubebot" update "$release"
    } >"$work/log" 2>&1 </dev/null &
    pid=$!
    if ! { spinner "Writing files" "Pulling release" && spinner "Pulling images" "Starting Cubebot" &&
        spinner "Starting containers" "Waiting for" && spinner "Waiting for the console"; }; then
        failed
    fi
    [ -z "$fresh_install" ] || [ -n "$fancy" ] || echo "Wrote $home/.env (agents: $CUBEBOT_RUNTIME)."
    [ "${CUBEBOT_ADMIN:-}" = now ] || return 0
    status=""
    CUBEBOT_BOOTSTRAP_PASSWORD="$CUBEBOT_BOOTSTRAP_PASSWORD" "$home/cubebot" admin "$CUBEBOT_BOOTSTRAP_LOGIN" >"$work/log" 2>&1 </dev/null &
    pid=$!
    spinner "Creating the administrator" || failed
}

failed() {
    sed 's/^/    /' "$work/log" >&2
    die "the installation stopped; its output is above"
}

finish() {
    local url runtime command=${home/#$HOME/\~}/cubebot
    url=$(console_url) runtime=$(setting CUBEBOT_RUNTIME)
    if [ -z "$fancy" ]; then
        if [ -n "$fresh_install" ] && [ "${CUBEBOT_ADMIN:-}" = now ]; then
            echo "Open $url and sign in as $CUBEBOT_BOOTSTRAP_LOGIN."
        elif [ -n "$fresh_install" ]; then
            echo "Open $url: the first account created there becomes the administrator. To claim it from this terminal instead, which matters when CUBEBOT_DOMAIN is public, run $command admin."
        fi
        [ "$runtime" = off ] || echo "Then add a provider key: the gear beside your name, then AI providers."
        echo "Manage this installation with $command up | down | update | logs | admin | uninstall."
        return
    fi
    local admin="" next=""
    if [ -n "$fresh_install" ] && [ "${CUBEBOT_ADMIN:-}" = now ]; then
        admin="Sign in as $CUBEBOT_BOOTSTRAP_LOGIN."
    elif [ -n "$fresh_install" ]; then
        admin="The first account created there becomes the administrator; $command admin claims it here."
    fi
    [ "$runtime" = off ] || next="${bold}Next$reset           add a provider key: the gear beside your name, then AI providers"
    card "${bold}Cubebot is running$reset" \
        "Console        $orange$url$reset" \
        "Agents         $runtime" \
        ${admin:+"$admin"} \
        ${next:+"$next"} \
        "" \
        "$command up          start it" \
        "$command down        stop it, keeping data" \
        "$command update      move to the latest release" \
        "$command logs        show the services' logs" \
        "$command admin       create the administrator" \
        "$command uninstall   remove it, its agents and all data"
}

# Wrapped so a download cut short runs nothing.
main() {
    setup_terminal
    work=$(mktemp -d)
    if [ -n "$fancy" ]; then
        echo
        draw_cube
    else
        echo "Cubebot installer"
    fi
    checks
    echo
    ask_all
    home=$CUBEBOT_HOME fresh_install=""
    fresh && fresh_install=1
    summary
    confirm "Install?" y || die "nothing installed"
    start
    finish
}

if [ "${BASH_SOURCE[0]:-$0}" = "$0" ]; then
    main "$@"
fi
