/**
 * Writing TypeScript files from a plugin: imports that follow the target's import extension, names
 * from the manifest, and the generated-file marker.
 */
import type { Export } from './generated/protocol.ts';

/** The options of a TypeScript target as plugins receive them (`ExtendInput.options`). */
export type TypeScriptTargetOptions = {
  readonly importExtension: '.js' | '.ts' | '';
  readonly clientModule: string;
};

const relative = (from: string, to: string): string => {
  const fromDir = from.split('/').slice(0, -1);
  const toParts = to.split('/');
  let common = 0;
  while (common < fromDir.length && fromDir[common] === toParts[common]) common++;
  const up = fromDir.slice(common).map(() => '..');
  const rest = toParts.slice(common);
  const path = [...up, ...rest].join('/');
  return path.startsWith('.') ? path : `./${path}`;
};

/** A TypeScript file at `path` (relative to the output root), with its imports collected. */
export class TsFile {
  readonly path: string;
  readonly #options: TypeScriptTargetOptions;
  readonly #values = new Map<string, Set<string>>();
  readonly #types = new Map<string, Set<string>>();
  readonly #defaults = new Map<string, string>();
  readonly #lines: string[] = [];

  constructor(path: string, options: TypeScriptTargetOptions) {
    this.path = path;
    this.#options = options;
  }

  /** The specifier of a generated file (`schemas/Pet.ts`) from this file. */
  specifier(file: string): string {
    const withoutExt = file.replace(/\.ts$/, '');
    return `${relative(this.path, withoutExt)}${this.#options.importExtension}`;
  }

  #add(map: Map<string, Set<string>>, from: string, name: string): string {
    const names = map.get(from) ?? new Set<string>();
    names.add(name);
    map.set(from, names);
    return name;
  }

  /** Imports a value of the manifest (a function, an OperationDef) and returns its identifier. */
  import(e: Export): string {
    return this.#add(this.#values, this.specifier(e.file), e.identifier);
  }

  /** Imports a type of the manifest and returns its identifier. */
  importType(e: Export): string {
    return this.#add(this.#types, this.specifier(e.file), e.identifier);
  }

  /** Imports a value from a package (`swr`, the runtime). */
  importFrom(module: string, name: string): string {
    return this.#add(this.#values, module, name);
  }

  /** Imports the default export of a package as `local`. */
  importDefault(module: string, local: string): string {
    this.#defaults.set(module, local);
    return local;
  }

  /** Imports a type from a package. */
  importTypeFrom(module: string, name: string): string {
    return this.#add(this.#types, module, name);
  }

  /** Appends lines of code. */
  line(code: string): this {
    this.#lines.push(code);
    return this;
  }

  /** The contents, starting with `marker`. */
  render(marker: string): string {
    const imports: string[] = [];
    const specifiers = new Set([
      ...this.#defaults.keys(),
      ...this.#values.keys(),
      ...this.#types.keys(),
    ]);
    // Packages first, then relative files, as Biome orders them.
    const rank = (s: string): string => `${s.startsWith('.') ? '1' : '0'}${s}`;
    for (const from of [...specifiers].sort((a, b) => rank(a).localeCompare(rank(b)))) {
      const values = [...(this.#values.get(from) ?? [])].sort();
      const types = [...(this.#types.get(from) ?? [])].filter((t) => !values.includes(t)).sort();
      const local = this.#defaults.get(from);
      if (local !== undefined && values.length > 0) {
        imports.push(`import ${local}, { ${values.join(', ')} } from '${from}';`);
      } else if (local !== undefined) {
        imports.push(`import ${local} from '${from}';`);
      } else if (values.length > 0) {
        imports.push(`import { ${values.join(', ')} } from '${from}';`);
      }
      if (types.length > 0) imports.push(`import type { ${types.join(', ')} } from '${from}';`);
    }
    return `${marker}\n\n${imports.join('\n')}${imports.length > 0 ? '\n\n' : ''}${this.#lines.join('\n')}\n`;
  }
}

const words = (s: string): string[] =>
  s
    .replace(/([a-z0-9])([A-Z])/g, '$1 $2')
    .split(/[^A-Za-z0-9]+/)
    .filter((w) => w !== '');

/** Helpers for writing TypeScript. */
export const ts = {
  /** A file at `path`, with imports that follow the target's options. */
  file: (path: string, options: TypeScriptTargetOptions): TsFile => new TsFile(path, options),
  /** `get pet` → `GetPet`. */
  pascal: (s: string): string =>
    words(s)
      .map((w) => w[0]?.toUpperCase() + w.slice(1))
      .join(''),
  /** `get pet` → `getPet`. */
  camel: (s: string): string => {
    const p = ts.pascal(s);
    return p.charAt(0).toLowerCase() + p.slice(1);
  },
};
