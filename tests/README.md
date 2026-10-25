# System tests

`system/smoke.py` checks built application processes and is run with `just smoke` after `just build` or `just check`.

Tests for one Rust package stay in that package. Regression tests for the complexity gate stay with `tooling/quality`. Scenarios requiring multiple applications belong here; qualification scenarios will be added with the corresponding features.
