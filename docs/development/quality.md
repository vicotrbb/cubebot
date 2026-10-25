# Code quality policy

The user requires complexity at most **15 throughout the codebase**. Cubebot interprets this as **cyclomatic complexity <= 15 per function**, including closures, tests, scripts, and executable configuration. A value of 15 passes; 16 fails. A low average cannot hide an overly complex function.

## Enforcement

`just check` is the required local and CI entry point. It runs:

1. Rustfmt, Prettier, and Ruff formatting checks.
2. The repository-wide complexity gate, Clippy with warnings denied, ESLint with zero warnings, and Ruff.
3. Rust tests and complexity regression tests.
4. Rust and frontend builds, followed by strict TypeScript checking.

`just smoke` additionally checks the real orchestrator process and built Bun frontend. It verifies configuration rejection, liveness, graceful shutdown, SSR content, and linked static assets. It is not a substitute for future agent, integration, security, or browser interaction tests.

The cross-language gate uses [Mozilla rust-code-analysis](https://github.com/mozilla/rust-code-analysis), pinned to **0.0.25**, which builds syntax trees rather than estimating Rust branches from tokens. Its cyclomatic metric counts Rust match arms and fallible `?` paths. The runner fails on parser error nodes, command failures, invalid analyzer output, and an empty source scan. A newer syntax construct unsupported by the analyzer requires a qualified analyzer update, not silently skipping the file.

ESLint additionally enforces its classic cyclomatic metric at 15 for JavaScript/TypeScript; inline lint directives are disabled. Ruff applies McCabe complexity at 15 to Python tooling. Clippy's cognitive-complexity threshold is also 15, as a supplemental rule. Cognitive and cyclomatic complexity are different metrics.

## Scope

The scanner walks the repository rather than a fixed list of existing source directories. It checks `.rs`, `.ts`, `.tsx`, `.mts`, `.cts`, `.js`, `.jsx`, `.mjs`, `.cjs`, and `.py` files. New languages require an appropriate gate before executable code is introduced.

Only dependencies, build/cache/tool output directories, and the exact generated file `apps/web/src/routeTree.gen.ts` are excluded. The directory names are explicit in `tooling/quality/complexity.py`; do not put authored source in those directories. Documentation prose, declarative YAML/TOML/JSON, CSS, and simple command recipes do not have function complexity. Any substantive automation logic belongs in analyzed source files rather than embedded shell pipelines.

This is source-level analysis, not analysis of generated machine code or macro expansion. Keep branching logic in ordinary Rust functions instead of hiding it in macros. Generated code is reviewed through its source generator and configuration. Native compiler/type/lint checks remain required alongside the metric.

## Regression coverage

The gate's tests cover the 15/16 boundary across every supported source extension, Rust match arms, TypeScript anonymous arrow functions, suppression comments, new directories, exclusions, empty scans, parser errors, and process exit status. Regression fixtures are generated in temporary directories so intentionally failing code is never excluded from a production source directory.

The first candidate analyzer, Lizard, was rejected after tests exposed undercounted Rust match arms and function suppression comments. Its dependency was removed. Keep those regression cases when changing analyzers.

Refactor excessive complexity into coherent operations with clear responsibilities. Do not raise the threshold, accept an average, add suppressions, or manufacture exclusions to make a check pass.
