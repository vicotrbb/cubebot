# Agent application

One Rust package, one executable: `cubebot-agent` (`src/main.rs`). It runs inside `apps/sandbox`'s `cubebot-sandbox` (ADR 0084), in the guest VM with the `kvm` backend or as a plain child process with the development `process` backend. The agent runs its own tools in that sandbox; there is no separate tool runner.

Normal runtime execution runs authenticated control and bounded text inference, synchronizes typed recovery evidence and acknowledges drain after work settles. It accepts scoped grants for the selected OpenAI model, journals intent before inference and outcomes before delivery, and retries an explicit transient provider response once within the original authority. It never resumes inference from a recovered local intent. See [text-chat setup and results](../../docs/development/text-chat-mvp.md). The terminal loop is gated off by default; see [terminal execution](../../docs/development/bounded-terminal.md). Browser and MCP execution remain unavailable.

It requires `CUBEBOT_ORCHESTRATOR_ORIGIN`, a bootstrap token (`CUBEBOT_BOOTSTRAP_TOKEN_FILE` on Kubernetes, `CUBEBOT_BOOTSTRAP_TOKEN` in Docker sandboxes) and a private `CUBEBOT_JOURNAL_DIRECTORY` (the sandbox creates it with mode 0700). Terminal commands also need `CUBEBOT_WORKSPACE_DIRECTORY`.

The internal modules own agent-specific behavior. Durable records, shared skill-library revisions, connection refresh, and installation permissions remain orchestrator responsibilities. Shared provider adapters and communication types belong under `modules/`.
