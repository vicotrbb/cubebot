# Contributing

Read [ARCHITECTURE](ARCHITECTURE.md) and the relevant [ADRs](docs/adr/README.md) before changing behavior, and [STATUS](docs/STATUS.md) for what actually runs. [AGENTS.md](AGENTS.md) holds the repository rules for people and coding agents alike.

## Before you open a change

```sh
just check        # formatting, lint, complexity gate, tests, builds
just smoke        # built HTTP services
just integration  # when you touch PostgreSQL, HTTP or identity
```

Say in the change which of these you ran. A passing build, liveness response or mock provider is not evidence of agent, security, recovery or scale behavior.

## Code rules

- **Errors keep their cause.** Wrap an error with context instead of replacing it; never `map_err(|_| ...)` a failure you could carry. Use `#[source]` or `#[from]` on error variants so the chain reaches the log. Redact secrets, not causes.
- **Tracing at boundaries.** Instrument entry and exit points: HTTP handlers, provider calls, Kubernetes calls, database transactions and agent-loop steps. Log once where an error is handled, not at every layer it passes through. Never log credentials, tokens or model-visible secrets.
- **Compile-time-checked SQL.** Use `sqlx::query!` and `query_as!` so queries are checked against the schema at build time, and commit the prepared `.sqlx` metadata so offline builds keep working. Runtime-built SQL needs a reason in the change.
- **Complexity gate on `src` only.** `just complexity` fails any function above cyclomatic complexity 15 in files under a `src/` directory. Tests, scripts and tooling are outside that gate, though Clippy, ESLint and Ruff still apply their own rules there. Refactor instead of raising the limit or adding exclusions.

Existing code predates the first three rules: it uses runtime `sqlx::query` throughout and often discards error causes. Apply the rules to code you write or touch; do not rewrite unrelated code in the same change.

## Decisions

A change that contradicts an accepted ADR needs a new ADR first. Mark the parts it replaces in the older ADRs and in the [index](docs/adr/README.md).

## License

Contributions are accepted under the [Apache License 2.0](LICENSE).
