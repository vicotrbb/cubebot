# System tests

`system/smoke.py` checks built application processes and is run with `just smoke` after `just build` or `just check`.

Tests for one Rust package stay in that package. Regression tests for the complexity gate stay with `tooling/quality`. Scenarios requiring multiple applications belong here; qualification scenarios will be added with the corresponding features.

`just integration` runs the orchestrator database and identity HTTP tests against a disposable isolated PostgreSQL 18 container, then exercises operator bootstrap and real HTTP sessions across server restart. It requires Docker; missing dependencies fail explicitly. No existing application database is used.

The identity integration workflow now also exercises local teams, scoped permission explanations, denied escalation, membership revocation without relogin, bounded cursor listing, durable mutation receipts and replay after a real service restart. PostgreSQL concurrency tests cover stale revisions, separate grant sources, suspended-account cleanup and serialization of event publication across administrators. These establish the identity component; actual agent/room content enforcement awaits those domain workflows.
