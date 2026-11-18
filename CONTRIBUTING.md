# Contributing to tsurara

Thank you for your interest in tsurara. Reports of bugs, ideas and questions are welcome as **issues**.

## Pull requests are not accepted

tsurara is developed by its maintainer alone, and **pull requests are closed without review**, however small. This is not a judgment of the change: keeping one author keeps the design, the generated code and the license history consistent, and lets changes go through the same test-first process and checks.

If you have a fix or an idea, open an issue instead. A description of the problem, a minimal definition that shows it, or a sketch of the change you have in mind is the most useful thing you can send; the fix will be written from it and credited in its commit message if you wish. Code attached to an issue is read as an illustration and is not merged as is.

## Before opening an issue

- Search the [existing issues](https://github.com/tsurara-openapi/tsurara/issues?q=is%3Aissue) (open and closed) for the same problem.
- Check the [documentation](README.md#documentation): the guides describe what tsurara generates, what it approximates on purpose, and what it does not check.
- For a security problem, do not open an issue: see [SECURITY.md](SECURITY.md).

## Reporting a bug

The issue form asks for the following. The faster a problem can be reproduced, the faster it can be fixed.

- **Version**: `tsurara --version`, and the runtime package versions (`@tsurara/client`, `tsurara-client`, the Go module).
- **A minimal definition** that shows the problem: as few paths and schemas as possible. If the definition is not public, reduce it until it is safe to share, or rename what is private.
- **The configuration** (`tsurara.config.yaml`), without secrets.
- **What happened**: the command, the warnings or errors it printed, and the part of the generated code that is wrong.
- **What you expected**, and for generated code, the compiler or checker output (`tsc`, pyright, `cargo`, `go vet`) when it fails.
- **Environment**: operating system, and the Node, Python, Rust or Go version if the problem is in generated code or a runtime.

`tsurara inspect warning <code>` explains each warning; include the warning code and the JSON Pointer it prints.

## Suggesting a feature

Describe the problem you want to solve before the solution: what you are building, what you write by hand today, and what you would like the generated code or the CLI to do instead. Mention the language and the OpenAPI version. Requests for a new target language are welcome; say what the language's typical HTTP client and JSON handling look like in your projects.

## Questions

Questions about using tsurara are welcome as issues too. Please include the relevant part of your configuration and definition.

## Conduct

Be respectful and constructive. Issues that are abusive or off topic are closed.
