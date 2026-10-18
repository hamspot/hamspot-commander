#!/bin/bash
# Install p25voice and the Commander P25 TX pipe under the home directory.
#
# scripts/install.sh and the Commander build call this only after the machine
# opts in. Running this file by itself always installs. It does not install
# GNU Radio or the Ubuntu dsdcc package, and it does not replace commander.
#
# Uninstall removes the paths in p25voice-files.txt only when they stay under
# $HOME, so leave PREFIX and MODEMS there.

set -euo pipefail

home_dir="${HOME:?HOME is not set}"
src="${P25VOICE_SRC:-${home_dir}/Documents/p25voice}"
prefix="${PREFIX:-${home_dir}/.local}"
modems="${MODEMS:-${home_dir}/.local/share/hamspot-commander/modems}"
state_dir="${home_dir}/.local/share/hamspot-commander"
list_path="${state_dir}/p25voice-files.txt"
manifest="${state_dir}/installed.manifest"

die() {
    echo "install-p25voice: $*" >&2
    exit 1
}

[[ "$prefix" == "${home_dir}/"* ]] || die "PREFIX must stay under ${home_dir}: ${prefix}"
[[ "$modems" == "${home_dir}/"* ]] || die "MODEMS must stay under ${home_dir}: ${modems}"
[[ -d "$src" ]] || die "missing source ${src} (set P25VOICE_SRC)"
[[ -f "${src}/Makefile" ]] || die "missing Makefile in ${src}"
[[ -f "${src}/examples/commander-mod-p25.txpipe" ]] || die "missing TX pipe example in ${src}"
command -v make >/dev/null 2>&1 || die "make is not on PATH"
command -v g++ >/dev/null 2>&1 || die "g++ is not on PATH"

make -C "$src" install PREFIX="$prefix" MODEMS="$modems"

[[ -x "${prefix}/bin/p25voice" ]] || die "p25voice was not installed at ${prefix}/bin/p25voice"
[[ -f "${modems}/commander-mod-p25.txpipe" ]] || die "TX pipe was not installed at ${modems}/commander-mod-p25.txpipe"
[[ -L "${prefix}/bin/commander-mod-p25.txpipe" ]] || die "TX pipe was not linked at ${prefix}/bin/commander-mod-p25.txpipe"

install -d -m 755 "$state_dir"
cat >"$list_path" <<EOF
${prefix}/bin/p25voice
${modems}/commander-mod-p25.txpipe
${prefix}/bin/commander-mod-p25.txpipe
EOF

if [[ -f "$manifest" ]]; then
    tmp="$(mktemp "${state_dir}/installed.manifest.XXXXXX")"
    awk -v value="$list_path" '
        BEGIN { done = 0 }
        /^p25voice_files=/ {
            print "p25voice_files=" value
            done = 1
            next
        }
        { print }
        END {
            if (!done) {
                print "p25voice_files=" value
            }
        }
    ' "$manifest" >"$tmp"
    mv "$tmp" "$manifest"
fi

echo "install-p25voice: ${prefix}/bin/p25voice" >&2
echo "install-p25voice: ${modems}/commander-mod-p25.txpipe" >&2
