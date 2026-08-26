#!/usr/bin/env sh
set -eu

repository="${SWMCTL_REPOSITORY:-Saniee/swmctl}"

version="${1:-latest}"
if [ "$version" = "latest" ]; then
    release_url="https://api.github.com/repos/$repository/releases/latest"
    tag="$(curl --fail --silent --show-error "$release_url" | sed -n 's/.*"tag_name": "\([^"]*\)".*/\1/p' | head -n 1)"
else
    tag="$version"
fi

if [ -z "$tag" ]; then
    printf '%s\n' 'Could not determine the latest swmctl release.' >&2
    exit 1
fi

arch="$(uname -m)"
case "$arch" in
    x86_64|amd64) asset="swmctl-x86_64-unknown-linux-gnu" ;;
    *)
        printf 'No swmctl release binary is published for %s.\n' "$arch" >&2
        printf 'Build from source instead: cargo install --git https://github.com/%s\n' "$repository" >&2
        exit 1
        ;;
esac
base_url="https://github.com/$repository/releases/download/$tag"
tmp_dir="$(mktemp -d)"
trap 'rm -rf "$tmp_dir"' EXIT INT TERM

curl --fail --silent --show-error --location "$base_url/$asset" --output "$tmp_dir/$asset"
curl --fail --silent --show-error --location "$base_url/checksums.txt" --output "$tmp_dir/checksums.txt"

expected="$(sed -n "s/^\([0-9a-fA-F]*\)  $asset$/\1/p" "$tmp_dir/checksums.txt")"
actual="$(sha256sum "$tmp_dir/$asset" | awk '{print $1}')"
if [ -z "$expected" ] || [ "$expected" != "$actual" ]; then
    printf '%s\n' 'Checksum verification failed.' >&2
    exit 1
fi

install_dir="${SWMCTL_INSTALL_DIR:-$HOME/.local/bin}"
mkdir -p "$install_dir"
install -m 755 "$tmp_dir/$asset" "$install_dir/swmctl"
printf 'Installed swmctl %s to %s\n' "$tag" "$install_dir/swmctl"
case ":$PATH:" in
    *":$install_dir:"*) ;;
    *) printf 'Add this directory to PATH before running swmctl:\n  export PATH="%s:$PATH"\n' "$install_dir" ;;
esac
