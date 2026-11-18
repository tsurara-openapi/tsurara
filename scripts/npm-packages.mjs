// Writes the npm packages of a release: `tsurara` and one `@tsurara/cli-<os>-<cpu>` package per
// platform, each a directory under the output directory ready for `npm publish`. The release
// workflow runs it with the binaries it built, at `<binaries>/<target>/tsurara[.exe]`.
//
//   node scripts/npm-packages.mjs <version> <binaries> <output>
import { cp, mkdir, readFile, writeFile } from 'node:fs/promises';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import {
  PLATFORMS,
  releaseManifest,
  writePlatformPackage,
} from '../packages/tsurara/lib/platforms.js';

const [version, binaries, output] = process.argv.slice(2);
if (!output) {
  console.error('usage: node scripts/npm-packages.mjs <version> <binaries> <output>');
  process.exit(2);
}
const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const wrapper = join(root, 'packages', 'tsurara');
const notices = join(root, 'target', 'THIRD_PARTY_LICENSES.md');

for (const platform of PLATFORMS) {
  await writePlatformPackage(platform, {
    binary: join(binaries, platform.target, platform.binary),
    version,
    out: join(output, platform.name.replace('@tsurara/', '')),
    files: { LICENSE: join(root, 'LICENSE'), 'THIRD_PARTY_LICENSES.md': notices },
  });
}

const out = join(output, 'tsurara');
await mkdir(out, { recursive: true });
for (const name of ['bin', 'lib', 'LICENSE', 'README.md']) {
  await cp(join(wrapper, name), join(out, name), { recursive: true });
}
// The JSON Schema of the configuration, for editors (docs/guide/configuration.md).
await cp(
  join(root, 'schema', 'tsurara.config.schema.json'),
  join(out, 'schema', 'tsurara.config.schema.json'),
);
const manifest = JSON.parse(await readFile(join(wrapper, 'package.json'), 'utf8'));
delete manifest.scripts;
await writeFile(
  join(out, 'package.json'),
  `${JSON.stringify(releaseManifest(manifest, version), null, 2)}\n`,
);
