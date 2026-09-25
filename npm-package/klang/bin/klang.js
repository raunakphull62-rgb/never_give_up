#!/usr/bin/env node
'use strict';
// klang npm wrapper: resolve the platform-specific binary and exec it.
// Pure passthrough – all args (including --version) are forwarded to the
// binary with stdio inherited, and the binary's exit code is propagated.
const { spawnSync } = require('child_process');
const os = require('os');
const path = require('path');
const fs = require('fs');

const PLATFORM_MAP = {
  'linux-x64': { pkg: 'klang-linux-x64', bin: 'klang' },
  'linux-arm64': { pkg: 'klang-linux-arm64', bin: 'klang' },
  'darwin-x64': { pkg: 'klang-darwin-x64', bin: 'klang' },
  'darwin-arm64': { pkg: 'klang-darwin-arm64', bin: 'klang' },
  'win32-x64': { pkg: 'klang-win32-x64', bin: 'klang.exe' },
};

function platformKey() {
  return `${os.platform()}-${os.arch()}`;
}

function candidatePaths(pkg, bin) {
  const candidates = [
    // Sibling install: <root>/node_modules/<pkg>/bin/<bin>.
    // Wrapper lives at <root>/node_modules/klang/bin/klang.js
    // (or .../lib/node_modules/ on `npm install -g`), so ../../<pkg>
    // is the sibling in both layouts.
    path.join(__dirname, '..', '..', pkg, 'bin', bin),
    // npm v7+ can nest optional deps under klang/node_modules.
    path.join(__dirname, '..', 'node_modules', pkg, 'bin', bin),
  ];
  try {
    candidates.push(
      path.join(require.resolve(`${pkg}/package.json`), '..', 'bin', bin)
    );
  } catch {
    // package not resolvable – sibling candidates already cover it
  }
  return candidates;
}

function resolveBinary() {
  // Escape hatch for testing / debugging (used by the Rule 9 proof test).
  if (process.env.KLANG_BINARY) {
    return process.env.KLANG_BINARY;
  }
  const key = platformKey();
  const entry = PLATFORM_MAP[key];
  if (!entry) {
    console.error(
      `klang: unsupported platform "${key}" ` +
        `(supported: ${Object.keys(PLATFORM_MAP).join(', ')})`
    );
    process.exit(1);
  }
  for (const p of candidatePaths(entry.pkg, entry.bin)) {
    try {
      if (fs.existsSync(p)) {
        return p;
      }
    } catch {
      // ignore and try next candidate
    }
  }
  let version = '0.1.0';
  try {
    version = require('../package.json').version;
  } catch {
    // fall back to hardcoded version
  }
  console.error(
    `klang: binary not found for "${key}". ` +
      `Looked for package "${entry.pkg}" (bin/${entry.bin}).`
  );
  console.error(`Try reinstalling: npm install -g klang@${version}`);
  process.exit(1);
}

const binary = resolveBinary();
const args = process.argv.slice(2);
const result = spawnSync(binary, args, { stdio: 'inherit' });
if (result.error) {
  console.error(`klang: failed to launch "${binary}": ${result.error.message}`);
  process.exit(1);
}
process.exit(result.status ?? 1);
