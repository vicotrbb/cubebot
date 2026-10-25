# ADR 0041: Local and remote MCP server support

Status: Accepted

Date: 2026-10-23

MCP is Cubebot's default external integration path under ADR 0040. Some integrations run as independently hosted endpoints; others need a local executable and access to the agent's workspace.

Decision: Support both remote MCP endpoints using Streamable HTTP and local MCP server subprocesses using stdio. The user selected both modes on 2026-10-23. Both feed the same tool-execution contract and follow the same authorization, resource accounting, and outcome-recording rules.

Usage: An agent uses a remote organizational integration for project records and a local MCP program for workspace processing. The runtime invokes either through the granted tool interface without requiring the model to manage protocol sessions or subprocess pipes.

Local third-party servers execute in the agent's tool container under ADR 0033. They do not run inside the trusted runtime process or inherit its model-provider credentials, platform identity, or database access. Local deployment introduces no extra pod per MCP server. Installation of required executables and dependencies follows ADR 0034's granted tool-environment rules.

Ownership: The MCP adapter owns protocol lifecycle and request/result translation for both transports. Tool execution owns local subprocess supervision, cancellation, and output bounds. Connection configuration describes the authorized endpoint or local executable and necessary configuration. Exact MCP client placement and any forwarding across the runtime/tool interface remain open.

A local process definition must identify its executable, arguments, working directory, and approved environment or credential references. Starting a process is code execution under the tool environment's grants. Registering an MCP tool does not authorize unrelated commands or new credentials. Permission to install software does not by itself authorize registering new tools or expanding an agent's grants.

Remote endpoints can be hosted outside the installation or elsewhere in the cluster. Cubebot connects to them; this decision does not make Cubebot responsible for deploying every remote server. Private endpoint access still requires the applicable grant under ADR 0035. Remote network reachability and MCP authentication do not replace application-level authorization.

Transport state is distinct from Cubebot agent, lane, and task identity. A restarted local server or reconnected remote session must be associated with the correct execution records and revalidated tool catalog. Neither transport reset proves an earlier operation had no effect. Do not replay an uncertain external write merely to recreate a protocol session.

Local process counts, startup concurrency, and resource use must remain bounded. Unused servers need lifecycle handling consistent with agent sleep, while active or uncertain invocations must be reconciled before shutdown. Exact sharing across lanes, startup strategy, and cleanup timing remain open. Local-server logs and protocol output must remain distinguishable and must not expose secrets through ordinary agent results.

Alternatives considered: Remote-only support would avoid local process supervision but require every integration to have a server deployment. Local-only support would place all integration dependencies and processes in agent environments. Both modes support the selected workspace and organizational integration use cases.

Consequences: Cubebot must qualify two transport paths and supervise local server execution. Local packages need not be written in Rust; Cubebot's engine and integration implementation remain Rust, while third-party tools may require other runtimes. Neither transport implies support for every optional MCP feature or legacy transport.

Configuration authority: [ADR 0042](0042-administrator-managed-integration-configuration.md) reserves integration registration and configuration for administrators. Managers can assign approved integrations only within delegated authority.

Account connections: [ADR 0043](0043-installation-managed-external-account-connections.md) selects installation-managed external account connections only.

Open details: Authentication and secret storage; client placement; supported protocol versions; local process sharing and restart policy; executable packaging; remote session recovery; and per-integration qualification.

Validation scenarios: Discover and invoke granted tools through each transport; run a local server without runtime credentials; deny unauthorized registration or private endpoint access; distinguish protocol output from logs; handle local process exit and remote disconnect; preserve uncertain invocation outcomes; and release idle local processes without losing active work.

Protocol reference reviewed during design: [MCP transport specification](https://modelcontextprotocol.io/specification/2025-11-25/basic/transports). Exact supported protocol versions and optional features remain to be qualified.
