import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { mkdtemp, readFile, stat, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';
import {
  PLATFORMS,
  platformPackage,
  releaseManifest,
  writePlatformPackage,
} from '../lib/platforms.js';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const bin = join(root, 'bin', 'tsurara.js');
const manifest = JSON.parse(await readFile(join(root, 'package.json'), 'utf8'));

function run(args, env) {
  return spawnSync(process.execPath, [bin, ...args], {
    encoding: 'utf8',
    env: { ...process.env, TSURARA_BINARY: '', ...env },
  });
}

test('names the package of each supported platform', () => {
  assert.equal(platformPackage('darwin', 'arm64')?.name, '@tsurara/cli-darwin-arm64');
  assert.equal(platformPackage('linux', 'x64')?.name, '@tsurara/cli-linux-x64');
  assert.equal(platformPackage('win32', 'x64')?.binary, 'tsurara.exe');
  assert.equal(platformPackage('linux', 'arm64')?.binary, 'tsurara');
  assert.equal(platformPackage('freebsd', 'x64'), undefined);
});

test('is released depending on every platform package at its own version', () => {
  // Not before: the workspace could not install packages that are not published yet.
  assert.equal(manifest.optionalDependencies, undefined);
  const released = releaseManifest(manifest, '1.2.3');
  assert.equal(released.version, '1.2.3');
  assert.deepEqual(
    released.optionalDependencies,
    Object.fromEntries(PLATFORMS.map((p) => [p.name, '1.2.3'])),
  );
  assert.deepEqual(released.bin, manifest.bin);
});

test('runs the binary with the arguments and exits with its status', () => {
  const result = run(
    ['-e', 'console.log(process.argv.slice(1).join(" ")); process.exit(3)', 'a b', 'c'],
    {
      TSURARA_BINARY: process.execPath,
    },
  );
  assert.equal(result.stdout, 'a b c\n');
  assert.equal(result.status, 3);
});

test('explains what to do when the platform package is missing', () => {
  const result = run(['--version']);
  assert.equal(result.status, 1);
  assert.match(result.stderr, /@tsurara\/cli-/);
  assert.match(result.stderr, /optional dependencies/);
});

test('writes a platform package around a binary', async () => {
  const dir = await mkdtemp(join(tmpdir(), 'tsurara-npm-'));
  const binary = join(dir, 'tsurara.exe');
  await writeFile(binary, 'binary');
  const platform = PLATFORMS.find((p) => p.os === 'win32' && p.cpu === 'x64');
  const out = join(dir, 'package');
  await writePlatformPackage(platform, {
    binary,
    version: '1.2.3',
    out,
    files: { LICENSE: binary },
  });
  const pkg = JSON.parse(await readFile(join(out, 'package.json'), 'utf8'));
  assert.equal(pkg.name, '@tsurara/cli-win32-x64');
  assert.equal(pkg.version, '1.2.3');
  assert.deepEqual(pkg.os, ['win32']);
  assert.deepEqual(pkg.cpu, ['x64']);
  assert.equal(pkg.license, 'MIT');
  assert.equal(await readFile(join(out, 'bin', 'tsurara.exe'), 'utf8'), 'binary');
  // Artifacts lose the executable bit between jobs; the package sets it again.
  if (process.platform !== 'win32') {
    assert.equal((await stat(join(out, 'bin', 'tsurara.exe'))).mode & 0o777, 0o755);
  }
  assert.equal(await readFile(join(out, 'LICENSE'), 'utf8'), 'binary');
});
