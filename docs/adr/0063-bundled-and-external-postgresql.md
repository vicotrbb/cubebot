# ADR 0063: Bundled PostgreSQL by default with external PostgreSQL support

Status: Accepted

Date: 2026-10-23

Cubebot requires PostgreSQL for authoritative structured state. Personal installations need a complete deployment path, while organizations may already operate a database they want to reuse.

Decision: Provide bundled PostgreSQL as the default installation mode and support connecting to externally provisioned PostgreSQL. The user selected this model on 2026-10-23. Both modes use the same Cubebot schema, persistence interfaces, and qualified database requirements.

Usage: An individual installs Cubebot with a PostgreSQL workload and persistent database storage supplied by the installation package. An organization selects external mode and supplies connection configuration and credentials for its database. External mode does not also start an unused bundled database.

Ownership: Installation packaging owns provisioning and configuration of the bundled database. In external mode, the operator owns database infrastructure, availability, and service maintenance. Cubebot owns its application schema, migrations, connection management, and compatibility checks in both modes. Migration authority and runtime database privileges must be explicitly designed; agent pods receive no database credentials under ADR 0009.

The supported PostgreSQL versions, extensions, privileges, and connection settings must form one documented compatibility contract. The current retrieval implementation direction uses PostgreSQL full-text search and pgvector under ADR 0022; extension availability must be qualified in both deployment modes. A database that accepts a connection is not necessarily a compatible Cubebot database. Unsupported requirements must produce actionable diagnostics rather than silent feature loss.

Bundled deployment requires persistent storage, credential lifecycle handling, health checks, and documented upgrade and recovery procedures. This decision does not select an operator, chart dependency, container image, replication topology, or database high-availability guarantee. Their selection must respect the installation's dependency-reduction goal and actual recovery requirements.

Connection secrets are managed through installation configuration and must not appear in normal logs or agent tool environments. External connection verification and transport protection must be specified for the deployment context. External support does not authorize Cubebot to modify unrelated databases or perform provider-level administration.

Changing database mode is a migration operation with backup, verification, and a controlled cutover. Editing a connection setting alone does not transfer existing state. Application upgrades must check schema compatibility and coordinate migrations with active orchestrator ownership; a failed migration must not leave an apparently ready installation serving incompatible state.

Database backups alone do not restore Cubebot. Recovery still needs matching shared-file storage and agent workspace data under ADRs 0008 and 0059. External database availability guarantees do not extend automatically to the single orchestrator or its volumes.

Alternatives considered: Require an external database, or support only the bundled database initially. Supporting both preserves a complete personal installation path and established organizational database operations without introducing separate persistence implementations.

[ADR 0064](0064-coordinated-backup-and-restore.md) assigns application-level backup and restore coordination to Cubebot for both database modes, using qualified database and storage mechanisms.

Open details: Deployment packaging; supported versions and extensions; provisioning components; credential and TLS configuration; migration execution; bundled topology; backup mechanisms and privileges; mode-change procedure; and upgrade and restore qualification.

Validation scenarios: Install and restart with bundled storage; connect to a compatible external database; avoid provisioning bundled resources in external mode; reject missing requirements; preserve data during upgrade; fail readiness on incompatible schema; and verify a controlled migration between modes.
