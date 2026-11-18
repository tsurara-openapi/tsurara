// The bundle has the operations the app uses and not the others (tree-shaking through the barrel).
import { readdirSync, readFileSync } from 'node:fs';

const dir = new URL('./dist/assets/', import.meta.url);
const code = readdirSync(dir)
  .filter((f) => f.endsWith('.js'))
  .map((f) => readFileSync(new URL(f, dir), 'utf8'))
  .join('\n');
const has = (id) => ['"', "'", '`'].some((q) => code.includes(`${q}${id}${q}`));
const problems = [];
for (const id of ['listPets', 'getPet']) if (!has(id)) problems.push(`${id} is missing`);
for (const id of ['createPet', 'deletePet']) if (has(id)) problems.push(`${id} was not removed`);
if (problems.length > 0) {
  console.error(problems.join('\n'));
  process.exit(1);
}
console.log('The bundle has listPets and getPet only.');
