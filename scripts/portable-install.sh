#!/bin/bash
# Build a portable Hamspot Commander tree for an Ubuntu live image.
#
#   ~/Applications/HamSpotCommander/
#     hamspot-commander.sh
#     hamspotcommander.desktop
#     bin/commander
#     bsp/
#     config/   XDG_CONFIG_HOME (station files and gqrx INI)
#     data/     XDG_DATA_HOME (eframe state)
#     cache/
#     state/
#     share/    desktop file and icon for the app id
#
# The launch script finds this directory from its own path, so the folder
# can move with casper. Apt packages are not installed or removed. rigctld,
# gqrx, and PipeWire must already be on the image.

set -euo pipefail

die() {
    echo "portable-install: $*" >&2
    exit 1
}

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
icon_src="${root}/crates/commander/assets/hamspot-commander.png"
stage=""
no_build=0

while [[ $# -gt 0 ]]; do
    case "$1" in
        --stage)
            [[ $# -ge 2 ]] || die "--stage needs a directory"
            stage="$2"
            shift 2
            ;;
        --no-build)
            no_build=1
            shift
            ;;
        *)
            die "unknown argument: $1"
            ;;
    esac
done

if [[ -n "$stage" ]]; then
    prefix="$stage"
else
    home_dir="${HOME:?HOME is not set}"
    prefix="${home_dir}/Applications/HamSpotCommander"
    apps_desktop="${home_dir}/Applications/HamSpotCommander.desktop"
fi

write_desktop() {
    local dest="$1"
    local relative_launcher="$2"
    local icon="$3"
    local workdir="${4:-}"
    local path_line=""
    if [[ -n "$workdir" ]]; then
        path_line="Path=${workdir}"
    fi
    cat >"$dest" <<EOF
[Desktop Entry]
Version=1.0
Type=Application
Name=Hamspot Commander
Comment=Portable hamlib rig face
Exec=sh -c 'exec "\$(dirname "\$(readlink -f "\$1")")/${relative_launcher}" \${2:+"\$2"}' sh %k %f
${path_line}
Icon=${icon}
StartupWMClass=hamspotcommander
Terminal=false
Categories=HamRadio;Network;
MimeType=application/x-hamspot-commander;
StartupNotify=true
Actions=new-window;

[Desktop Action new-window]
Name=New Window
Exec=sh -c 'exec "\$(dirname "\$(readlink -f "\$1")")/${relative_launcher}"' sh %k
EOF
    # Drop the blank line left when Path is omitted so the file stays valid.
    if [[ -z "$path_line" ]]; then
        local tmp
        tmp="$(mktemp)"
        grep -v '^$' "$dest" >"$tmp" || true
        # The entry still needs a blank line before the action group.
        awk 'BEGIN{blank=0} /^\[Desktop Action/{print ""; blank=1} {print}' "$tmp" >"$dest"
        rm -f "$tmp"
    fi
}

[[ -f "$icon_src" ]] || die "missing icon ${icon_src}"
[[ -d "${root}/bsp" ]] || die "missing ${root}/bsp"
if [[ "$no_build" -eq 0 ]]; then
    command -v cargo >/dev/null 2>&1 || die "cargo is not on PATH"
    (
        cd "$root"
        cargo build --release -p commander
    )
fi
[[ -x "${root}/target/release/commander" ]] || die "release binary was not produced"

install -d -m 755 \
    "${prefix}/bin" \
    "${prefix}/bsp" \
    "${prefix}/config" \
    "${prefix}/data" \
    "${prefix}/cache" \
    "${prefix}/state" \
    "${prefix}/share/applications" \
    "${prefix}/share/icons/hicolor/128x128/apps"

install -m 755 "${root}/target/release/commander" "${prefix}/bin/commander"
install -m 755 "${root}/scripts/hamspot-commander.sh" "${prefix}/hamspot-commander.sh"
install -m 644 "$icon_src" "${prefix}/hamspot-commander.png"
install -m 644 "$icon_src" "${prefix}/share/icons/hicolor/128x128/apps/hamspot-commander.png"
install -d -m 755 "${prefix}/share/mime/packages"
install -m 644 "${root}/share/mime/packages/hamspot-commander.xml" \
    "${prefix}/share/mime/packages/hamspot-commander.xml"
if command -v update-mime-database >/dev/null 2>&1; then
    update-mime-database "${prefix}/share/mime" || true
fi
cp -a "${root}/bsp/." "${prefix}/bsp/"

# Keep the directories when the tree is packed into a squashfs.
for dir in config data cache state; do
    [[ -e "${prefix}/${dir}/.keep" ]] || : >"${prefix}/${dir}/.keep"
done

if [[ -n "$stage" ]]; then
    # Paths stay relative to each desktop file so the zip unpacks anywhere.
    write_desktop "${prefix}/hamspotcommander.desktop" "hamspot-commander.sh" "./hamspot-commander.png"
    write_desktop "${prefix}/share/applications/hamspotcommander.desktop" "../../hamspot-commander.sh" "../../hamspot-commander.png"
else
    write_desktop "${prefix}/hamspotcommander.desktop" "hamspot-commander.sh" "${prefix}/hamspot-commander.png" "$prefix"
    # This copy lives in share/applications, two levels down from the prefix.
    write_desktop "${prefix}/share/applications/hamspotcommander.desktop" "../../hamspot-commander.sh" "${prefix}/hamspot-commander.png" "$prefix"
    write_desktop "$apps_desktop" "HamSpotCommander/hamspot-commander.sh" "${prefix}/hamspot-commander.png" "$prefix"
fi

echo "Portable tree ${prefix}"
echo "Launch ${prefix}/hamspot-commander.sh"
