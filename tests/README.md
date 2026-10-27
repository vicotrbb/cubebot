# System tests

`system/smoke.py` checks built application processes and is run with `just smoke` after `just build` or `just check`.

Tests for one Rust package stay in that package. Regression tests for the complexity gate stay with `tooling/quality`. Scenarios requiring multiple applications belong here; qualification scenarios will be added with the corresponding features.

`just integration` runs the orchestrator database and identity HTTP tests against a disposable isolated PostgreSQL 18 container, then exercises operator bootstrap and real HTTP sessions across server restart. It requires Docker; missing dependencies fail explicitly. No existing application database is used.

The identity integration workflow now also exercises local teams, scoped permission explanations, denied escalation, membership revocation without relogin, bounded cursor listing, durable mutation receipts and replay after a real service restart. PostgreSQL concurrency tests cover stale revisions, separate grant sources, suspended-account cleanup and serialization of event publication across administrators. These establish the identity component; text-conversation content enforcement is covered by the MVP tests; room behavior remains outside the implementation.

Authentication tests reserve the real PostgreSQL attempt budget under concurrent requests, verify persistence across reconnect/restart and reset after the window expires, and prove that invitation replay cannot bypass that budget. Unit checks exercise the shared Argon2 capacity limit and its retryable HTTP response. The HTTP workflow also observes generic credential denial after exhaustion and after a real orchestrator restart.

The text-chat slice adds encrypted provider configuration, explicit provider grants, authorized durable conversation intake, retry receipts, finite inference authority, typed result reconciliation, and real native-binary recovery tests to `just integration`. The controlled transient-provider scenario verifies one retry on the same model and persisted retrying progress. The HTTP workflow checks accepted chat history and duplicate sends across an actual orchestrator-process restart. Frontend byte-boundary and pagination checks run through Bun as part of `just check`.

`system/text_chat_workflow.py` serves an owned disposable PostgreSQL/native-runtime workflow for browser testing on frontend port 3300. Its Kubernetes API and default provider are controlled. A credential-backed run is opt-in through the invoking environment and is recorded separately. See [MVP evidence and limits](../docs/development/text-chat-mvp.md).
