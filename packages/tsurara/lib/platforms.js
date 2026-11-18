// The platforms the `tsurara` binary is built for, and the npm package that carries each one.
// The released `tsurara` depends on all of them as optional dependencies (`releaseManifest`); npm
// installs the one whose `os` and `cpu` match. The release workflow builds each `target` and writes its package with
// `scripts/npm-platform-packages.mjs`.
import { chmod, copyFile, mkdir, writeFile } from 'node:fs/promises';
import { join } from 'node:path';

export const PLATFORMS = [
  { os: 'darwin', cpu: 'arm64', target: 'aarch64-apple-darwin' },
  { os: 'darwin', cpu: 'x64', target: 'x86_64-apple-darwin' },
  // Static musl binaries, which run on glibc and musl distributions alike.
  { os: 'linux', cpu: 'arm64', target: 'aarch64-unknown-linux-musl' },
  { os: 'linux', cpu: 'x64', target: 'x86_64-unknown-linux-musl' },
  { os: 'win32', cpu: 'arm64', target: 'aarch64-pc-windows-msvc' },
  { os: 'win32', cpu: 'x64', target: 'x86_64-pc-windows-msvc' },
].map((p) => ({
  ...p,
  name: `@tsurara/cli-${p.os}-${p.cpu}`,
  binary: p.os === 'win32' ? 'tsurara.exe' : 'tsurara',
}));

/** The platform package for `process.platform` and `process.arch` values, if there is one. */
export function platformPackage(os, cpu) {
  return PLATFORMS.find((p) => p.os === os && p.cpu === cpu);
}

/**
 * The `package.json` of `tsurara` as released at `version`: with the platform packages as optional
 * dependencies, which the workspace cannot have before they are published.
 */
export function releaseManifest(manifest, version) {
  return {
    ...manifest,
    version,
    optionalDependencies: Object.fromEntries(PLATFORMS.map((p) => [p.name, version])),
  };
}

/**
 * Writes the npm package of one platform into `out`: `package.json`, the binary in `bin/`, and
 * `files` (published name to source path) next to them.
 */
export async function writePlatformPackage(platform, { binary, version, out, files = {} }) {
  await mkdir(join(out, 'bin'), { recursive: true });
  const manifest = {
    name: platform.name,
    version,
    description: `The tsurara binary for ${platform.os} ${platform.cpu}`,
    license: 'MIT',
    repository: { type: 'git', url: 'git+https://github.com/tsurara-openapi/tsurara.git' },
    os: [platform.os],
    cpu: [platform.cpu],
    files: ['bin', ...Object.keys(files)],
    preferUnplugged: true,
  };
  await writeFile(join(out, 'package.json'), `${JSON.stringify(manifest, null, 2)}\n`);
  await copyFile(binary, join(out, 'bin', platform.binary));
  await chmod(join(out, 'bin', platform.binary), 0o755);
  for (const [name, source] of Object.entries(files)) {
    await copyFile(source, join(out, name));
  }
}
