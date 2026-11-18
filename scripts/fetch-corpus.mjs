// Downloads the real-world corpus (corpus/sources.json) into target/corpus, checking each file's
// sha256. Files that are already present with the right hash are kept.
import { createHash } from 'node:crypto';
import { mkdir, readFile, rename, writeFile } from 'node:fs/promises';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const dir = join(root, 'target', 'corpus');
const { sources } = JSON.parse(await readFile(join(root, 'corpus', 'sources.json'), 'utf8'));
const sha256 = (bytes) => createHash('sha256').update(bytes).digest('hex');

await mkdir(dir, { recursive: true });
let failed = false;
for (const source of sources) {
  const path = join(dir, source.file);
  const existing = await readFile(path).catch(() => undefined);
  if (existing && sha256(existing) === source.sha256) {
    console.log(`${source.name}: up to date`);
    continue;
  }
  const response = await fetch(source.url);
  if (!response.ok) {
    console.error(`${source.name}: HTTP ${response.status} from ${source.url}`);
    failed = true;
    continue;
  }
  const bytes = Buffer.from(await response.arrayBuffer());
  const actual = sha256(bytes);
  if (actual !== source.sha256) {
    console.error(`${source.name}: sha256 ${actual} does not match ${source.sha256}`);
    failed = true;
    continue;
  }
  await writeFile(`${path}.tmp`, bytes);
  await rename(`${path}.tmp`, path);
  console.log(`${source.name}: downloaded (${bytes.length} bytes)`);
}
if (failed) process.exit(1);
