#!/bin/bash
# Undo scripts/install.sh. Apt packages stay installed. The git checkout
# and target/ are left in place. Group membership is removed only when the
# install manifest says this install added it.

set -euo pipefail

user_name="${USER:-$(id -un)}"
home_dir="${HOME:?HOME is not set}"
state_dir="${home_dir}/.local/share/hamspot-commander"
manifest="${state_dir}/installed.manifest"
marker="# hamspot-commander-install"
default_bin="${home_dir}/.local/bin/commander"
default_desktop="${home_dir}/.local/share/applications/hamspotcommander.desktop"
default_icon="${home_dir}/.local/share/icons/hicolor/128x128/apps/hamspot-commander.png"

die() {
    echo "uninstall: $*" >&2
    exit 1
}

under_home() {
    local path="$1"
    [[ "$path" == "${home_dir}/"* ]] || die "refusing path outside ${home_dir}: ${path}"
}

remove_path() {
    local path="$1"
    under_home "$path"
    if [[ -e "$path" || -L "$path" ]]; then
        rm -f "$path"
        echo "Removed ${path}"
    fi
}

refresh_caches() {
    if command -v update-desktop-database >/dev/null 2>&1; then
        update-desktop-database "${home_dir}/.local/share/applications" || true
    fi
    if command -v gtk-update-icon-cache >/dev/null 2>&1; then
        gtk-update-icon-cache -f -t "${home_dir}/.local/share/icons/hicolor" || true
    fi
}

drop_group() {
    local group="$1"
    local members
    members="$(getent group "$group" | awk -F: '{print $4}')"
    if [[ ",${members}," == *",${user_name},"* ]]; then
        sudo gpasswd -d "$user_name" "$group"
        echo "Removed ${user_name} from ${group}. Log in again to drop it from this seat."
    fi
}

if [[ ! -f "$manifest" ]]; then
    if [[ -f "$default_desktop" ]] && grep -qx "$marker" "$default_desktop"; then
        remove_path "$default_bin"
        remove_path "$default_desktop"
        remove_path "$default_icon"
        refresh_caches
        echo "No manifest, so group membership was left as it is."
        exit 0
    fi
    echo "Hamspot Commander install manifest was not found."
    exit 0
fi

binary="$(grep -E '^binary=' "$manifest" | head -n 1 | cut -d= -f2-)"
desktop="$(grep -E '^desktop=' "$manifest" | head -n 1 | cut -d= -f2-)"
icon="$(grep -E '^icon=' "$manifest" | head -n 1 | cut -d= -f2-)"
added_dialout="$(grep -E '^added_dialout=' "$manifest" | head -n 1 | cut -d= -f2-)"
added_plugdev="$(grep -E '^added_plugdev=' "$manifest" | head -n 1 | cut -d= -f2-)"

[[ -n "$binary" && -n "$desktop" && -n "$icon" ]] || die "manifest ${manifest} is missing a path"

remove_path "$binary"
remove_path "$desktop"
remove_path "$icon"

if [[ "$added_dialout" == yes || "$added_plugdev" == yes ]]; then
    command -v sudo >/dev/null 2>&1 || die "sudo is required to undo group membership"
    sudo -n true || die "sudo needs a password. Run this script in a terminal."
fi
if [[ "$added_dialout" == yes ]]; then
    drop_group dialout
fi
if [[ "$added_plugdev" == yes ]]; then
    drop_group plugdev
fi

rm -f "$manifest"
rmdir "$state_dir" 2>/dev/null || true
refresh_caches
echo "Uninstalled. Apt packages were left installed."
