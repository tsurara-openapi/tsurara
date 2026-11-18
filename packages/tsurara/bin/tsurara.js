#!/usr/bin/env node
// Runs the native `tsurara` binary of this platform, from its optional dependency
// `@tsurara/cli-<os>-<cpu>`. `TSURARA_BINARY` names another binary, such as a local build.
import { spawnSync } from 'node:child_process';
import { createRequire } from 'node:module';
import { dirname, join } from 'node:path';
import { platformPackage } from '../lib/platforms.js';

function binaryPath() {
  if (process.env.TSURARA_BINARY) return process.env.TSURARA_BINARY;
  const platform = platformPackage(process.platform, process.arch);
  if (platform === undefined) {
    fail(
      `tsurara has no binary for ${process.platform} ${process.arch}. ` +
        'Build it from https://github.com/tsurara-openapi/tsurara and set TSURARA_BINARY to its path.',
    );
  }
  try {
    const manifest = createRequire(import.meta.url).resolve(`${platform.name}/package.json`);
    return join(dirname(manifest), 'bin', platform.binary);
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

const result = spawnSync(binaryPath(), process.argv.slice(2), { stdio: 'inherit' });
if (result.error) fail(`tsurara: ${result.error.message}`);
if (result.signal) process.kill(process.pid, result.signal);
process.exit(result.status ?? 1);
