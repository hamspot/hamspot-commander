#!/bin/bash
# Install Hamspot Commander for Ubuntu 26.04.
#
# Apt packages, a release binary under ~/.local/bin, the desktop entry, and
# its icon. Serial ports stay on the distro dialout udev rule. This script
# adds the user to dialout or plugdev only when they are not already a member.
# It does not start Commander or Gqrx.

set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
user_name="${USER:-$(id -un)}"
home_dir="${HOME:?HOME is not set}"
bin_path="${home_dir}/.local/bin/commander"
desktop_path="${home_dir}/.local/share/applications/hamspotcommander.desktop"
icon_path="${home_dir}/.local/share/icons/hicolor/128x128/apps/hamspot-commander.png"
state_dir="${home_dir}/.local/share/hamspot-commander"
manifest="${state_dir}/installed.manifest"
icon_src="${root}/crates/commander/assets/hamspot-commander.png"
marker="# hamspot-commander-install"

packages=(
    gqrx-sdr
    libhamlib-utils
    pipewire-bin
    helvum
    rustc
    cargo
    pkg-config
    libgtk-3-dev
    # dsd-fme and mbelib. Not GNU Radio, and not the Ubuntu dsdcc package.
    build-essential
    cmake
    git
    libcodec2-dev
    libncurses-dev
    libpulse-dev
    librtlsdr-dev
    libsndfile1-dev
    libusb-1.0-0-dev
    socat
    sox
)

die() {
    echo "install: $*" >&2
    exit 1
}

require_ubuntu_2604() {
    # shellcheck disable=SC1091
    [[ -r /etc/os-release ]] || die "cannot read /etc/os-release"
    # shellcheck source=/dev/null
    . /etc/os-release
    if [[ "${ID:-}" != "ubuntu" || "${VERSION_ID:-}" != "26.04" ]]; then
        die "targets Ubuntu 26.04. This machine is ${PRETTY_NAME:-unknown}."
    fi
}

in_group() {
    local group="$1"
    local members
    members="$(getent group "$group" | awk -F: '{print $4}')"
    [[ ",${members}," == *",${user_name},"* ]]
}

manifest_flag() {
    local key="$1"
    [[ -f "$manifest" ]] || return 1
    grep -qx "${key}=yes" "$manifest"
}

refresh_caches() {
    # The desktop Icon= line is the PNG path, so a cache miss does not hide it.
    if command -v update-desktop-database >/dev/null 2>&1; then
        update-desktop-database "${home_dir}/.local/share/applications"
    fi
    if command -v gtk-update-icon-cache >/dev/null 2>&1; then
        if ! gtk-update-icon-cache -f -t "${home_dir}/.local/share/icons/hicolor"; then
            echo "install: icon cache was not updated. The desktop file points at the PNG." >&2
        fi
    fi
}

# Copy repo modems/ into the install directory and link each script onto PATH.
install_modems() {
    local src_dir="${root}/modems"
    local dest_dir="${state_dir}/modems"
    [[ -d "$src_dir" ]] || die "missing ${src_dir}"
    install -d -m 755 "$dest_dir" "${home_dir}/.local/bin"
    local src name
    for src in "$src_dir"/*; do
        [[ -f "$src" ]] || continue
        name="$(basename "$src")"
        install -m 755 "$src" "${dest_dir}/${name}"
        ln -sfn "${dest_dir}/${name}" "${home_dir}/.local/bin/${name}"
    done
}

write_desktop() {
    cat >"$desktop_path" <<EOF
${marker}
[Desktop Entry]
Version=1.0
Type=Application
Name=Hamspot Commander
Comment=Hamlib rig face
Exec=${bin_path}
Path=${root}
Icon=${icon_path}
StartupWMClass=hamspotcommander
Terminal=false
Categories=HamRadio;Network;
StartupNotify=true
Actions=new-window;

[Desktop Action new-window]
Name=New Window
Exec=${bin_path}
EOF
}

write_manifest() {
    local added_dialout="$1"
    local added_plugdev="$2"
    mkdir -p "$state_dir"
    cat >"$manifest" <<EOF
# hamspot-commander install manifest
ubuntu=26.04
binary=${bin_path}
desktop=${desktop_path}
icon=${icon_path}
added_dialout=${added_dialout}
added_plugdev=${added_plugdev}
dsd_fme_files=${state_dir}/dsd-fme-files.txt
modems_dir=${state_dir}/modems
EOF
}

require_ubuntu_2604

[[ -f "$icon_src" ]] || die "missing icon ${icon_src}"
command -v sudo >/dev/null 2>&1 || die "sudo is required to install apt packages"
# -n fails closed when sudo would ask for a password and no terminal is attached.
sudo -n true || die "sudo needs a password. Run this script in a terminal."

sudo DEBIAN_FRONTEND=noninteractive apt-get update
sudo DEBIAN_FRONTEND=noninteractive apt-get install -y "${packages[@]}"

command -v cargo >/dev/null 2>&1 || die "cargo is not on PATH after apt install"
command -v rustc >/dev/null 2>&1 || die "rustc is not on PATH after apt install"

(
    cd "$root"
    cargo build --release -p commander
)
[[ -f "${root}/target/release/commander" ]] || die "release binary was not produced"

install -d -m 755 "${home_dir}/.local/bin"
install -m 755 "${root}/target/release/commander" "$bin_path"
"${root}/scripts/build-dsd-fme.sh"
install_modems
install -d -m 755 "$(dirname "$icon_path")"
install -m 644 "$icon_src" "$icon_path"
write_desktop

added_dialout=no
added_plugdev=no
if manifest_flag added_dialout; then
    added_dialout=yes
elif ! in_group dialout; then
    sudo usermod -aG dialout "$user_name"
    added_dialout=yes
fi
if manifest_flag added_plugdev; then
    added_plugdev=yes
elif ! in_group plugdev; then
    sudo usermod -aG plugdev "$user_name"
    added_plugdev=yes
fi

if ! write_manifest "$added_dialout" "$added_plugdev"; then
    if [[ "$added_dialout" == yes ]] && ! manifest_flag added_dialout; then
        sudo gpasswd -d "$user_name" dialout || true
    fi
    if [[ "$added_plugdev" == yes ]] && ! manifest_flag added_plugdev; then
        sudo gpasswd -d "$user_name" plugdev || true
    fi
    die "could not write ${manifest}"
fi

refresh_caches

echo "Installed ${bin_path}"
echo "Desktop entry ${desktop_path}"
if [[ "$added_dialout" == yes || "$added_plugdev" == yes ]]; then
    echo "Log in again so dialout and plugdev membership applies to this seat."
fi
