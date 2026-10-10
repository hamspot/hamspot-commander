#!/bin/bash
# Start Hamlib rigctld on the dummy radio (model 1) and park one amateur
# band on each VFO. The dummy's single RX/TX range is 150 kHz–1.5 GHz, so
# these HF, VHF, and UHF frequencies are all inside it.
#
#   VFO A   14.153.608 MHz USB   20 m
#   VFO B  146.520     MHz FM    2 m
#   VFO C  446.000     MHz FM    70 cm
#
# Ctrl-C stops rigctld. PORT and ADDR override the listen socket.

set -euo pipefail

port="${PORT:-4532}"
addr="${ADDR:-127.0.0.1}"
target="${addr}:${port}"

if ! command -v rigctld >/dev/null 2>&1 || ! command -v rigctl >/dev/null 2>&1; then
    echo "rigctld and rigctl are required (package libhamlib-utils)." >&2
    exit 1
fi

if ss -ltn "sport = :${port}" | grep -q LISTEN; then
    echo "Something is already listening on port ${port}." >&2
    exit 1
fi

rigctld -m 1 -T "$addr" -t "$port" &
pid=$!

cleanup() {
    if kill -0 "$pid" 2>/dev/null; then
        kill "$pid" 2>/dev/null || true
        wait "$pid" 2>/dev/null || true
    fi
}
trap cleanup INT TERM EXIT

# rigctl answering is the ready signal. A refused connect returns at once,
# so the pause is only the gap between those tries.
ready=0
for _ in $(seq 1 50); do
    if ! kill -0 "$pid" 2>/dev/null; then
        echo "rigctld exited before it accepted a connection." >&2
        exit 1
    fi
    if rigctl -m 2 -r "$target" f >/dev/null 2>&1; then
        ready=1
        break
    fi
    sleep 0.05
done
if [ "$ready" -ne 1 ]; then
    echo "rigctld on ${target} did not answer." >&2
    exit 1
fi

# One rigctl per command. A multi-line stdin script stays on the last VFO
# and makes a later failure harder to see.
rigctl -m 2 -r "$target" V VFOA
rigctl -m 2 -r "$target" F 14153608
rigctl -m 2 -r "$target" M USB 2400
rigctl -m 2 -r "$target" V VFOB
rigctl -m 2 -r "$target" F 146520000
rigctl -m 2 -r "$target" M FM 15000
rigctl -m 2 -r "$target" V VFOC
rigctl -m 2 -r "$target" F 446000000
rigctl -m 2 -r "$target" M FM 15000
rigctl -m 2 -r "$target" V VFOA

show() {
    local vfo="$1"
    local label="$2"
    rigctl -m 2 -r "$target" V "$vfo" >/dev/null
    local freq mode
    freq="$(rigctl -m 2 -r "$target" f)"
    mode="$(rigctl -m 2 -r "$target" m | head -n 1)"
    printf '  %-5s %12s Hz  %s   %s\n' "$vfo" "$freq" "$mode" "$label"
}

echo "Hamlib dummy listening on ${target} (model 1, 150 kHz–1.5 GHz)."
show VFOA "HF  20 m"
show VFOB "VHF  2 m"
show VFOC "UHF 70 cm"
rigctl -m 2 -r "$target" V VFOA >/dev/null
echo "Commander connects to ${target}. Ctrl-C stops this radio."

wait "$pid"
