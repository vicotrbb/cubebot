# ADR 0078: Initial OpenAI, Anthropic, and compatible-endpoint support

Status: Accepted

Date: 2026-10-24

Decision: Target native OpenAI and Anthropic provider adapters, plus an adapter for configurable OpenAI-compatible endpoints, in the first release. The user selected this scope on 2026-10-24. Native Google Gemini support is outside the initial adapter scope. Cubebot retains its own Rust agent loop under ADR 0006.

Usage: An administrator configures a supported provider connection and grants access to selected agents. A manager chooses an available model through that connection. For a compatible hosted or local endpoint, the administrator configures its endpoint and supported authentication without assuming that every capability of the native OpenAI adapter is available.

Ownership: Provider adapters translate inference requests, streams, tool calls, usage, errors, and capability differences. The runtime owns context, tool execution, and turn progression. Installation administration owns connections and agent-use grants. Agents call providers directly under ADR 0031; the orchestrator owns refresh under ADR 0032.

API credential access and subscription-based authentication are separate qualification paths. Codex and Claude subscription connections remain desired integrations from the product direction. Their actual availability for the native Rust engine, account eligibility, authentication lifecycle, supported inference path, and deployment compatibility must be researched and demonstrated before support is claimed. This scope decision neither guarantees those paths nor replaces them silently with supervising external agent engines.

OpenAI-compatible means support for a documented and tested protocol subset, not universal compatibility with every endpoint using that description. Specify the supported request APIs and capability matrix during design and implementation. Validate streaming, tool calling, tool-result ordering, context limits, image inputs where needed, authentication, cancellation, and usage reporting for qualified endpoints. Unsupported required capabilities must produce a clear configuration or execution error rather than silently changing the requested behavior.

Endpoint configuration is installation-managed. Credentials must remain scoped to their configured destination, including during redirects or endpoint changes. A private local-model endpoint needs the appropriate deployment network path; configuring inference does not grant arbitrary tool programs access to that endpoint or its credentials. Exact validation and network enforcement remain design work.

Provider selection does not add automatic fallback. ADR 0030 retains the selected model and connection through bounded retry and visible waiting. Missing usage information must not be reported as zero, particularly when an optional cost limit is enabled. Connection-wide limits still apply across agents sharing an account.

This decision selects inference adapter scope. Exact models, SDK or HTTP implementation, protocol versions, embeddings for hybrid retrieval, and the room coordinator's configured model remain engineering proposals or qualification details. The specification must identify these explicitly rather than infer that every inference model provides every supporting capability.

Alternatives considered: Include a native Gemini adapter in the initial release as well. The user selected OpenAI, Anthropic, and compatible endpoints to concentrate initial implementation and qualification while retaining multi-provider access.

Open details: Adapter interfaces and concrete protocols; capability discovery and validation; initial tested models and compatible endpoints; subscription authentication feasibility; error classification; usage accounting; embeddings; and provider-specific recovery behavior.

Validation scenarios: Complete a tool-using turn through each native adapter and a qualified compatible endpoint; stream output and recover partial-stream failure; reject unsupported required capabilities; enforce connection grants; rotate credentials without disclosure; preserve selected-model waiting; and qualify each claimed subscription path separately. No provider or subscription integration has been executed by this ADR work.
