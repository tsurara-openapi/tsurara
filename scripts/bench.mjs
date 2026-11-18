// Performance measurement (design.md 6.7 and chapter 13). For each large definition of
// the corpus (target/corpus, `pnpm corpus:fetch`), measures:
//
// - generation time of the release binary (median of 3 runs),
// - tsc on the whole output: total and check time, and type instantiations (--extendedDiagnostics),
// - one operation imported through the barrel against its subpath: tsc time and instantiations, and
//   the startup time of Node with type stripping.
//
// Results go to target/bench/results.json. With `--baseline <file>` (an earlier results.json), the
// script reports regressions beyond the thresholds below and exits with code 1 if there are any.
// Instantiation counts are deterministic and get a tight threshold; times vary between machines and
// runs and get a loose one.
import { execFileSync, spawnSync } from 'node:child_process';
import { existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const out = join(root, 'target', 'bench');
const tsurara = join(
  root,
  'target',
  'release',
  process.platform === 'win32' ? 'tsurara.exe' : 'tsurara',
);
const tsc = join(root, 'node_modules', '.bin', 'tsc');
const THRESHOLDS = { instantiations: 0.05, time: 0.5 };
const DEFINITIONS = ['github-3.1.json', 'stripe.json', 'kubernetes.json', 'openai.yaml'];

const args = process.argv.slice(2);
const baselinePath = args[args.indexOf('--baseline') + 1];
const baseline =
  args.includes('--baseline') && existsSync(baselinePath)
    ? JSON.parse(readFileSync(baselinePath, 'utf8'))
    : undefined;

const time = (f) => {
  const start = process.hrtime.bigint();
  f();
  return Number(process.hrtime.bigint() - start) / 1e6;
};
const median = (xs) => [...xs].sort((a, b) => a - b)[Math.floor(xs.length / 2)];

/** Runs tsc with --extendedDiagnostics on a project and reads its numbers. */
const diagnostics = (project) => {
  const r = spawnSync(tsc, ['-p', project, '--extendedDiagnostics', '--pretty', 'false'], {
    encoding: 'utf8',
  });
  if (r.status !== 0) throw new Error(`tsc failed for ${project}:\n${r.stdout}${r.stderr}`);
  const number = (label) => {
    const m = new RegExp(`^${label}:\\s+([\\d.]+)`, 'm').exec(r.stdout);
    return m ? Number(m[1]) : undefined;
  };
  return {
    instantiations: number('Instantiations'),
    checkTime: number('Check time') !== undefined ? number('Check time') * 1000 : undefined,
    totalTime: number('Total time') !== undefined ? number('Total time') * 1000 : undefined,
  };
};

const tsconfig = (files) =>
  JSON.stringify({
    compilerOptions: {
      target: 'es2022',
      module: 'nodenext',
      moduleResolution: 'nodenext',
      lib: ['es2022', 'dom'],
      types: [],
      strict: true,
      noEmit: true,
      skipLibCheck: true,
      allowImportingTsExtensions: true,
      exactOptionalPropertyTypes: true,
      noUncheckedIndexedAccess: true,
      verbatimModuleSyntax: true,
      paths: { '@tsurara/client': [join(root, 'runtimes/typescript/src/index.ts')] },
    },
    ...files,
  });

if (!existsSync(tsurara))
  execFileSync('cargo', ['build', '--release', '-p', 'tsurara-cli'], {
    cwd: root,
    stdio: 'inherit',
  });
rmSync(out, { recursive: true, force: true });
mkdirSync(out, { recursive: true });

const results = { date: new Date().toISOString(), node: process.version, definitions: {} };
for (const file of DEFINITIONS) {
  const input = join(root, 'target', 'corpus', file);
  if (!existsSync(input)) {
    console.error(`skipping ${file}: run pnpm corpus:fetch`);
    continue;
  }
  const dir = join(out, file.replace(/\.\w+$/, ''));
  mkdirSync(dir, { recursive: true });
  writeFileSync(
    join(dir, 'tsurara.config.yaml'),
    `input: ${JSON.stringify(input)}\noutput: gen\ntypescript:\n  importExtension: ts\n`,
  );
  const generate = () => execFileSync(tsurara, ['generate'], { cwd: dir, stdio: 'ignore' });
  const generation = median([1, 2, 3].map(() => time(generate)));
  writeFileSync(join(dir, 'package.json'), '{ "type": "module" }');
  writeFileSync(join(dir, 'tsconfig.json'), tsconfig({ include: ['gen/**/*.ts'] }));
  const whole = diagnostics(dir);

  // One operation, through the barrel and through its subpath.
  const index = readFileSync(join(dir, 'gen', 'index.ts'), 'utf8');
  const op = /export \* from '\.\/operations\/(\w+)\.ts'/.exec(index)?.[1];
  const imports = {};
  for (const [kind, from] of [
    ['barrel', './gen/index.ts'],
    ['subpath', `./gen/operations/${op}.ts`],
  ]) {
    const entry = join(dir, `${kind}.ts`);
    writeFileSync(entry, `import { ${op} } from '${from}';\nconsole.log(typeof ${op});\n`);
    const project = join(dir, `tsconfig.${kind}.json`);
    writeFileSync(project, tsconfig({ files: [`${kind}.ts`] }));
    const d = diagnostics(project);
    const startup = median(
      [1, 2, 3].map(() =>
        time(() => execFileSync(process.execPath, [entry], { cwd: dir, stdio: 'ignore' })),
      ),
    );
    imports[kind] = {
      tscTotalTime: d.totalTime,
      instantiations: d.instantiations,
      nodeStartup: startup,
    };
  }
  results.definitions[file] = { generation, tsc: whole, operation: op, imports };
  console.log(
    `${file}: generation ${generation.toFixed(0)} ms, tsc ${whole.totalTime?.toFixed(0)} ms, ${whole.instantiations} instantiations; barrel ${imports.barrel.nodeStartup.toFixed(0)} ms, subpath ${imports.subpath.nodeStartup.toFixed(0)} ms in Node`,
  );
}
writeFileSync(join(out, 'results.json'), `${JSON.stringify(results, null, 2)}\n`);

if (baseline) {
  const regressions = [];
  const compare = (label, now, before, threshold) => {
    if (now === undefined || before === undefined || before === 0) return;
    const change = (now - before) / before;
    if (change > threshold)
      regressions.push(`${label}: ${before} -> ${now} (+${(change * 100).toFixed(0)}%)`);
  };
  for (const [file, r] of Object.entries(results.definitions)) {
    const b = baseline.definitions?.[file];
    if (!b) continue;
    compare(
      `${file} instantiations`,
      r.tsc.instantiations,
      b.tsc.instantiations,
      THRESHOLDS.instantiations,
    );
    compare(`${file} generation (ms)`, r.generation, b.generation, THRESHOLDS.time);
    compare(`${file} tsc (ms)`, r.tsc.totalTime, b.tsc.totalTime, THRESHOLDS.time);
    for (const kind of ['barrel', 'subpath']) {
      compare(
        `${file} ${kind} instantiations`,
        r.imports[kind].instantiations,
        b.imports?.[kind]?.instantiations,
        THRESHOLDS.instantiations,
      );
    }
  }
  if (regressions.length > 0) {
    console.error(`Regressions against ${baselinePath}:\n${regressions.join('\n')}`);
    process.exit(1);
  }
  console.log(`No regressions against ${baselinePath}.`);
}
