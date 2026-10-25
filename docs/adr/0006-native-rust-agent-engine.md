# ADR 0006: Native Rust agent engine

Status: Accepted

Date: 2026-10-22

Cubebot needs multi-provider inference, unified agent memory, and the execution lanes defined in [ADR 0005](0005-agent-execution-lanes.md). The engine model determines who owns reasoning, tool execution, context construction, and recovery.

Decision: Build Cubebot's own Rust agent engine around model/provider adapters. The user selected this approach on 2026-10-22. Cubebot owns the agent loop; provider adapters supply inference. Supervising external agent engines such as Codex or Claude Code is outside the selected architecture.

Usage: A lane supplies a conversation and work item to the engine. The engine constructs context, requests a model response, handles requested tools through Cubebot's execution rules, incorporates results, and continues until completion, a limit, cancellation, or a defined pause. Results and progress retain their agent, conversation, task, and turn identities.

The model adapter interface handles request and response translation, streaming, model capabilities, authentication integration, usage reporting where available, and provider errors. It must preserve distinctions callers need for correct execution instead of pretending all providers behave identically. [ADR 0078](0078-initial-provider-support.md) selects native OpenAI and Anthropic adapters plus configurable OpenAI-compatible endpoints for the first release. Exact types, models, protocol subsets, and authentication qualification remain open.

The engine owns turn progression and context assembly. Memory owns retrieval and validated memory changes. Tool execution owns authorization and invocation. Scheduling owns lane admission and budgets. These are logical module responsibilities, not a requirement for separate processes, crates, or a generic plugin framework. Keep each interface small and introduce abstractions only for demonstrated variation.

Consequences: Context maintenance, checkpoints, tool behavior, and memory integration are Cubebot responsibilities across providers. Each execution lane has its own working context while retaining the agent's shared memory and workspace. Provider-specific inference and account limits still constrain admission. Native sessions, child agents, or compaction behavior of external agent engines are not assumed capabilities.

Alternatives considered: Supervising existing engines would reuse their reasoning and tool loops but delegate control over important runtime behavior. Supporting both native and external engines would require two execution paths and different capability contracts. The user chose the native engine alone.

Subscription authentication remains a desired integration where the provider supports the selected deployment and inference path. This decision does not establish access to any particular subscription, and authentication support must be verified separately. [ADR 0028](0028-installation-managed-provider-connections.md) makes provider connections installation-managed, with explicit agent-use grants and no separate personal-connection ownership category. It also does not decide whether to use a Rust library internally for inference or agent-loop utilities.

Validation scenarios: A tool call followed by a final response; multiple lanes with separate working contexts and shared agent memory; provider stream failure; unavailable capabilities; cancellation during a tool operation; context pressure; and restart after an action whose outcome is uncertain. Detailed recovery and acceptance criteria remain open.
