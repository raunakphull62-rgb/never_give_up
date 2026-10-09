#!/usr/bin/env bash
#
# Install the prebuilt `klang` binary for your platform.
#
# Usage:
#   ./install.sh                       # latest release, auto-detect platform
#   ./install.sh v0.7.0                # a specific release tag
#   VERSION=v0.7.0 ./install.sh
#   PREFIX=$HOME/.local ./install.sh   # custom install prefix
#
# Termux/Android: detected via $PREFIX containing "com.termux" or
# `uname -o` reporting "Android", and downloads the real bionic build
# (klang-android-arm64.tar.gz). Ordinary ARM64 Linux downloads
# klang-linux-arm64.tar.gz (musl static). The old combined
# klang-linux-aarch64-musl-termux asset no longer exists.
#
# No secrets are read or written; only GitHub release archives are fetched.
set -euo pipefail

REPO="raunakphull62-rgb/never_give_up"
VERSION="${1:-${VERSION:-latest}}"

is_termux_android() {
  # Termux sets PREFIX to .../com.termux/...; `uname -o` is "Android".
  case "${PREFIX:-}" in
    *com.termux*) return 0 ;;
  esac
  local os_name=""
  os_name="$(uname -o 2>/dev/null || echo unknown)"
  [ "$os_name" = "Android" ]
}

detect_asset() {
  local arch os
  arch="$(uname -m)"
  os="$(uname -s)"
  if is_termux_android; then
    case "$arch" in
      aarch64|arm64) echo "klang-android-arm64.tar.gz" ;;
      *) echo "error: unsupported Android arch: $arch (only arm64 is shipped)" >&2; exit 1 ;;
    esac
    return
  fi
  case "$os" in
    Linux)
      case "$arch" in
        x86_64) echo "klang-linux-x86_64.tar.gz" ;;
        aarch64|arm64) echo "klang-linux-arm64.tar.gz" ;;
        *) echo "error: unsupported Linux arch: $arch" >&2; exit 1 ;;
      esac
      ;;
    Darwin)
      case "$arch" in
        arm64|aarch64) echo "klang-macos-aarch64-apple-silicon.tar.gz" ;;
        x86_64) echo "klang-macos-x86_64-intel.tar.gz" ;;
        *) echo "error: unsupported macOS arch: $arch" >&2; exit 1 ;;
      esac
      ;;
    MINGW*|MSYS*|CYGWIN*|Windows_NT)
      echo "klang-windows-x86_64.zip"
      ;;
    *)
      echo "error: unsupported OS: $os" >&2
      exit 1
      ;;
  esac
}

download() { # <url> <dest>
  if command -v curl >/dev/null 2>&1; then
    curl -fL --retry 3 -o "$2" "$1"
  elif command -v wget >/dev/null 2>&1; then
    wget -O "$2" "$1"
  else
    echo "error: need curl or wget to download $1" >&2
    exit 1
  fi
}

main() {
  local asset base_url dest_dir tmpdir archive bin_name
  asset="$(detect_asset)"
  if [ "$VERSION" = "latest" ]; then
    base_url="https://github.com/$REPO/releases/latest/download"
  else
    base_url="https://github.com/$REPO/releases/download/$VERSION"
  fi
  # Termux has no /usr/local/bin; install into $PREFIX/bin there.
  if is_termux_android; then
    dest_dir="${PREFIX:-$HOME/.local}/bin"
  else
    dest_dir="${PREFIX:-$HOME/.local}/bin"
  fi
  tmpdir="$(mktemp -d)"
  trap 'rm -rf "$tmpdir"' EXIT
  echo "install: downloading $asset ($VERSION) ..."
  download "$base_url/$asset" "$tmpdir/$asset"
  # Verify against SHA256SUMS when possible (best effort; warns, not fatal,
  # if the checksum file or sha256sum itself is unavailable).
  if command -v sha256sum >/dev/null 2>&1; then
    if download "$base_url/SHA256SUMS" "$tmpdir/SHA256SUMS" 2>/dev/null; then
      (cd "$tmpdir" && grep -F "  $asset" SHA256SUMS | sha256sum -c -) \
        && echo "install: checksum OK" \
        || { echo "warning: checksum verification failed for $asset" >&2; }
    fi
  fi
  case "$asset" in
    *.tar.gz) tar -xzf "$tmpdir/$asset" -C "$tmpdir" ;;
    *.zip)
      if command -v unzip >/dev/null 2>&1; then
        unzip -o -j "$tmpdir/$asset" -d "$tmpdir"
      else
        echo "error: need unzip for $asset" >&2
        exit 1
      fi
      ;;
  esac
  bin_name="klang"
  [ "$asset" = "klang-windows-x86_64.zip" ] && bin_name="klang.exe"
  mkdir -p "$dest_dir"
  install -m 755 "$tmpdir/$bin_name" "$dest_dir/$bin_name"
  echo "install: wrote $dest_dir/$bin_name"
  "$dest_dir/$bin_name" --version || true
  case ":$PATH:" in
    *":$dest_dir:"*) ;;
    *) echo "install: add $dest_dir to PATH to run 'klang' directly." ;;
  esac
}

main "$@"
