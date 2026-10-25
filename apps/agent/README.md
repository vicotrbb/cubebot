# Agent application

One Rust package owns each agent's runtime and tool-runner implementation:

| Executable            | Entry point              | Intended process                                                          |
| --------------------- | ------------------------ | ------------------------------------------------------------------------- |
| `cubebot-agent`       | `src/main.rs`            | Runtime container: agent loop, provider calls, orchestrator communication |
| `cubebot-tool-runner` | `src/bin/tool-runner.rs` | Tool container: authorized browser, terminal, and MCP execution           |

Both entry points currently print an explicit unimplemented message and exit unsuccessfully. Building them proves package wiring, not agent behavior. No commands, browser, provider requests, or authentication are exposed yet.

The separation is between execution environments, not top-level applications. ADR 0033 requires keeping provider credentials and runtime authority out of the tool process. A shared source package does not establish that isolation; container construction, invocation authentication, and recovery still need implementation and qualification.

The internal modules own agent-specific behavior. Durable records, shared skill-library revisions, connection refresh, and installation permissions remain orchestrator responsibilities. Shared provider adapters and communication types belong under `modules/`.
