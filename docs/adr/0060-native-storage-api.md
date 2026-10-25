# ADR 0060: Native Cubebot storage API

Status: Accepted

Date: 2026-10-23

ADR 0059 selects shared file storage on an orchestrator-managed persistent volume. Its interface must fit Cubebot's resource permissions and recovery contracts without adding an external storage dependency.

Decision: Expose storage through a native Cubebot API and corresponding native agent tools. The user selected this model on 2026-10-23. S3 wire compatibility and a separate S3 credential model are outside the initial interface. Exact transport, endpoints, and schemas remain implementation-design details.

Usage: An agent uploads a report from its workspace, receives a stable artifact identity, and attaches it to an authorized task or room. A human downloads it through the console under the same resource-access rules. Skills use the same storage module for package bytes while their domain module owns revision publication and installation.

Ownership: The storage module owns transfer state, integrity information, file publication, and reclamation. The API adapter authenticates callers and translates requests into that interface. Domain modules own resource references and access decisions. Agents use their existing authenticated orchestrator identity; human clients use platform authentication. Storage does not introduce an independent set of user accounts or grant broad volume access.

The interface must cover bounded uploads, completion and status lookup, metadata inspection, authorized download, and lifecycle operations appropriate to the caller. Use stable operation identities to reconcile lost responses and safe retries. Partial transfer, stored bytes, and successful attachment to a resource must remain distinguishable outcomes. A client must not infer successful publication merely because it finished sending bytes.

Consumers address logical artifacts and resource references, never arbitrary server paths. Shared-volume layout and PostgreSQL schema remain implementation details. Agent tools use the same underlying domain operations as human clients rather than maintaining a second file store. An object identifier is not an access credential.

The native interface preserves the durability, authorization, capacity, and backup requirements in ADR 0059. Choosing a smaller protocol does not weaken those requirements. The detailed upload and recovery flow in the shared-storage design remains an engineering proposal pending specification and validation.

Alternative considered: An S3-compatible interface for existing S3 clients. That would add protocol and authentication semantics beyond the selected Cubebot workflows. The native API directly serves agents and the console with one identity and permission model.

Open details: Transport and endpoint design; transfer limits and resumability; range reads; resource attachment contracts; lifecycle operation scopes; revocation during streaming; and error and retry schemas.

Validation scenarios: Agent upload and console download through the same artifact record; resource-scoped access denial; idempotent completion after lost acknowledgement; explicit incomplete-upload status; rejection of caller-supplied filesystem paths; and unchanged stored content across orchestrator restart.
