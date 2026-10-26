# Database migrations

The orchestrator embeds and applies SQLx migrations from this directory when a database is configured. `0001_identity.sql` creates the singleton installation bootstrap record, built-in accounts, hashed sessions and identity audit trail. The build script tracks migration changes. `0002_invitations.sql` adds login-bound expiring invitations with digested bearer tokens and redemption/revocation provenance. `0003_teams_access.sql` adds local teams, scoped grants, durable non-secret command receipts and their transactional event journal. `0004_identity_request_replay.sql` adds separate one-way password proofs for secret-bearing command replay and invitation revisions/redemption identities. Applied migrations are immutable; add a new numbered migration for changes.

`just integration` applies migrations against real isolated PostgreSQL, reconnects, reapplies them and verifies constraints and transaction rollback. Agents never receive database credentials or access the schema directly.

Coordinated maintenance, backup/restore and upgrades under ADRs 0064, 0065 and 0077 remain unimplemented. Development migration tests do not qualify those workflows.
