# Install Klang

Prebuilt binaries from the GitHub releases. Pick your platform.
Anything else must be built from source with Cargo.

The fastest path is the installer script (detects Termux/Android
automatically via `$PREFIX` / `uname -o`):

```sh
curl -fsSL https://raw.githubusercontent.com/raunakphull62-rgb/never_give_up/main/install.sh | bash
```

Or pass a specific release tag: `... | bash -s -- v0.7.0`.
Every release also ships a `SHA256SUMS` file; `install.sh` verifies against
it when `sha256sum` is available.

## Linux x86_64

```sh
wget https://github.com/raunakphull62-rgb/never_give_up/releases/download/v0.3.0/klang-linux-x86_64.tar.gz
tar -xzf klang-linux-x86_64.tar.gz
chmod +x klang
sudo mv klang /usr/local/bin/klang
klang --version
```

## Linux aarch64 (musl static; ordinary ARM64 Linux)

Commands not executed by the author of this doc (no aarch64 hardware
in this environment). Asset names and formats are verified real; the
commands themselves were not run here.

```sh
wget https://github.com/raunakphull62-rgb/never_give_up/releases/latest/download/klang-linux-arm64.tar.gz
tar -xzf klang-linux-arm64.tar.gz
chmod +x klang
sudo mv klang /usr/local/bin/klang
klang --version
```

## Android (Termux; bionic NDK build)

A real `aarch64-linux-android` (bionic, API 24+) build. Do NOT use the
musl `klang-linux-arm64` asset here: the musl static binary cannot resolve
DNS on Android (no `/etc/resolv.conf`), so `klang add` fails unless run
under `termux-chroot`. Termux has no `/usr/local/bin`, so install into
`$PREFIX/bin` instead (no `sudo` on Android).

```sh
wget https://github.com/raunakphull62-rgb/never_give_up/releases/latest/download/klang-android-arm64.tar.gz
tar -xzf klang-android-arm64.tar.gz
chmod +x klang
mv klang $PREFIX/bin/klang
klang --version
```

## macOS Apple Silicon (aarch64)

`wget` is not installed on macOS by default. Install it first:

```sh
brew install wget
```

Commands not executed by the author of this doc (Linux-only
environment here).

```sh
wget https://github.com/raunakphull62-rgb/never_give_up/releases/download/v0.3.0/klang-macos-aarch64-apple-silicon.tar.gz
tar -xzf klang-macos-aarch64-apple-silicon.tar.gz
chmod +x klang
sudo mv klang /usr/local/bin/klang
klang --version
```

## Windows x86_64

Windows ships neither `wget` nor `unzip` natively. The commands below
assume both are available, for example through Git Bash or WSL.

Commands not executed by the author of this doc (Linux-only
environment here).

```sh
wget https://github.com/raunakphull62-rgb/never_give_up/releases/download/v0.3.0/klang-windows-x86_64.zip
unzip klang-windows-x86_64.zip
mv klang.exe /usr/local/bin/klang.exe
klang --version
```
