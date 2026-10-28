# Agent application

One Rust package, one executable: `cubebot-agent` (`src/main.rs`). It runs inside `apps/sandbox`'s `cubebot-sandbox` (ADR 0084), in the guest VM with the `kvm` backend or as a plain child process with the development `process` backend. The agent runs its own tools in that sandbox; there is no separate tool runner.

Normal runtime execution runs authenticated control and bounded text inference, synchronizes typed recovery evidence and acknowledges drain after work settles. It accepts scoped grants for the selected OpenAI or Anthropic model, streams provider output, compacts its context at 80% of the model window, and retries explicit transient provider responses with bounded backoff on the same model. It never resumes inference from a recovered local intent. See [text-chat setup and results](../../docs/development/text-chat-mvp.md). Tools, including MCP server tools, are gated off by default; see [tool execution](../../docs/development/bounded-terminal.md). Browser execution remains unavailable.

It requires `CUBEBOT_ORCHESTRATOR_ORIGIN`, a bootstrap token (`CUBEBOT_BOOTSTRAP_TOKEN_FILE` on Kubernetes, `CUBEBOT_BOOTSTRAP_TOKEN` in Docker sandboxes) and a private `CUBEBOT_JOURNAL_DIRECTORY` (the sandbox creates it with mode 0700). Tools also need `CUBEBOT_WORKSPACE_DIRECTORY` and `CUBEBOT_TOOL_USER`; MCP servers are read from `CUBEBOT_MCP_SERVERS`.

The internal modules own agent-specific behavior. Durable records, shared skill-library revisions, connection refresh, and installation permissions remain orchestrator responsibilities. Shared provider adapters and communication types belong under `modules/`.
