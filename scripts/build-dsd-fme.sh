#!/bin/bash
# Build dsd-fme and install the binary under ~/.local/bin.
#
# With no arguments this builds the webaugur no-halfrate pair. That mbelib
# omits the 3600x2450 half-rate decoder, and dsd-fme fills those frames with
# silence. Phase 1 IMBE, D-STAR 3600x2400, and the Phase 2 air interface stay.
#
# --upstream builds unmodified lwvmobile mbelib and dsd-fme (audio_work).
# scripts/install.sh passes --upstream.
# Both libraries always come from the same side. Upstream
# dsd-fme will not link a mbelib that is missing the half-rate symbols.
#
# Read the mbelib README before you use either voice library. The shared
# library is found through the binary's RPATH, not ldconfig. Each variant
# keeps its own cache clone, so a hard reset cannot reuse the other tree.
# This script does not install GNU Radio or the Ubuntu dsdcc package.

set -euo pipefail

home_dir="${HOME:?HOME is not set}"
prefix="${home_dir}/.local"
cache="${XDG_CACHE_HOME:-${home_dir}/.cache}/hamspot-commander"
state_dir="${home_dir}/.local/share/hamspot-commander"
file_list="${state_dir}/dsd-fme-files.txt"
rev_file="${state_dir}/dsd-fme-rev"

die() {
    echo "build-dsd-fme: $*" >&2
    exit 1
}

variant="fork"
if [[ $# -gt 1 ]]; then
    die "expected no arguments or --upstream"
fi
if [[ $# -eq 1 ]]; then
    case "$1" in
        --upstream) variant="upstream" ;;
        *) die "unknown argument '$1' (expected no arguments or --upstream)" ;;
    esac
fi

if [[ "$variant" == "fork" ]]; then
    mbe_url="https://github.com/webaugur/mbelib"
    mbe_branch="no-halfrate"
    dsd_url="https://github.com/webaugur/dsd-fme"
    dsd_branch="no-halfrate"
    src_tag="no-halfrate"
else
    mbe_url="https://github.com/lwvmobile/mbelib"
    mbe_branch="ambe_tones"
    dsd_url="https://github.com/lwvmobile/dsd-fme"
    dsd_branch="audio_work"
    src_tag="upstream"
fi

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
    else
        # dsd-fme prints `git describe`. A depth-1 fetch drops the tags and
        # the banner becomes "-128-NOTFOUND".
        local fetched=0
        if [[ -f "${dir}/.git/shallow" ]]; then
            if git -C "$dir" fetch --unshallow origin "$branch"; then
                fetched=1
            fi
        elif git -C "$dir" fetch origin "$branch"; then
            fetched=1
        fi
        if [[ "$fetched" -eq 1 ]]; then
            git -C "$dir" checkout -q "$branch"
            # reset prints the new HEAD on stdout. The caller captures only the rev.
            git -C "$dir" reset --hard FETCH_HEAD >/dev/null
        else
            echo "build-dsd-fme: could not fetch ${url} ${branch}; using the clone already on disk." >&2
        fi
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

mbe_src="${cache}/mbelib-${src_tag}"
dsd_src="${cache}/dsd-fme-${src_tag}"
mbe_build="${cache}/mbelib-${src_tag}-build"
dsd_build="${cache}/dsd-fme-${src_tag}-build"
mbe_rev="$(update_repo "$mbe_url" "$mbe_branch" "$mbe_src")"
dsd_rev="$(update_repo "$dsd_url" "$dsd_branch" "$dsd_src")"
stamp="${variant} ${mbe_rev} ${dsd_rev}"

if [[ -x "${prefix}/bin/dsd-fme" && -f "$rev_file" && "$(cat "$rev_file")" == "$stamp" ]]; then
    echo "dsd-fme ${variant} ${dsd_rev} is already installed at ${prefix}/bin/dsd-fme"
    exit 0
fi

build_tree "$mbe_src" "$mbe_build"
build_tree "$dsd_src" "$dsd_build"

[[ -x "${prefix}/bin/dsd-fme" ]] || die "install did not produce ${prefix}/bin/dsd-fme"

# cmake's install_manifest.txt may omit the final newline. A separator
# keeps the next path from being glued onto that line.
{
    for manifest in \
        "${mbe_build}/install_manifest.txt" \
        "${dsd_build}/install_manifest.txt"
    do
        [[ -f "$manifest" ]] || die "missing ${manifest}"
        cat "$manifest"
        printf '\n'
    done
} | awk 'NF && !seen[$0]++' >"${file_list}.tmp"
mv "${file_list}.tmp" "$file_list"
printf '%s\n' "$stamp" >"$rev_file"

echo "Installed ${prefix}/bin/dsd-fme (${variant})"
echo "mbelib ${mbe_rev}"
echo "dsd-fme ${dsd_rev}"
