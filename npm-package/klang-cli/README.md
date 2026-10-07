# klang-cli

Klang language CLI (npm wrapper). The installed command is `klang`
— no Rust toolchain, no compiling.

Install globally:

```sh
npm install -g klang-cli
klang --help
```

Or run without installing:

```sh
npx klang-cli --help
```

The launcher (`bin/klang.js`) resolves to the prebuilt platform binary
shipped in the matching optional package (`klang-cli-linux-x64`,
`klang-cli-linux-arm64`, `klang-cli-android-arm64`,
`klang-cli-darwin-x64`, `klang-cli-darwin-arm64`,
`klang-cli-windows-x64`). No install scripts run and no network calls are
made at install time, so `npm install --ignore-scripts` works.

On Termux (Android), Node reports `android`/`arm64` and the launcher picks
`klang-cli-android-arm64` automatically.
