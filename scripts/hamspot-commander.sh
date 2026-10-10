#!/bin/sh
# Start the portable Hamspot Commander next to this script.
#
# Config, eframe state, and the Gqrx files Commander writes stay under this
# directory. XDG_RUNTIME_DIR is left alone so PipeWire stays on the session
# socket. The folder can be copied onto an Ubuntu live image; casper does
# not need a path baked in here.

set -eu

HERE=$(unset CDPATH; cd -- "$(dirname -- "$0")" && pwd)
# Install creates these. mkdir covers a tree that was copied without them.
mkdir -p "$HERE/config" "$HERE/data" "$HERE/cache" "$HERE/state"

export XDG_CONFIG_HOME="$HERE/config"
export XDG_DATA_HOME="$HERE/data"
export XDG_CACHE_HOME="$HERE/cache"
export XDG_STATE_HOME="$HERE/state"
export XDG_DATA_DIRS="$HERE/share:${XDG_DATA_DIRS:-/usr/local/share:/usr/share}"

# Station file dialogs open here. Band plans still come from ../bsp next
# to bin/commander, not from this working directory.
cd "$HERE/config"
exec "$HERE/bin/commander" "$@"
