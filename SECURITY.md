# Security policy

## Reporting a vulnerability

Please do not report security problems in public issues. Use GitHub's private vulnerability reporting instead: open the repository's **Security** tab and choose **Report a vulnerability**. The report is visible only to the maintainer.

Include what you would include in a bug report (see [CONTRIBUTING.md](CONTRIBUTING.md)): the version, a definition or configuration that shows the problem, and what an attacker can achieve with it.

Problems that are in scope include, for example:

- a definition that makes the generator write code that runs or behaves other than its declared types (text escaping comments or literals, injected imports);
- a definition or configuration that makes the CLI read or write files, or send requests, outside what the configuration allows;
- a runtime (`@tsurara/client`, `tsurara-client`, the Go module) sending a request or credentials to a host other than the configured one, or mishandling a response in a way that leaks data.

You will get an answer within a week. Fixes are released as soon as they are ready, and the advisory credits the reporter unless they prefer not to be named.

## Supported versions

tsurara has not been released yet. Once it is, security fixes go into the latest release.
