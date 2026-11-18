/**
 * The other end of tsurara's plugin session (`tsurara_backend::session`): reads one request per line
 * from stdin, calls the plugin's stages and writes one response per line to stdout. tsurara starts it
 * as `node --input-type=module -e "import('@tsurara/plugin/runner').then((m) => m.run(process.argv[1]))" <module>`.
 */
import { createInterface } from 'node:readline';
import { pathToFileURL } from 'node:url';
import type { SessionRequest, SessionResponse, Stage } from './generated/protocol.ts';
import { type PluginDefinition, SESSION_PROTOCOL } from './index.ts';

const STAGES: readonly Stage[] = [
  'transformDocument',
  'transformModel',
  'extend',
  'transformFiles',
];

const answer = (response: SessionResponse): void => {
  process.stdout.write(`${JSON.stringify(response)}\n`);
};

/** Serves the plugin that `modulePath` default-exports until stdin closes. */
export const run = async (modulePath: string): Promise<void> => {
  const loaded = (await import(pathToFileURL(modulePath).href)) as { default?: PluginDefinition };
  const plugin = loaded.default;
  if (plugin === undefined || typeof plugin !== 'object') {
    throw new TypeError(`${modulePath} does not default-export definePlugin({ ... })`);
  }
  const lines = createInterface({ input: process.stdin, crlfDelay: Number.POSITIVE_INFINITY });
  for await (const line of lines) {
    if (line.trim() === '') continue;
    const request = JSON.parse(line) as SessionRequest;
    if (request.type === 'hello') {
      answer({
        type: 'hello',
        protocol: SESSION_PROTOCOL,
        name: plugin.name,
        stages: STAGES.filter((s) => typeof plugin[s] === 'function'),
      });
      continue;
    }
    try {
      const stage = plugin[request.stage] as ((input: unknown) => unknown) | undefined;
      if (stage === undefined) throw new Error(`the plugin has no ${request.stage} stage`);
      answer({ type: 'result', output: await stage(request.input) });
    } catch (e) {
      answer({ type: 'error', message: e instanceof Error ? e.message : String(e) });
    }
  }
};
