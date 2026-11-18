// Runs @tsurara/client on Hermes the way a React Native app does. For each React Native
// version below:
//
// 1. esbuild bundles React Native's polyfills (`src/prelude.js`) and `src/smoke.ts` into one script.
// 2. Babel with that version's `@react-native/babel-preset` transforms it, as Metro does for app and
//    library code, and esbuild bundles the Babel helpers it requires.
// 3. Current React Native: `hermesc` from `hermes-compiler` (the compiler React Native ships) compiles
//    it, which fails on syntax the current Hermes does not accept. Its VM is not published as a binary.
// 4. React Native 0.75: the Hermes VM of the last Hermes CLI release (v0.13.0, from the same time) runs
//    it, when `TSURARA_HERMES` names its `hermes` binary. Without it this step is skipped with a
//    message, unless `TSURARA_REQUIRE_HERMES=1`.
import { spawnSync } from 'node:child_process';
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { transformSync } from '@babel/core';
import { build } from 'esbuild';

const here = dirname(fileURLToPath(import.meta.url));
const require = createRequire(import.meta.url);
const out = mkdtempSync(join(tmpdir(), 'tsurara-hermes-'));

const fail = (message) => {
  console.error(message);
  process.exit(1);
};

const run = (cmd, args) => {
  const r = spawnSync(cmd, args, { encoding: 'utf8', maxBuffer: 64 * 1024 * 1024 });
  if (r.error) throw r.error;
  return r;
};

const version = (bin) =>
  /release version: (\S+)/i.exec(run(bin, ['-version']).stdout)?.[1] ?? 'unknown version';

const bundle = async (entry) => {
  const r = await build({
    entryPoints: [entry],
    bundle: true,
    format: 'iife',
    target: 'esnext',
    write: false,
    logLevel: 'error',
  });
  return r.outputFiles[0].text;
};

/** The script as Metro would give it to Hermes, with the Babel preset `preset`. */
const script = async (name, preset) => {
  const source = await bundle(join(here, 'src/main.js'));
  const transformed = transformSync(source, {
    babelrc: false,
    configFile: false,
    sourceType: 'script',
    presets: [[require.resolve(preset), { disableImportExportTransform: true }]],
  });
  if (!transformed?.code) throw new Error('babel produced no code');
  // Babel's helpers are `require('@babel/runtime/...')` calls; bundle them as Metro does.
  const transformedPath = join(here, `.${name}.cjs`);
  writeFileSync(transformedPath, transformed.code);
  try {
    const path = join(out, `${name}.js`);
    writeFileSync(path, await bundle(transformedPath));
    return path;
  } finally {
    rmSync(transformedPath);
  }
};

const hermescPath = () => {
  const dir = dirname(require.resolve('hermes-compiler/package.json'));
  const platform = { darwin: 'osx-bin', linux: 'linux64-bin', win32: 'win64-bin' }[
    process.platform
  ];
  return join(dir, 'hermesc', platform, process.platform === 'win32' ? 'hermesc.exe' : 'hermesc');
};

// Current React Native: compile with its Hermes compiler.
const current = await script('current', '@react-native/babel-preset');
const hermesc = hermescPath();
const compiled = run(hermesc, [
  '-emit-binary',
  '-O',
  '-w',
  '-out',
  join(out, 'current.hbc'),
  current,
]);
if (compiled.status !== 0) fail(`hermesc ${version(hermesc)} failed:\n${compiled.stderr}`);
console.log(`React Native 0.88, hermesc ${version(hermesc)}: compiled`);

// React Native 0.75: run on the Hermes VM.
const vm = process.env.TSURARA_HERMES;
if (!vm) {
  const message = 'TSURARA_HERMES is not set: skipped running on a Hermes VM';
  if (process.env.TSURARA_REQUIRE_HERMES === '1') fail(message);
  console.log(message);
  process.exit(0);
}
const legacy = await script('rn075', 'babel-preset-rn075');
const ran = run(vm, ['-w', legacy]);
process.stdout.write(ran.stdout);
if (ran.stderr) process.stderr.write(ran.stderr);
const done = /^done (\d+)\/(\d+)$/m.exec(ran.stdout);
if (ran.status !== 0 || !done || done[1] !== done[2] || /^not ok/m.test(ran.stdout)) {
  fail(`React Native 0.75, hermes ${version(vm)}: failed`);
}
console.log(`React Native 0.75, hermes ${version(vm)}: ${done[1]} checks passed`);
