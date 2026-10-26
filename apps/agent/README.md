# Agent application

One Rust package owns each agent's runtime and tool-runner implementation:

| Executable            | Entry point              | Intended process                                                          |
| --------------------- | ------------------------ | ------------------------------------------------------------------------- |
| `cubebot-agent`       | `src/main.rs`            | Runtime container: agent loop, provider calls, orchestrator communication |
| `cubebot-tool-runner` | `src/bin/tool-runner.rs` | Tool container: authorized browser, terminal, and MCP execution           |

Normal runtime and tool-runner execution still print an explicit unimplemented message and exit unsuccessfully. The runtime also supports the trusted init-container mode `cubebot-agent --initialize-storage runtime|tools`. It initializes the corresponding fixed directory beneath `/storage`, validates ownership/type, restores private permissions and synchronizes directory metadata while retaining existing contents. The pod compiler invokes it with the matching non-root identity before application containers start. This local operation does not implement agent work, arbitrary commands, browser, provider requests or authentication.

The separation is between execution environments, not top-level applications. ADR 0033 requires keeping provider credentials and runtime authority out of the tool process. A shared source package does not establish that isolation; container construction, invocation authentication, and recovery still need implementation and qualification.

The internal modules own agent-specific behavior. Durable records, shared skill-library revisions, connection refresh, and installation permissions remain orchestrator responsibilities. Shared provider adapters and communication types belong under `modules/`.
