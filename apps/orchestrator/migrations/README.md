# Database migrations

Add the first PostgreSQL migration with the first persistent feature. No schema exists in this scaffold. The orchestrator owns migrations and database access; agents use the orchestrator API.

Migrations must support the coordinated maintenance and backup/restore requirements in ADRs 0064, 0065, and 0077. Do not infer production readiness from the development Compose database.
