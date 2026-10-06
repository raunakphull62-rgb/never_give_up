'use strict';
// Spawn test for the klang-cli launcher (bin/klang.js).
// No network: builds a fake install tree under a temp dir and runs the real
// launcher against a fake platform binary.
//
// Covers:
//   1. Launcher resolves the fake platform package via require.resolve,
//      spawns its binary with stdio inherited, and forwards stdout + args.
//   2. The binary's exit code is forwarded.
//   3. With no platform package installed, the launcher exits non-zero and
//      prints the unsupported-platform / --no-optional error text.
const assert = require('node:assert');
const { spawnSync } = require('node:child_process');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');

const launcher = require('../klang-cli/bin/klang.js');

const entry = launcher.lookup(os.platform(), os.arch());
if (!entry) {
  console.log(`SKIP - host ${os.platform()}/${os.arch()} has no klang-cli platform package`);
  process.exit(0);
}

const REAL_LAUNCHER = path.join(__dirname, '..', 'klang-cli', 'bin', 'klang.js');

function writeFakeBinary(binPath) {
  fs.mkdirSync(path.dirname(binPath), { recursive: true });
  fs.writeFileSync(
    binPath,
    '#!/usr/bin/env node\n' +
      "'use strict';\n" +
      '// Fake klang binary: echo argv as JSON, exit with $FAKE_EXIT.\n' +
      'console.log(JSON.stringify(process.argv.slice(2)));\n' +
      'process.exit(parseInt(process.env.FAKE_EXIT || "0", 10));\n'
  );
  fs.chmodSync(binPath, 0o755);
}

function makeInstall({ withPlatformPackage }) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'klang-cli-test-'));
  const nm = path.join(root, 'node_modules');
  const launcherDir = path.join(nm, 'klang-cli', 'bin');
  fs.mkdirSync(launcherDir, { recursive: true });
  fs.copyFileSync(REAL_LAUNCHER, path.join(launcherDir, 'klang.js'));
  fs.writeFileSync(
    path.join(nm, 'klang-cli', 'package.json'),
    JSON.stringify({ name: 'klang-cli', version: '0.0.0-test' })
  );
  if (withPlatformPackage) {
    const pkgDir = path.join(nm, entry.pkg);
    fs.mkdirSync(pkgDir, { recursive: true });
    fs.writeFileSync(
      path.join(pkgDir, 'package.json'),
      JSON.stringify({ name: entry.pkg, version: '0.0.0-test' })
    );
    writeFakeBinary(path.join(pkgDir, 'bin', entry.bin));
  }
  return root;
}

function runLauncher(root, args, extraEnv) {
  const env = { ...process.env, ...extraEnv };
  delete env.KLANG_BINARY; // exercise the real require.resolve path
  return spawnSync('node', [path.join(root, 'node_modules', 'klang-cli', 'bin', 'klang.js'), ...args], {
    env,
    encoding: 'utf8',
  });
}

let root = makeInstall({ withPlatformPackage: true });
try {
  // 1. Args + stdout passthrough.
  let r = runLauncher(root, ['--help', 'hello.klang'], { FAKE_EXIT: '0' });
  assert.strictEqual(r.status, 0, `expected exit 0, stderr: ${r.stderr}`);
  assert.strictEqual(r.stdout.trim(), JSON.stringify(['--help', 'hello.klang']));
  console.log('ok - spawns platform binary, forwards args/stdout');

  // 2. Exit code forwarding.
  r = runLauncher(root, [], { FAKE_EXIT: '42' });
  assert.strictEqual(r.status, 42, `expected exit 42, got ${r.status} (stderr: ${r.stderr})`);
  console.log('ok - forwards binary exit code (42)');
} finally {
  fs.rmSync(root, { recursive: true, force: true });
}

// 3. Missing platform package -> clear error, non-zero exit.
root = makeInstall({ withPlatformPackage: false });
try {
  const r = runLauncher(root, ['--version'], {});
  assert.notStrictEqual(r.status, 0, 'expected non-zero exit when binary is missing');
  const err = r.stderr || '';
  assert.ok(
    err.includes('unsupported platform, or installed with --no-optional'),
    `error text missing hint phrase, got: ${err}`
  );
  assert.ok(err.includes(entry.pkg), `error text should name package ${entry.pkg}, got: ${err}`);
  console.log('ok - missing-package error names the package and hints at --no-optional');
} finally {
  fs.rmSync(root, { recursive: true, force: true });
}

console.log('spawn tests: PASS');
