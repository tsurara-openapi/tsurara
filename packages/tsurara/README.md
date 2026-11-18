# tsurara

Generate types and one call function per operation from OpenAPI 3.0, 3.1, 3.2 and Swagger 2.0, for TypeScript, Python, Rust and Go.

```sh
npm install --save-dev tsurara
npx tsurara init
npx tsurara generate
```

This package runs the native binary of your platform, which npm installs from an optional dependency (`@tsurara/cli-<os>-<cpu>`): macOS, Linux and Windows on x64 and arm64. Set `TSURARA_BINARY` to use another build.

The documentation is in the [repository](https://github.com/tsurara-openapi/tsurara).
