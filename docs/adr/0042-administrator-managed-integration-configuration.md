# ADR 0042: Administrator-managed integration configuration

Status: Accepted

Date: 2026-10-23

Cubebot supports native core tools and both local and remote MCP integrations. Registering an integration adds an executable or endpoint and its configuration to the platform's available tools. That authority must be distinct from using an existing tool or managing an agent.

Decision: Only installation administrators register and configure integrations. The user selected this policy on 2026-10-23. Administrators define available servers, applicable configuration, credential references, and tool availability. Agent managers may assign approved integrations only within explicitly delegated assignment authority; ordinary agent management does not grant integration-registration rights.

Usage: An administrator registers a local workspace-analysis server or a remote project-management endpoint and determines which tools may be assigned. A manager with the appropriate delegation enables an approved tool for an agent they manage. The manager cannot replace the server address, executable, credential reference, or configured authority to obtain access outside that delegation.

Ownership: Integration configuration owns the registered definition and its revision history. Authorization restricts definition changes to administrators and separately evaluates assignment and invocation rights. The MCP adapter discovers tools against the configured server; discovery is not an administrative grant. Native external integration configuration follows the same administration rule where applicable.

The rule applies to both transport modes selected in ADR 0041. Agents cannot register their own managed MCP integrations merely because they may install software under ADR 0034. Approved local launch definitions must retain their identity through updates; changing code behind a writable executable path must not silently be treated as the same administrator-approved implementation. Executable provenance, package pinning, and update enforcement remain implementation details.

This controls Cubebot's integration registry and managed credential distribution. It does not pretend to prevent arbitrary protocol clients in an environment already granted code execution and network access. Such programs retain only the filesystem, network, and credential authority actually available to them. Preventing access outside grants requires enforcement of those underlying capabilities rather than relying on a registry check alone.

Definition updates must be attributable and must not silently approve tools merely because a server advertises them. Exact handling of configuration revisions, changed tool schemas, and in-flight calls remains open. [ADR 0081](0081-installation-wide-default-connection-access.md) subsequently makes approved connections available to all eligible agents by default, with a per-connection specific-grants mode. Once an administrator approves a capability, the connection's effective audience may use it. Human administration rights remain separate.

External account identity and credential ownership are separate from integration-definition administration. [ADR 0043](0043-installation-managed-external-account-connections.md) selects installation-managed external account connections only, without a user-owned category. It does not override provider-specific consent or confer ownership of an external account. Model-provider connections retain their installation-managed policy under ADR 0028.

Alternatives considered: Allow managers to register integrations within delegated limits. That would reduce administrator involvement but require another delegated configuration model. The user selected administrator-only registration and configuration, with scoped assignment of approved integrations.

Consequences: Adding or changing managed integrations requires administrator involvement. Once configured and granted, agents invoke tools autonomously under ADR 0013 without approval for each call. A single-person installation follows the same rule through its administrator account.

Open details: Credential storage and lifecycle; assignment delegation vocabulary; configuration revisions and activation; executable provenance and updates; catalog visibility; tool-schema changes; audit retention; and in-flight revocation behavior.

Validation scenarios: An administrator registers each transport mode; a delegated manager assigns an approved integration; a manager or agent cannot register or replace a definition; an ungranted tool remains unavailable after discovery; configuration edits are attributable; and an agent's ability to install software does not grant managed integration credentials or registry authority.
