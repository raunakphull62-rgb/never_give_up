# Install Klang

Prebuilt binaries from the `v0.3.0` GitHub release. Pick your platform.
Only the four builds below exist — anything else must be built from
source with Cargo.

## Linux x86_64

```sh
wget https://github.com/raunakphull62-rgb/never_give_up/releases/download/v0.3.0/klang-linux-x86_64.tar.gz
tar -xzf klang-linux-x86_64.tar.gz
chmod +x klang
mv klang /usr/local/bin/klang
klang --version
```

## Linux aarch64, musl (also works under Termux/Android)

```sh
wget https://github.com/raunakphull62-rgb/never_give_up/releases/download/v0.3.0/klang-linux-aarch64-musl-termux.tar.gz
tar -xzf klang-linux-aarch64-musl-termux.tar.gz
chmod +x klang
mv klang /usr/local/bin/klang
klang --version
```

## macOS Apple Silicon (aarch64)

```sh
wget https://github.com/raunakphull62-rgb/never_give_up/releases/download/v0.3.0/klang-macos-aarch64-apple-silicon.tar.gz
tar -xzf klang-macos-aarch64-apple-silicon.tar.gz
chmod +x klang
mv klang /usr/local/bin/klang
klang --version
```

## Windows x86_64

The asset is a `.zip` containing `klang.exe`. In PowerShell:

```powershell
Invoke-WebRequest -Uri https://github.com/raunakphull62-rgb/never_give_up/releases/download/v0.3.0/klang-windows-x86_64.zip -OutFile klang-windows-x86_64.zip
Expand-Archive klang-windows-x86_64.zip -DestinationPath .
Move-Item klang.exe C:\Windows\klang.exe
klang --version
```
