// Writes the notices of the crates the `tsurara` binary links (normal dependencies of tsurara-cli,
// for every platform), with each crate's license files, as Markdown. Ship the output with every
// binary release: the MIT, BSD and Apache licenses of the dependencies ask for their notices.
//
//   node scripts/third-party-licenses.mjs [output]   (default: target/THIRD_PARTY_LICENSES.md)
import { execFileSync } from 'node:child_process';
import { mkdir, readdir, readFile, writeFile } from 'node:fs/promises';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const output = process.argv[2] ?? join(root, 'target', 'THIRD_PARTY_LICENSES.md');
const metadata = JSON.parse(
  execFileSync('cargo', ['metadata', '--format-version', '1', '--locked'], {
    cwd: root,
    maxBuffer: 256 * 1024 * 1024,
  }).toString(),
);
const packages = new Map(metadata.packages.map((p) => [p.id, p]));
const nodes = new Map(metadata.resolve.nodes.map((n) => [n.id, n]));
const workspace = new Set(metadata.workspace_members);
const cli = metadata.packages.find((p) => p.name === 'tsurara-cli' && workspace.has(p.id));

// The normal (not dev or build) dependencies reachable from the CLI.
const linked = new Set();
const queue = [cli.id];
while (queue.length > 0) {
  const id = queue.pop();
  for (const dep of nodes.get(id)?.deps ?? []) {
    const normal = dep.dep_kinds.some((k) => k.kind === null);
    if (normal && !linked.has(dep.pkg)) {
      linked.add(dep.pkg);
      queue.push(dep.pkg);
    }
  }
}

const licenseFile = /^(LICEN[CS]E|COPYING|NOTICE|UNLICENSE)/i;
const sections = [];
for (const id of [...linked].sort((a, b) =>
  packages.get(a).name.localeCompare(packages.get(b).name),
)) {
  const p = packages.get(id);
  if (workspace.has(id)) continue;
  const dir = dirname(p.manifest_path);
  const files = (await readdir(dir)).filter((f) => licenseFile.test(f)).sort();
  let text = `## ${p.name} ${p.version}\n\nLicense: ${p.license ?? p.license_file ?? 'unknown'}`;
  if (p.repository) text += `\nRepository: ${p.repository}`;
  for (const f of files) {
    const body = await readFile(join(dir, f), 'utf8');
    text += `\n\n### ${f}\n\n\`\`\`text\n${body.trimEnd()}\n\`\`\``;
  }
  if (files.length === 0)
    text += '\n\n(The crate has no license file; its license is the SPDX expression above.)';
  sections.push(text);
}

await mkdir(dirname(output), { recursive: true });
await writeFile(
  output,
  `# Third-party licenses of the tsurara binary\n\nThe binary links these crates (${sections.length}).\n\n${sections.join('\n\n')}\n`,
);
console.log(`wrote ${sections.length} crates to ${output}`);
