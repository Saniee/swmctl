#!/usr/bin/env sh
set -eu

install_dir="${SWMCTL_INSTALL_DIR:-$HOME/.local/bin}"
binary="$install_dir/swmctl"
if [ -e "$binary" ]; then
    rm "$binary"
    printf 'Removed %s\n' "$binary"
else
    printf 'swmctl is not installed at %s\n' "$binary"
fi
