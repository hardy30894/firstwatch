#!/bin/sh
# Installs the latest firstwatch release binary.
# Usage: curl -fsSL https://raw.githubusercontent.com/firstwatch-dev/firstwatch/main/install.sh | sh
set -eu

repo="firstwatch-dev/firstwatch"
install_dir="${FIRSTWATCH_INSTALL_DIR:-$HOME/.local/bin}"

os=$(uname -s)
arch=$(uname -m)
case "$os" in
    Darwin) os="apple-darwin" ;;
    Linux) os="unknown-linux-gnu" ;;
    *) echo "unsupported OS: $os (build from source: cargo install --git https://github.com/$repo firstwatch-cli)" >&2; exit 1 ;;
esac
case "$arch" in
    arm64|aarch64) arch="aarch64" ;;
    x86_64|amd64) arch="x86_64" ;;
    *) echo "unsupported arch: $arch" >&2; exit 1 ;;
esac

target="$arch-$os"
url="https://github.com/$repo/releases/latest/download/firstwatch-$target.tar.gz"

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
echo "fetching firstwatch ($target)"
curl -fsSL "$url" -o "$tmp/firstwatch.tar.gz"
tar -xzf "$tmp/firstwatch.tar.gz" -C "$tmp"

mkdir -p "$install_dir"
mv "$tmp/firstwatch" "$install_dir/firstwatch"
chmod +x "$install_dir/firstwatch"

echo "installed $install_dir/firstwatch"
case ":$PATH:" in
    *":$install_dir:"*) ;;
    *) echo "add it to your PATH: export PATH=\"$install_dir:\$PATH\"" ;;
esac
"$install_dir/firstwatch" --version
