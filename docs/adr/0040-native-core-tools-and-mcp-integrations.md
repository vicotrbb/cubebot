# ADR 0040: Native core tools with MCP integrations

Status: Accepted

Date: 2026-10-23

Cubebot needs reliable control over its own agent behavior and an extensible way to connect external systems. Implementing every external integration directly would make each supported service depend on Cubebot development and maintenance.

Decision: Keep memory, rooms, messaging, delegation, terminal, and browser tools native to Cubebot. Use Model Context Protocol, MCP, as the default extension path for external integrations, adding native adapters when concrete requirements justify them. The user selected this model on 2026-10-23. Cubebot acts as an MCP client for this integration path; exposing Cubebot itself as an MCP server is not selected here.

Usage: An installation administrator configures an external tool server under [ADR 0042](0042-administrator-managed-integration-configuration.md), and an authorized administrator or delegated manager assigns approved tools to an agent. The agent can invoke those tools alongside native tools through Cubebot's execution interface. For example, it can retrieve a project record through an integration, work with local files, and report a result to a room without the external server owning the room or task lifecycle.

Ownership: Native domain modules own Cubebot's core operations. An MCP adapter owns protocol negotiation, discovery, request and result translation, and connection errors. Authorization owns which external tools may be exposed and invoked. The execution module owns invocation identity, budgets, cancellation, and outcome records across native and MCP tools. This does not require a new deployed gateway or an in-process plugin loader.

Connecting a server does not grant all of its tools. Tool identity must include its configured server identity so an external tool cannot impersonate a native tool or another integration. Discovered schemas and capability changes must be reconciled with grants. Newly discovered tools do not receive authority merely because the server was already connected. [ADR 0044](0044-tool-allowlists-and-resource-restrictions.md) combines named tool allowlists with supported resource restrictions and external credential scopes. Exact grant representation and handling of schema changes remain open.

MCP metadata describes a tool; it is not proof of its behavior or a replacement for credential scopes and enforcement. Validate arguments and bound outputs at the interface. Treat returned content and tool descriptions as external data, without allowing them to rewrite Cubebot's permissions or runtime instructions. Network policy under ADR 0035 still applies to the integration path; routing through a trusted process must not become a bypass.

Within granted permissions, MCP tool use follows ADR 0013's autonomous execution policy without a new per-call approval queue. External authentication or interactive requirements must be surfaced when actually required. Configuring an MCP connection does not automatically authorize additional protocol features such as server-requested model inference or user interaction. Supported features must be explicitly selected and qualified.

Disconnects, timeouts, and cancellation do not prove that a remote operation had no effect. Recovery must preserve uncertain outcomes and use a server's supported reconciliation or idempotency mechanisms where available rather than blindly retrying external writes. Successful protocol delivery and successful application work remain distinct outcomes.

Integration definitions are administrator-managed under ADR 0042. [ADR 0043](0043-installation-managed-external-account-connections.md) makes external account connections installation-managed as well. Authentication storage and client placement remain open. [ADR 0041](0041-local-and-remote-mcp-servers.md) selects both remote Streamable HTTP endpoints and local stdio subprocesses. Local third-party servers run in the tool container under ADR 0033.

Alternative considered: Native integrations only initially. That would give direct control over each adapter but require Cubebot-specific development for every service. The user selected native core tools and MCP extensibility, with targeted native adapters still possible.

Consequences: Cubebot gains a standard extension interface while retaining responsibility for permission enforcement, observability, and failure handling. Server quality and capability vary and need qualification. MCP support alone does not provide a turnkey connector catalog, inbound event system, or universal authentication flow.

[ADR 0045](0045-explicit-tool-search-and-on-demand-discovery.md) selects a small core tool set with explicit tool search and on-demand definition loading. Catalog discovery does not expand grants.

Open details: Supported protocol versions and transport mechanics; local-server lifecycle; client placement; credential storage and lifecycle; grant vocabulary; catalog freshness and schema changes; supported optional protocol features; and integration qualification.

Validation scenarios: Expose and invoke a granted external tool alongside native tools; deny an ungranted or newly added tool; distinguish identical tool names from different servers; handle incompatible schemas and bounded outputs; report authentication failure; cancel without falsely claiming rollback; and retain an uncertain external-write outcome after connection loss.

Protocol reference reviewed during design: [MCP tools specification](https://modelcontextprotocol.io/specification/2025-11-25/server/tools). Exact supported protocol versions remain an implementation qualification decision.
