# klang-cli-android-arm64

Prebuilt `klang` binary for **android-arm64** (Termux; musl static build).

This package is not meant to be installed directly – install `klang-cli`
instead, which pulls this in as an optional dependency. The binary will be
placed at `bin/klang` by the release workflow (same static binary as
`klang-cli-linux-arm64`; on Termux, Node reports `android`/`arm64`).
