# tsurara-lang-typescript tests

## Mapping table tests (`cases/*.yaml`)

`cases.rs` reads every YAML file in `cases/` and compares the generated type with the expected type for each case. The case format is described in the comment at the top of `cases.rs`. The output of all cases is passed through `tsc --noEmit` together at the end.

To add a case, append it to the relevant file. Rows of the mapping table in design.md 6.4 go in `table.yaml`.

### Accepted and rejected values

Comparing printed types cannot catch an expected type that is itself wrong (the fixed-length tuples of 028 were such a mistake). So cases also list values, checked by `tsc` against the generated type:

```yaml
- name: prefixItems with items false have optional elements
  schema: { type: array, prefixItems: [{ type: string }, { type: number }], items: false }
  expect: "[string?, number?]"
  accept: [[], [a], [a, 1]]     # allowed by the schema; must type-check
  reject: [[a, 1, true]]        # not allowed; must be a type error
```

- `accept`: values the schema allows. Each becomes `export const acceptN: T = <value>;`.
- `reject`: values the schema does not allow. Each is preceded by `// @ts-expect-error`, so a type that accepts it fails with "Unused '@ts-expect-error' directive".
- `approx`: values the schema does not allow but the type accepts, because the constraint is approximated (design.md 6.4.1: string and number constraints, `oneOf` exclusivity, `patternProperties` patterns, and so on). They must type-check; listing them documents the approximation.

For `schema` cases the lists are for `T`; for `schemas` cases they are maps from type name to a list. Values are written as YAML and emitted as JSON literals. Keep in mind that TypeScript rejects excess properties in object literals, so a value with an extra property is a valid `reject` only when the schema also forbids it.

Every row of the mapping table should have at least one accepted or rejected value.

## Snapshot tests

Full output is checked with [insta](https://insta.rs) snapshots. Snapshots live in `snapshots/` next to each test.

When you change the output on purpose, update the snapshots as follows.

1. Run `INSTA_UPDATE=always cargo test -p tsurara-lang-typescript` (or `cargo insta test --review` if you have `cargo-insta`)
2. Check with `git diff` that only the intended changes are there
3. Commit the snapshot files together with the change

CI runs with `INSTA_UPDATE=no` and fails if the snapshots differ.

## Tests that use tsc

`assert_tsc_passes` in `tsurara-testkit` writes the generated output to a temporary directory and type-checks it with `node_modules/.bin/tsc`. `@tsurara/client` points to `runtimes/typescript/src`.

Without tsc the tests are skipped, so run `pnpm install` at the repository root first. CI sets `TSURARA_REQUIRE_CHECKERS=typescript` so that a missing tsc fails the tests. On failure, the error shows where the files were written.
