'use strict';
// Postinstall: verify the platform binary exists and is executable.
// Warns (never fails) when the binary is absent so installs on
// unsupported platforms or with --ignore-scripts stay non-fatal.
const os = require('os');
const fs = require('fs');
const path = require('path');

const PLATFORM_MAP = {
  'linux-x64': { pkg: 'klang-linux-x64', bin: 'klang' },
  'linux-arm64': { pkg: 'klang-linux-arm64', bin: 'klang' },
  'darwin-x64': { pkg: 'klang-darwin-x64', bin: 'klang' },
  'darwin-arm64': { pkg: 'klang-darwin-arm64', bin: 'klang' },
  'win32-x64': { pkg: 'klang-win32-x64', bin: 'klang.exe' },
};

function candidatePaths(pkg, bin) {
  const candidates = [
    // Sibling install: <root>/node_modules/<pkg>/bin/<bin>.
    // NOTE: this script lives at <pkg-root>/klang/postinstall.js
    // (one level above bin/klang.js), so the sibling is ONE level up,
    // not two.
    path.join(__dirname, '..', pkg, 'bin', bin),
    path.join(__dirname, 'node_modules', pkg, 'bin', bin),
  ];
  try {
    candidates.push(
      path.join(require.resolve(`${pkg}/package.json`), '..', 'bin', bin)
    );
  } catch {
    // ignore – sibling candidates already cover it
  }
  return candidates;
}

function main() {
  if (process.env.KLANG_BINARY) {
    console.log(
      `klang postinstall: KLANG_BINARY override set (${process.env.KLANG_BINARY}), skipping check.`
    );
    return;
  }
  const key = `${os.platform()}-${os.arch()}`;
  const entry = PLATFORM_MAP[key];
  if (!entry) {
    console.warn(
      `klang postinstall: unsupported platform "${key}", skipping binary check.`
    );
    return;
  }
  const found = candidatePaths(entry.pkg, entry.bin).find((p) => {
    try {
      return fs.existsSync(p);
    } catch {
      return false;
    }
  });
  if (!found) {
    console.warn(
      `klang postinstall: binary not yet present for "${key}" ` +
        `(expected package "${entry.pkg}"). ` +
        `If you installed with --no-optional, install it explicitly: ` +
        `npm install ${entry.pkg}.`
    );
    return;
  }
  if (os.platform() !== 'win32') {
    try {
      fs.chmodSync(found, 0o755);
    } catch (err) {
      console.warn(
        `klang postinstall: could not chmod +x "${found}": ${err.message}`
      );
      return;
    }
  }
  console.log(`klang postinstall: binary OK at ${found}`);
}

main();
