# Releasing Klang

This covers the binary release flow: GitHub Release assets plus the
`klang-cli` npm packages. For the normal user-side install instructions,
see the README Quick start and `docs/install.md`.

## How a release works

1. Push a version tag: `git tag vX.Y.Z && git push origin vX.Y.Z`
   (or trigger the workflow manually via `workflow_dispatch` for a dry build).
2. The `build` job (`.github/workflows/release.yml`) compiles `klang` for
   six targets: Linux x86_64, Linux aarch64 (musl static, ordinary ARM64
   Linux), Android aarch64 (bionic via the Android NDK, API 24+, for
   Termux), Windows x86_64, macOS Apple Silicon, and macOS Intel
   (cross-compiled on the Apple Silicon runner).
3. The `release` job attaches those archives plus a generated `SHA256SUMS`
   file to a GitHub Release. `install.sh` (repo root) downloads the right
   asset per platform — Termux/Android detected via `$PREFIX` containing
   `com.termux` or `uname -o` reporting `Android` — and verifies against
   `SHA256SUMS` when `sha256sum` is available.
4. The `publish-npm` job downloads the build artifacts, copies each binary
   into its npm platform package, stamps all seven package versions from
   the git tag (`vX.Y.Z` → `X.Y.Z`), and runs `npm publish` for the six
   platform packages first and the `klang-cli` wrapper last.

## npm package layout (`npm-package/`)

- `klang-cli/` — the wrapper. `bin.klang` stays mapped to `bin/klang.js`,
  so the installed command is `klang`. It has **no install scripts** and
  makes no network calls, so `npm install --ignore-scripts` works. Its
  `optionalDependencies` pin all six platform packages to its own version.
- `klang-cli-linux-x64`, `klang-cli-linux-arm64`, `klang-cli-android-arm64`,
  `klang-cli-darwin-x64`, `klang-cli-darwin-arm64`, `klang-cli-windows-x64` —
  one prebuilt binary each, with `os`/`cpu` fields so npm installs only the
  matching one. Termux (Node reports `android`/`arm64`) resolves to
  `klang-cli-android-arm64`, which ships the real `aarch64-linux-android`
  (bionic, NDK, API 24+) build — never the musl binary, which cannot resolve
  DNS on Android. `klang-cli-linux-arm64` ships the musl static build for
  ordinary ARM64 Linux.
- `tests/` — offline launcher tests (mapping table, fake-package spawn and
  exit-code forwarding, error text). Run with plain `node`, no dependencies:
  `node npm-package/tests/test-launcher-mapping.js` and
  `node npm-package/tests/test-launcher-spawn.js`.
- `build.sh` — local cross-build helper that fills the same `bin/`
  directories; only CI artifacts are published, never local builds.
  `android-arm64` needs an NDK (r27c, API 24+) via `ANDROID_NDK_HOME`.

## Android build (NDK)

The `android-arm64` matrix entry targets `aarch64-linux-android` on the
Ubuntu runner. The workflow installs a pinned NDK (`nttld/setup-ndk@v1`,
`ndk-version: r27c`; the preinstalled `ANDROID_NDK_HOME` is used when the
step output is empty) and builds with the NDK LLVM clang wrapper as both
linker and C compiler at API level 24:

- `CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER=.../aarch64-linux-android24-clang`
- `CC_aarch64_linux_android` (same clang wrapper)
- `AR_aarch64_linux_android=.../llvm-ar`

TLS needs no extra work on Android: `Cargo.toml` pins ureq to
`rustls` + `rustls-webpki-roots`, so HTTPS uses the bundled Mozilla roots
(`RootCerts::WebPki`) with no system CA path or config.

Binaries are **not** committed to git (`npm-package/*/bin/klang*` is
gitignored); the release workflow stages them from build artifacts at
publish time.

## Setting up NPM_TOKEN

Publishing needs an npm token with publish rights on `klang-cli` and all
six `klang-cli-*` packages:

1. On npmjs.com, create a token (Automation type for CI, or a granular
   token scoped to the seven packages with read+write).
2. In the GitHub repo: Settings → Secrets and variables → Actions →
   New repository secret named `NPM_TOKEN`, pasting the token value.
3. Never commit the token anywhere. The workflow reads it only as
   `secrets.NPM_TOKEN` into the step environment; it is never printed or
   written to any file.

If `NPM_TOKEN` is not set, the `publish-npm` job logs a skip message and
succeeds — the GitHub Release is unaffected.

## Provenance and idempotency

- The job publishes with `npm publish --provenance --access public`
  (`id-token: write` permission is set for the OIDC attestation). If the
  account/repo combination rejects provenance, the job retries that package
  once without `--provenance` rather than failing the release.
- Before publishing each package, the job runs
  `npm view <name>@<version> version`; if that version is already published
  it is skipped. Re-running the job for a tag is therefore safe.

## Do not publish from the sandbox

Publishing happens **only** from the release workflow with the `NPM_TOKEN`
secret. Never run `npm publish` from a local checkout or sandbox. Local
verification instead:

```sh
node npm-package/tests/test-launcher-mapping.js
node npm-package/tests/test-launcher-spawn.js
for p in npm-package/klang-cli*/; do (cd "$p" && npm pack --dry-run); done
```

`npm pack --dry-run` must list only `package.json`, the README, and
`bin/klang` (or `bin/klang.exe`) for each package — no scripts, no secrets,
no extra files.

## Troubleshooting installs

- `klang: binary not found ... unsupported platform, or installed with
  --no-optional`: npm skipped the platform package. Reinstall without
  `--no-optional`, or install the platform package explicitly, e.g.
  `npm install -g klang-cli-linux-arm64`.
- Unsupported OS/arch prints the supported list; only the six shipped
  combos work without building from source (`cargo build --release`).
