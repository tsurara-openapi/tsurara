// A tsurara plugin that writes an SWR hook per read operation (docs/guide/plugins.md). Add it to
// tsurara.config.yaml with `plugins: [./tsurara.plugin.ts]`; it needs `@tsurara/plugin` and Node 22.18
// or later (type stripping), and the app needs `swr`.
import { definePlugin, type TypeScriptTargetOptions, ts } from '@tsurara/plugin';

export default definePlugin({
  name: 'swr-hooks',
  extend({ language, manifest, marker, options, model }) {
    if (language !== 'typescript' || manifest == null) return [];
    const target = options as TypeScriptTargetOptions;
    const reads = new Set(model.operations.filter((op) => op.method === 'GET').map((op) => op.id));
    return manifest.operations.flatMap((op) => {
      const fn = op.function;
      const data = op.types?.data;
      const result = op.types?.result;
      if (!reads.has(op.operationId) || fn == null || data == null || result == null) return [];
      const name = `use${ts.pascal(fn.identifier)}`;
      const f = ts.file(`swr/${name}.ts`, target);
      const useSWR = f.importDefault('swr', 'useSWR');
      const swrResponse = f.importTypeFrom('swr', 'SWRResponse');
      const client = f.importTypeFrom(target.clientModule, 'Client');
      const apiError = f.importTypeFrom(target.clientModule, 'ApiError');
      const successData = f.importTypeFrom(target.clientModule, 'SuccessData');
      const unwrap = f.importFrom(target.clientModule, 'unwrap');
      const call = f.import(fn);
      const dataType = f.importType(data);
      const resultType = f.importType(result);
      f.line(
        `/** \`${fn.identifier}\` with SWR; the key is the operation id and the request data. */`,
      );
      f.line(`export function ${name}<Ctx>(`);
      f.line(`  client: ${client}<Ctx>,`);
      f.line(`  data: ${dataType} | null,`);
      f.line(`): ${swrResponse}<${successData}<${resultType}>, ${apiError}<${resultType}>> {`);
      f.line(`  return ${useSWR}(data === null ? null : ['${op.operationId}', data], async () =>`);
      f.line(`    ${unwrap}(await ${call}(client, data as ${dataType})),`);
      f.line('  );');
      f.line('}');
      return [{ path: f.path, contents: f.render(marker) }];
    });
  },
});
