// The checker of the example backend, for `tsurara test-backend --check 'node check.mjs'`: it runs in
// an output directory and fails when two files declare the same type or a type refers to one that
// no file declares. A real backend runs its language's compiler or linter here.
import { readdirSync, readFileSync } from 'node:fs';

const declared = new Map();
const referenced = [];
for (const file of readdirSync('.').filter((f) => f.endsWith('.go.txt'))) {
  const text = readFileSync(file, 'utf8');
  const match = /^type (\S+)/m.exec(text);
  if (match == null) {
    console.error(`${file}: no type`);
    process.exit(1);
  }
  const name = match[1];
  if (declared.has(name)) {
    console.error(`${file}: ${name} is already declared in ${declared.get(name)}`);
    process.exit(1);
  }
  declared.set(name, file);
  for (const m of text.matchAll(/^\t\S+ (?:\[\]|\*|map\[string\])*([A-Z]\w*)/gm))
    referenced.push([file, m[1]]);
}
for (const [file, name] of referenced) {
  if (!declared.has(name)) {
    console.error(`${file}: ${name} is not declared`);
    process.exit(1);
  }
}
