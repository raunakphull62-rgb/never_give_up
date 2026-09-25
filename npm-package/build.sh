#!/usr/bin/env bash
#
# Build prebuilt `klang` binaries for all npm platform packages.
#
# CRITICAL (Colab/Drive): the repo's ./target dir is on a no-exec mount,
# so every cargo invocation MUST use CARGO_TARGET_DIR=/tmp/klang-target.
#
# Usage:
#   ./npm-package/build.sh                 # build all five targets
#   ./npm-package/build.sh linux-x64       # build one (npm platform name)
#   ./npm-package/build.sh linux-x64 win32-x64
#
# Output: each binary lands in npm-package/klang-<platform>/bin/
#   (bin/klang on unix, bin/klang.exe on Windows).
#
# Notes:
# - Linux uses *-unknown-linux-musl so the binary is statically linked
#   (required for Alpine / Termux).
# - macOS/Windows cross builds from Linux need the right linker:
#     * darwin targets need osxcross (APPLE targets cannot link otherwise);
#     * the Windows target uses -gnu (mingw-w64) so it links without MSVC.
#   If the linker for a target is missing, that target fails loudly with
#   a hint instead of producing a silently broken binary.
set -euo pipefail

# --- Rust toolchain on PATH (Colab: installed under $HOME, not on PATH) ---
if [ -f "$HOME/.cargo/env" ]; then
  # shellcheck disable=SC1091
  source "$HOME/.cargo/env"
fi
# Fallback for non-interactive shells where env script layout differs.
for _d in "$HOME/.cargo/bin" "/usr/local/cargo/bin"; do
  case ":$PATH:" in
    *":$_d:"*) ;;
    *) [ -d "$_d" ] && export PATH="$_d:$PATH" ;;
  esac
done
unset _d

# --- no-exec-mount guard -------------------------------------------------
export CARGO_TARGET_DIR=/tmp/klang-target

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
NPM_DIR="$ROOT/npm-package"
CRATE_NAME="klang"

# npm-platform -> "rust_target;dest_dir;dest_file;link_hint"
declare -A TARGETS=(
  ["linux-x64"]="x86_64-unknown-linux-musl;klang-linux-x64;klang;need musl-tools (sudo apt install musl-tools)"
  ["linux-arm64"]="aarch64-unknown-linux-musl;klang-linux-arm64;klang;need musl-tools + aarch64 musl cross gcc"
  ["darwin-x64"]="x86_64-apple-darwin;klang-darwin-x64;klang;need osxcross for darwin linking from Linux"
  ["darwin-arm64"]="aarch64-apple-darwin;klang-darwin-arm64;klang;need osxcross for darwin linking from Linux"
  ["win32-x64"]="x86_64-pc-windows-gnu;klang-win32-x64;klang.exe;need mingw-w64 (sudo apt install mingw-w64)"
)

WANT=("$@")
if [ "${#WANT[@]}" -eq 0 ]; then
  WANT=(linux-x64 linux-arm64 darwin-x64 darwin-arm64 win32-x64)
fi

build_one() {
  local platform="$1"
  local spec="${TARGETS[$platform]:-}"
  if [ -z "$spec" ]; then
    echo "error: unknown platform '$platform' (want one of: ${!TARGETS[*]})" >&2
    exit 1
  fi
  IFS=';' read -r triple dest_dir dest_file hint <<< "$spec"

  echo "==> [$platform] rustup target add $triple"
  rustup target add "$triple"

  echo "==> [$platform] cargo build --release --target $triple"
  echo "    (CARGO_TARGET_DIR=$CARGO_TARGET_DIR)"
  if ! cargo build --release --target "$triple" --manifest-path "$ROOT/Cargo.toml"; then
    echo "error: build failed for $platform ($triple)." >&2
    echo "hint: $hint" >&2
    exit 1
  fi

  local src="$CARGO_TARGET_DIR/$triple/release/$CRATE_NAME"
  if [ "$dest_file" = "klang.exe" ]; then
    # -gnu toolchain emits `klang` or `klang.exe` depending on rustc version.
    if [ ! -f "$src" ] && [ -f "$src.exe" ]; then
      src="$src.exe"
    fi
  fi
  if [ ! -f "$src" ]; then
    echo "error: expected binary missing at $src" >&2
    exit 1
  fi

  local out_dir="$NPM_DIR/$dest_dir/bin"
  mkdir -p "$out_dir"
  cp -f "$src" "$out_dir/$dest_file"
  if [ "$dest_file" != "klang.exe" ]; then
    chmod +x "$out_dir/$dest_file"
  fi
  echo "==> [$platform] wrote $out_dir/$dest_file"
}

for platform in "${WANT[@]}"; do
  build_one "$platform"
done

echo "All requested targets built."
