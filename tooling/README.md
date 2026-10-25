# Development tooling

`quality/` contains the repository complexity gate, its regression tests, and the locked Python development environment. It is not part of any deployed Cubebot application.

Run checks through the root `justfile`. The gate scans all authored source in `apps/`, `modules/`, `tooling/`, `tests/`, and any new source directories; the per-function cyclomatic limit remains 15. Process smoke checks live in `tests/system/`.
