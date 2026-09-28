# Install Klang

Prebuilt binaries from the `v0.3.0` GitHub release. Pick your platform.
Only the four builds below exist — anything else must be built from
source with Cargo.

## Linux x86_64

```sh
wget https://github.com/raunakphull62-rgb/never_give_up/releases/download/v0.3.0/klang-linux-x86_64.tar.gz
tar -xzf klang-linux-x86_64.tar.gz
chmod +x klang
sudo mv klang /usr/local/bin/klang
klang --version
```

## Linux aarch64 (musl)

Commands not executed by the author of this doc (no aarch64 hardware
in this environment). Asset names and formats are verified real; the
commands themselves were not run here.

```sh
wget https://github.com/raunakphull62-rgb/never_give_up/releases/download/v0.3.0/klang-linux-aarch64-musl-termux.tar.gz
tar -xzf klang-linux-aarch64-musl-termux.tar.gz
chmod +x klang
sudo mv klang /usr/local/bin/klang
klang --version
```

## Android (Termux)

Same asset as Linux aarch64 above. Termux has no `/usr/local/bin`,
so install into `$PREFIX/bin` instead (no `sudo` on Android).
Owner-verified by hand on a physical Android device (2026-09-25,
v0.3.0: wget, tar, chmod, mv, and `klang run` all succeeded);
commands not re-executed by the author of this doc.

```sh
wget https://github.com/raunakphull62-rgb/never_give_up/releases/download/v0.3.0/klang-linux-aarch64-musl-termux.tar.gz
tar -xzf klang-linux-aarch64-musl-termux.tar.gz
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
