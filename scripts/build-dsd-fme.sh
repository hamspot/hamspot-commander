#!/bin/bash
# Build lwvmobile/dsd-fme and install the binary under ~/.local/bin.
#
# mbelib (branch ambe_tones) is installed under the same prefix so the
# decoder can synthesize voice. Its shared library is found through the
# binary's RPATH, not ldconfig. Sources stay in the XDG cache.
#
# Read https://github.com/lwvmobile/mbelib before you use the voice library.
# This script does not install GNU Radio or the Ubuntu dsdcc package.

set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
home_dir="${HOME:?HOME is not set}"
prefix="${home_dir}/.local"
cache="${XDG_CACHE_HOME:-${home_dir}/.cache}/hamspot-commander"
state_dir="${home_dir}/.local/share/hamspot-commander"
file_list="${state_dir}/dsd-fme-files.txt"
rev_file="${state_dir}/dsd-fme-rev"

mbe_url="https://github.com/lwvmobile/mbelib"
mbe_branch="ambe_tones"
dsd_url="https://github.com/lwvmobile/dsd-fme"
dsd_branch="audio_work"

die() {
    echo "build-dsd-fme: $*" >&2
    exit 1
}

require_tools() {
    local missing=()
    local cmd
    for cmd in git cmake gcc pkg-config; do
        command -v "$cmd" >/dev/null 2>&1 || missing+=("$cmd")
    done
    if [[ ${#missing[@]} -gt 0 ]]; then
        die "missing ${missing[*]}. Run scripts/install.sh on Ubuntu 26.04."
    fi
}

update_repo() {
    local url="$1"
    local branch="$2"
    local dir="$3"
    mkdir -p "$(dirname "$dir")"
    if [[ ! -d "${dir}/.git" ]]; then
        git clone --branch "$branch" --single-branch "$url" "$dir"
    elif ! git -C "$dir" fetch --depth 1 origin "$branch"; then
        echo "build-dsd-fme: could not fetch ${url} ${branch}; using the clone already on disk." >&2
    else
        git -C "$dir" checkout -q "$branch"
        # reset prints the new HEAD on stdout. The caller captures only the rev.
        git -C "$dir" reset --hard FETCH_HEAD >/dev/null
    fi
    git -C "$dir" rev-parse HEAD
}

build_tree() {
    local src="$1"
    local build="$2"
    shift 2
    cmake -S "$src" -B "$build" \
        -DCMAKE_BUILD_TYPE=Release \
        -DCMAKE_INSTALL_PREFIX="$prefix" \
        -DCMAKE_PREFIX_PATH="$prefix" \
        -DCMAKE_INSTALL_RPATH="${prefix}/lib" \
        -DCMAKE_BUILD_WITH_INSTALL_RPATH=ON \
        "$@"
    cmake --build "$build" --parallel "$(nproc)"
    cmake --install "$build"
}

require_tools
install -d -m 755 "${prefix}/bin" "${prefix}/lib" "$state_dir" "$cache"

mbe_src="${cache}/mbelib"
dsd_src="${cache}/dsd-fme"
mbe_rev="$(update_repo "$mbe_url" "$mbe_branch" "$mbe_src")"
dsd_rev="$(update_repo "$dsd_url" "$dsd_branch" "$dsd_src")"
stamp="${mbe_rev} ${dsd_rev}"

if [[ -x "${prefix}/bin/dsd-fme" && -f "$rev_file" && "$(cat "$rev_file")" == "$stamp" ]]; then
    echo "dsd-fme ${dsd_rev} is already installed at ${prefix}/bin/dsd-fme"
    exit 0
fi

build_tree "$mbe_src" "${cache}/mbelib-build"
build_tree "$dsd_src" "${cache}/dsd-fme-build"

[[ -x "${prefix}/bin/dsd-fme" ]] || die "install did not produce ${prefix}/bin/dsd-fme"

# cmake's install_manifest.txt may omit the final newline. A separator
# keeps the next path from being glued onto that line.
{
    for manifest in \
        "${cache}/mbelib-build/install_manifest.txt" \
        "${cache}/dsd-fme-build/install_manifest.txt"
    do
        [[ -f "$manifest" ]] || die "missing ${manifest}"
        cat "$manifest"
        printf '\n'
    done
} | awk 'NF && !seen[$0]++' >"${file_list}.tmp"
mv "${file_list}.tmp" "$file_list"
printf '%s\n' "$stamp" >"$rev_file"

echo "Installed ${prefix}/bin/dsd-fme"
echo "mbelib ${mbe_rev}"
echo "dsd-fme ${dsd_rev}"
