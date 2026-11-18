// Which jobs of ci.yml a pull request needs, from the files it changes (one per line on stdin).
// Writes `name=true|false` lines for $GITHUB_OUTPUT. A file that no group below names runs every
// job, so a new kind of file is checked by everything until it is given a group; tests in
// ci-scope.test.mjs.
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

// Repository settings and texts for contributors: no job reads them.
const settings = [
  /^\.github\/(dependabot\.yml|FUNDING\.yml|pull_request_template\.md)$/,
  /^\.github\/ISSUE_TEMPLATE\//,
  /^(CONTRIBUTING|SECURITY)\.md$/,
];

// Documentation: the tests of tsurara-cli and tsurara-config read it (llms.txt, the documents
// of the MCP server, the skill's code blocks, the configuration guide, the license copies, the
// changelog of the schemas), and they run in the rust job.
const docs = [
  /^(README|CLAUDE|THIRD_PARTY_NOTICES)\.md$/,
  /^LICENSE$/,
  /^LICENSES\//,
  /^llms(-full)?\.txt$/,
  /^docs\//,
  /^skills\//,
  /^spec\/CHANGELOG\.md$/,
];

/** The jobs the changed files need. */
export function scope(files) {
  const jobs = {
    all: false,
    rust: false,
    example: false,
    exampleReact: false,
    audit: false,
  };
  for (const file of files) {
    if (settings.some((re) => re.test(file))) continue;
    if (docs.some((re) => re.test(file))) {
      jobs.rust = true;
    } else if (file.startsWith('examples/pnpm-monorepo/')) {
      jobs.example = jobs.audit = true;
    } else if (file.startsWith('examples/react-tanstack/')) {
      jobs.exampleReact = jobs.audit = true;
    } else {
      jobs.all = true;
    }
  }
  // Nothing changed is unexpected for a pull request; check everything rather than nothing.
  if (files.length === 0) jobs.all = true;
  if (jobs.all) {
    for (const key of Object.keys(jobs)) jobs[key] = true;
  }
  return jobs;
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const files = readFileSync(0, 'utf8')
    .split('\n')
    .filter((line) => line !== '');
  const jobs = scope(files);
  process.stdout.write(
    `all=${jobs.all}\nrust=${jobs.rust}\nexample=${jobs.example}\nexample-react=${jobs.exampleReact}\naudit=${jobs.audit}\n`,
  );
}
