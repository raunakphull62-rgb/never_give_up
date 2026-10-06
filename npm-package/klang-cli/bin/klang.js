#!/usr/bin/env node
'use strict';
// klang-cli launcher: resolve the platform-specific binary and exec it.
// Pure passthrough – all args (including --version) are forwarded to the
// binary with stdio inherited; the binary's exit code and terminating
// signal are propagated back to the caller.
//
// No install scripts, no network calls: the binary arrives via the matching
// optionalDependency (klang-cli-<platform>-<arch>), installed by npm itself.
// Termux note: Node on Termux reports process.platform === "android", which
// maps to the klang-cli-android-arm64 package.
//
// When required as a module (tests), only the pure helpers are exported and
// nothing is spawned. When run directly, main() executes.
const { spawnSync } = require('child_process');
const os = require('os');
const path = require('path');
const fs = require('fs');

const PLATFORM_MAP = {
  'linux-x64': { pkg: 'klang-cli-linux-x64', bin: 'klang' },
  'linux-arm64': { pkg: 'klang-cli-linux-arm64', bin: 'klang' },
  'android-arm64': { pkg: 'klang-cli-android-arm64', bin: 'klang' },
  'darwin-x64': { pkg: 'klang-cli-darwin-x64', bin: 'klang' },
  'darwin-arm64': { pkg: 'klang-cli-darwin-arm64', bin: 'klang' },
  'win32-x64': { pkg: 'klang-cli-win32-x64', bin: 'klang.exe' },
};

function lookup(platform, arch) {
  return PLATFORM_MAP[`${platform}-${arch}`] || null;
}

function platformKey() {
  return `${os.platform()}-${os.arch()}`;
}

function candidatePaths(pkg, bin) {
  const candidates = [
    // Sibling install: <root>/node_modules/<pkg>/bin/<bin>.
    // Launcher lives at <root>/node_modules/klang-cli/bin/klang.js
    // (or .../lib/node_modules/ on `npm install -g`), so ../../<pkg>
    // is the sibling in both layouts.
    path.join(__dirname, '..', '..', pkg, 'bin', bin),
    // npm v7+ can nest optional deps under klang-cli/node_modules.
    path.join(__dirname, '..', 'node_modules', pkg, 'bin', bin),
  ];
  try {
    // Whatever layout npm chose, require.resolve finds the installed copy.
    // This is a local filesystem lookup – no network involved.
    candidates.push(
      path.join(require.resolve(`${pkg}/package.json`), '..', 'bin', bin)
    );
  } catch {
    // package not resolvable – sibling candidates already cover it
  }
  return candidates;
}

function resolveBinary() {
  // Escape hatch for debugging (bypasses platform resolution entirely).
  if (process.env.KLANG_BINARY) {
    return process.env.KLANG_BINARY;
  }
  const key = platformKey();
  const entry = lookup(os.platform(), os.arch());
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
  let version = 'latest';
  try {
    version = require('../package.json').version;
  } catch {
    // fall back to 'latest'
  }
  console.error(
    `klang: binary not found for "${key}". ` +
      `Looked for package "${entry.pkg}" (bin/${entry.bin}). ` +
      `This usually means an unsupported platform, or installed with --no-optional ` +
      `(or --ignore-scripts pruning optional dependencies).`
  );
  console.error(`Try reinstalling: npm install -g klang-cli@${version}`);
  process.exit(1);
}

function main() {
  const binary = resolveBinary();
  const args = process.argv.slice(2);
  const result = spawnSync(binary, args, { stdio: 'inherit' });
  if (result.error) {
    console.error(`klang: failed to launch "${binary}": ${result.error.message}`);
    process.exit(1);
  }
  if (result.signal) {
    // Re-raise the terminating signal so callers observe it conventionally.
    try {
      process.kill(process.pid, result.signal);
    } catch {
      // fall through to non-zero exit
    }
    process.exit(1);
  }
  process.exit(result.status ?? 1);
}

if (require.main === module) {
  main();
}

module.exports = { PLATFORM_MAP, lookup, platformKey, candidatePaths, resolveBinary };
