'use strict';
// Mapping test for the klang-cli launcher (bin/klang.js).
// No network: everything is local filesystem + in-memory tables.
//
// Covers:
//   1. (platform, arch) -> platform-package mapping, incl. android/arm64.
//   2. Unsupported combos resolve to null.
//   3. Cross-consistency: every mapped package is an optionalDependency of
//      klang-cli pinned to the same version, and every platform package.json
//      has matching name/version/os/cpu/files and no install scripts.
const assert = require('node:assert');
const path = require('node:path');

const launcher = require('../klang-cli/bin/klang.js');

const NPM_DIR = path.join(__dirname, '..');

const CASES = [
  // [platform, arch, expected package (or null)]
  ['linux', 'x64', 'klang-cli-linux-x64'],
  ['linux', 'arm64', 'klang-cli-linux-arm64'],
  ['android', 'arm64', 'klang-cli-android-arm64'], // Termux
  ['darwin', 'x64', 'klang-cli-darwin-x64'],
  ['darwin', 'arm64', 'klang-cli-darwin-arm64'],
  ['win32', 'x64', 'klang-cli-win32-x64'],
  // Unsupported combos must resolve to null (launcher prints a clear error).
  ['freebsd', 'x64', null],
  ['openbsd', 'arm64', null],
  ['sunos', 'x64', null],
  ['linux', 'ia32', null],
  ['linux', 'ppc64', null],
  ['android', 'x64', null], // 64-bit Intel Android: no package shipped
  ['android', 'arm', null],
  ['win32', 'arm64', null],
  ['darwin', 'x64', 'klang-cli-darwin-x64'], // duplicate on purpose: no typo drift
];

let n = 0;
for (const [platform, arch, expected] of CASES) {
  const entry = launcher.lookup(platform, arch);
  assert.strictEqual(
    entry && entry.pkg,
    expected,
    `lookup(${platform}, ${arch}): expected ${expected}, got ${entry && entry.pkg}`
  );
  if (expected) {
    assert.ok(entry.bin, `lookup(${platform}, ${arch}): missing bin filename`);
    const wantBin = platform === 'win32' ? 'klang.exe' : 'klang';
    assert.strictEqual(entry.bin, wantBin, `bin for ${platform}/${arch}`);
  }
  n += 1;
}
console.log(`ok - ${n} platform-mapping cases`);

// Main package consistency.
const mainPkg = require('../klang-cli/package.json');
assert.strictEqual(mainPkg.name, 'klang-cli');
assert.ok(!mainPkg.scripts || Object.keys(mainPkg.scripts).length === 0,
  'klang-cli must define no scripts (works with --ignore-scripts)');
assert.deepStrictEqual(mainPkg.bin, { klang: 'bin/klang.js' });
const mappedPkgs = new Set(
  Object.values(launcher.PLATFORM_MAP).map((e) => e.pkg)
);
assert.strictEqual(mappedPkgs.size, 6, 'expected 6 platform packages');
const optDeps = mainPkg.optionalDependencies || {};
assert.deepStrictEqual(
  new Set(Object.keys(optDeps)),
  mappedPkgs,
  'optionalDependencies must exactly match the launcher platform map'
);
for (const [pkg, ver] of Object.entries(optDeps)) {
  assert.strictEqual(ver, mainPkg.version,
    `${pkg} must be pinned to the main package version (${mainPkg.version})`);
}
console.log('ok - main package optionalDependencies match launcher map, pinned to', mainPkg.version);

// Platform package consistency.
const EXPECTED_OS_CPU = {
  'klang-cli-linux-x64': [['linux'], ['x64']],
  'klang-cli-linux-arm64': [['linux'], ['arm64']],
  'klang-cli-android-arm64': [['android'], ['arm64']],
  'klang-cli-darwin-x64': [['darwin'], ['x64']],
  'klang-cli-darwin-arm64': [['darwin'], ['arm64']],
  'klang-cli-win32-x64': [['win32'], ['x64']],
};
for (const pkg of mappedPkgs) {
  const pkgJson = require(path.join(NPM_DIR, pkg, 'package.json'));
  assert.strictEqual(pkgJson.name, pkg);
  assert.strictEqual(pkgJson.version, mainPkg.version, `${pkg}: version drift`);
  assert.deepStrictEqual(pkgJson.os, EXPECTED_OS_CPU[pkg][0], `${pkg}: os`);
  assert.deepStrictEqual(pkgJson.cpu, EXPECTED_OS_CPU[pkg][1], `${pkg}: cpu`);
  assert.deepStrictEqual(pkgJson.files, ['bin/'], `${pkg}: files must be ["bin/"]`);
  assert.ok(!pkgJson.scripts || Object.keys(pkgJson.scripts).length === 0,
    `${pkg} must define no scripts`);
}
console.log('ok - all 6 platform packages have matching name/version/os/cpu/files, no scripts');

console.log('mapping tests: PASS');
