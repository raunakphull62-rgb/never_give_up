# klang-cli-android-arm64

Prebuilt `klang` binary for **android-arm64** (Termux; bionic build via the
Android NDK, API 24+).

This package is not meant to be installed directly – install `klang-cli`
instead, which pulls this in as an optional dependency. The binary will be
placed at `bin/klang` by the release workflow (a real `aarch64-linux-android`
build, NOT the musl static binary: musl cannot resolve DNS on Android, so
ordinary `klang-cli-linux-arm64` must not be substituted here; on Termux,
Node reports `android`/`arm64`).
