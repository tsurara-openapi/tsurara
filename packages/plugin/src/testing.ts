/**
 * Testing plugins: `runPlugin` runs tsurara with a plugin on a definition and returns the files it
 * writes.
 */
import { spawnSync } from 'node:child_process';
import { mkdtempSync, readdirSync, readFileSync, rmSync, statSync, writeFileSync } from 'node:fs';
import { dirname, join, relative } from 'node:path';

export type RunPluginOptions = {
  /** The OpenAPI definition, as an object or as YAML or JSON text. */
  readonly definition: unknown;
  /** Extra keys of the target (`language`, `typescript`, ...), as YAML text. */
  readonly config?: string;
  /** The tsurara binary (default: `TSURARA_BIN`, then `tsurara` on the PATH). */
  readonly bin?: string;
};

const collect = (root: string, dir: string, out: Map<string, string>): void => {
  for (const name of readdirSync(dir)) {
    const full = join(dir, name);
    if (statSync(full).isDirectory()) collect(root, full, out);
    else out.set(relative(root, full).replaceAll('\\', '/'), readFileSync(full, 'utf8'));
  }
};

/** Generates with the plugin module at `modulePath` and returns the output files by path. */
export const runPlugin = (modulePath: string, options: RunPluginOptions): Map<string, string> => {
  // Next to the module, so that Node resolves `@tsurara/plugin` from the project's node_modules.
  const dir = mkdtempSync(join(dirname(modulePath), '.tsurara-plugin-'));
  try {
    return generateIn(dir, modulePath, options);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
};

const generateIn = (
  dir: string,
  modulePath: string,
  options: RunPluginOptions,
): Map<string, string> => {
  const definition =
    typeof options.definition === 'string'
      ? options.definition
      : JSON.stringify(options.definition);
  writeFileSync(join(dir, 'openapi.yaml'), definition);
  writeFileSync(
    join(dir, 'tsurara.config.yaml'),
    `input: openapi.yaml\noutput: out\nplugins:\n  - ${JSON.stringify(modulePath)}\n${options.config ?? ''}`,
  );
  const bin = options.bin ?? process.env.TSURARA_BIN ?? 'tsurara';
  const r = spawnSync(bin, ['generate'], { cwd: dir, encoding: 'utf8' });
  if (r.status !== 0) {
    throw new Error(`tsurara generate failed:\n${r.stdout}${r.stderr}`);
  }
  const files = new Map<string, string>();
  collect(join(dir, 'out'), join(dir, 'out'), files);
  return files;
};
