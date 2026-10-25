# ADR 0066: Encrypted credentials with a separate installation key

Status: Accepted

Date: 2026-10-23

Installation-managed connections contain authentication material used by the orchestrator and authorized agent runtimes. Their storage must support durable updates and recovery without requiring an external secret-management service.

Decision: Store recoverable credential material as encrypted records in PostgreSQL, with the installation encryption key held separately and initially supplied through a Kubernetes Secret. The user selected this model on 2026-10-23. Database records do not contain the plaintext installation key. An external vault is not required.

Usage: An administrator establishes a provider connection. The orchestrator encrypts its authentication material before persistence. When an authorized agent needs inference credentials, the orchestrator reads the applicable record, decrypts it in the trusted runtime path, and delivers only the material that agent is allowed to use under ADR 0031. The installation encryption key is never delivered to agents.

Ownership: A credential-storage module owns encryption, decryption, encrypted-record format, and key-version handling. Connection modules own external authentication state and refresh behavior. Authorization determines permitted use and distribution. Installation administration owns provisioning and recovery of the separately stored key. Domain modules do not implement their own cryptographic formats.

Use an established authenticated-encryption implementation with explicit format and key versions, binding ciphertext to its intended record context. Algorithm, library, nonce handling, and direct versus envelope encryption remain implementation choices requiring qualification. A malformed or unauthentic record must fail explicitly; missing keys must not cause fallback to plaintext or automatic replacement with a new unrelated key.

Credential revision and encryption-key version are distinct. Provider refresh under ADR 0032 updates the authoritative authentication revision; rotating the storage key changes protection of stored data, not the external credential itself. Rotation must be restartable, preserve readable records during transition, and retain required old keys until live records and supported recovery points no longer depend on them. Concurrent refresh and re-encryption must not restore older authentication state.

The key is accessible only to the orchestrator and explicitly authorized recovery paths, not agent tool containers or ordinary user sessions. Decrypted material must stay out of model context, normal logs, shared artifacts, and workspace files. External-tool credential delivery and agent-local credential persistence still need their own contracts. This decision covers recoverable secrets; it does not select the representation of built-in account password verifiers or other non-recoverable authentication data.

Encryption separates database contents from key custody. It does not prevent an actor who controls both the orchestrator and its key from accessing credentials. Kubernetes access controls and deployment configuration must enforce the intended key-access boundary; choosing a Kubernetes Secret does not by itself establish the entire security posture.

Backups under ADR 0064 must identify the key versions required for recovery without exposing key material in ordinary manifests. The database backup alone cannot restore encrypted credentials. Key recovery material must be available through a separately protected procedure, including for cluster loss. Exact key export or escrow mechanics remain open. Restoring old ciphertext does not establish that an external token remains valid.

Alternatives considered: Store each credential in Kubernetes Secrets, or require an external secret manager. Encrypted PostgreSQL records with separately held keys keep connection persistence in Cubebot's existing state model while limiting required services.

Open details: Cryptographic library and record format; key generation and provisioning; rotation mechanics; recovery custody; protected export; bootstrap permissions; and agent-local secret storage and deletion.

Validation scenarios: Encrypt and retrieve an authorized credential; deny ungranted distribution; keep the key out of agent environments; reject altered ciphertext; report missing or wrong keys; resume interrupted key rotation alongside refresh; and restore records with the correct key version without claiming revoked external credentials are usable.
