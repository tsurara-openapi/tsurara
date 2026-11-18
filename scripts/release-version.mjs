// Checks that every published package has the same version, and with an argument, that it is that
// version (the release workflow passes the version of the tag). Prints the version. The Go modules
// are versioned by their tags (`runtimes/go/v<version>`, `packages/plugin-go/v<version>`), so the
// plugin SDK must require the runtime of the same release: its `replace` directive only applies in
// this repository.
//
//   node scripts/release-version.mjs [version]
import { readFile } from 'node:fs/promises';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const read = (path) => readFile(join(root, path), 'utf8');
const toml = async (path, section) => {
  const text = await read(path);
  const body = text.split(`[${section}]`)[1]?.split(/^\[/m)[0] ?? '';
  return body.match(/^version\s*=\s*"([^"]+)"/m)?.[1];
};
const json = async (path) => JSON.parse(await read(path)).version;
const goRequire = async (path, module) =>
  (await read(path)).match(new RegExp(`^require ${module} v(\\S+)$`, 'm'))?.[1];

const versions = {
  'Cargo.toml (the crates)': await toml('Cargo.toml', 'workspace.package'),
  'runtimes/rust/Cargo.toml': await toml('runtimes/rust/Cargo.toml', 'package'),
  'runtimes/python/pyproject.toml': await toml('runtimes/python/pyproject.toml', 'project'),
  'packages/plugin-python/pyproject.toml': await toml(
    'packages/plugin-python/pyproject.toml',
    'project',
  ),
  'runtimes/typescript/package.json': await json('runtimes/typescript/package.json'),
  'packages/plugin/package.json': await json('packages/plugin/package.json'),
  'packages/tsurara/package.json': await json('packages/tsurara/package.json'),
  'packages/plugin-go/go.mod (runtimes/go)': await goRequire(
    'packages/plugin-go/go.mod',
    'github.com/tsurara-openapi/tsurara/runtimes/go',
  ),
};
const expected = process.argv[2] ?? versions['Cargo.toml (the crates)'];
const wrong = Object.entries(versions).filter(([, v]) => v !== expected);
if (wrong.length > 0) {
  for (const [path, v] of wrong)
    console.error(`${path}: ${v ?? 'no version'}, expected ${expected}`);
  process.exit(1);
}
console.log(expected);
