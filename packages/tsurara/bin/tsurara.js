#!/usr/bin/env node
// Runs the native `tsurara` binary of this platform, from its optional dependency
// `@tsurara/cli-<os>-<cpu>`. `TSURARA_BINARY` names another binary, such as a local build.
import { spawnSync } from 'node:child_process';
import { chmodSync } from 'node:fs';
import { createRequire } from 'node:module';
import { dirname, join } from 'node:path';
import { platformPackage } from '../lib/platforms.js';

/** The binary to run, and whether it comes from the platform package. */
function binaryPath() {
  if (process.env.TSURARA_BINARY) return { path: process.env.TSURARA_BINARY, packaged: false };
  const platform = platformPackage(process.platform, process.arch);
  if (platform === undefined) {
    fail(
      `tsurara has no binary for ${process.platform} ${process.arch}. ` +
        'Build it from https://github.com/tsurara-openapi/tsurara and set TSURARA_BINARY to its path.',
    );
  }
  try {
    const manifest = createRequire(import.meta.url).resolve(`${platform.name}/package.json`);
    return { path: join(dirname(manifest), 'bin', platform.binary), packaged: true };
  } catch {
    fail(
      `tsurara could not find ${platform.name}, the package with the binary for this platform. ` +
        'Reinstall without skipping optional dependencies (npm --omit=optional, pnpm --no-optional, ' +
        'or a lock file made on another platform skip them).',
    );
  }
}

function fail(message) {
  console.error(message);
  process.exit(1);
}

const binary = binaryPath();
const run = () => spawnSync(binary.path, process.argv.slice(2), { stdio: 'inherit' });
let result = run();
// A platform package whose binary lost its executable bit (the binaries of 0.1.0 were published
// that way): set it and run again.
if (result.error?.code === 'EACCES' && binary.packaged && process.platform !== 'win32') {
  try {
    chmodSync(binary.path, 0o755);
    result = run();
  } catch {
    // Report the first error below.
  }
}
if (result.error) fail(`tsurara: ${result.error.message}`);
if (result.signal) process.kill(process.pid, result.signal);
process.exit(result.status ?? 1);
