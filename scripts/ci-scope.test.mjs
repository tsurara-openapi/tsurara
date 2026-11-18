// Tests of scripts/ci-scope.mjs, run by the `changes` job of ci.yml before it decides.
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';
import { scope } from './ci-scope.mjs';

const none = {
  all: false,
  rust: false,
  example: false,
  exampleReact: false,
  audit: false,
};
const everything = {
  all: true,
  rust: true,
  example: true,
  exampleReact: true,
  audit: true,
};

test('files of the repository settings and contributors run no job', () => {
  assert.deepEqual(
    scope([
      '.github/dependabot.yml',
      '.github/FUNDING.yml',
      '.github/ISSUE_TEMPLATE/bug_report.yml',
      '.github/pull_request_template.md',
      'CONTRIBUTING.md',
      'SECURITY.md',
    ]),
    none,
  );
});

test('documentation runs the rust job, whose tests read it', () => {
  for (const file of [
    'README.md',
    'CLAUDE.md',
    'THIRD_PARTY_NOTICES.md',
    'LICENSE',
    'LICENSES/Apache-2.0.txt',
    'llms.txt',
    'llms-full.txt',
    'docs/design.md',
    'docs/guide/client.md',
    'docs/languages/go.md',
    'skills/tsurara/SKILL.md',
    'skills/tsurara/references/typescript.md',
    'spec/CHANGELOG.md',
  ]) {
    assert.deepEqual(scope([file]), { ...none, rust: true }, file);
  }
});

test('an example runs its own job and the audit', () => {
  assert.deepEqual(scope(['examples/pnpm-monorepo/pnpm-lock.yaml']), {
    ...none,
    example: true,
    audit: true,
  });
  assert.deepEqual(scope(['examples/react-tanstack/package.json']), {
    ...none,
    exampleReact: true,
    audit: true,
  });
});

test('the jobs of several groups add up', () => {
  assert.deepEqual(
    scope(['docs/guide/react.md', 'examples/react-tanstack/app/page.tsx', 'SECURITY.md']),
    { ...none, rust: true, exampleReact: true, audit: true },
  );
});

test('any other file runs every job', () => {
  for (const file of [
    'crates/tsurara-sem/src/schema.rs',
    'runtimes/python/README.md',
    'runtimes/go/LICENSE',
    'examples/backend-typescript/backend.mjs',
    '.github/workflows/ci.yml',
    'scripts/ci-scope.mjs',
    'spec/operation-def.schema.json',
    'Cargo.lock',
    'pnpm-lock.yaml',
    'mise.toml',
    '.claude-plugin/plugin.json',
  ]) {
    assert.deepEqual(scope(['README.md', file]), everything, file);
  }
});

test('no changed file runs every job', () => {
  assert.deepEqual(scope([]), everything);
});

test('the command reads file names and writes GitHub outputs', () => {
  const out = execFileSync(
    process.execPath,
    [fileURLToPath(new URL('./ci-scope.mjs', import.meta.url))],
    { input: 'docs/guide/client.md\n\nSECURITY.md\n', encoding: 'utf8' },
  );
  assert.equal(out, 'all=false\nrust=true\nexample=false\nexample-react=false\naudit=false\n');
});

test('--all writes every job, for pushes and runs by hand', () => {
  const out = execFileSync(
    process.execPath,
    [fileURLToPath(new URL('./ci-scope.mjs', import.meta.url)), '--all'],
    { input: 'SECURITY.md\n', encoding: 'utf8' },
  );
  assert.equal(out, 'all=true\nrust=true\nexample=true\nexample-react=true\naudit=true\n');
});
