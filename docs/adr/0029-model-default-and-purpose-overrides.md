# ADR 0029: Default agent model with optional purpose overrides

Status: Accepted

Date: 2026-10-23

Agents perform ordinary work and maintenance inference with different requirements. Selecting a separate model for every operation would complicate setup, while requiring one model for all purposes would prevent deliberate maintenance choices.

Decision: Each agent has one default model and optional model overrides for memory extraction, dreaming, and context compaction. An unspecified purpose inherits the default. The user selected this model on 2026-10-23. Authorized managers configure these choices from the installation-managed connections granted to the agent under ADR 0028.

Usage: A manager selects the agent's default model and starts using it without configuring maintenance separately. Later, the manager can assign a different extraction model while leaving dreaming and compaction on the default. Main-lane and task-lane work use the default; the agent does not autonomously select a different model from a list.

A model selection identifies both a provider connection and a model on that connection. An override may use another granted connection. Choosing an override does not grant new connection access or bypass budgets, provider limits, or data-access restrictions. The console should distinguish inherited settings from explicit overrides and show the effective selection for each purpose.

Ownership: Agent configuration owns the default and optional overrides. The engine resolves the effective model from the inference purpose through a small shared interface. Authorization validates use of the selected connection. Provider adapters translate requests and expose relevant capabilities; each consuming module defines its requirements. Memory and context modules should not independently invent selection rules.

Implementation direction: Represent the choice as a connection reference plus model identifier, and resolve it from agent configuration and inference purpose. Record the effective selection with the execution or maintenance record so users can inspect which model actually performed the work. Exact types, model-catalog discovery, and configuration-version handling remain open.

An absent override inherits the default. An explicitly configured but unavailable, unauthorized, or incompatible override is an execution problem, not an absent override. [ADR 0030](0030-retry-selected-model-without-fallback.md) selects bounded retries and visible waiting, with no automatic fallback to the default or another connection or model.

Purpose compatibility includes the input and output needed for the operation. For compaction, the summarizing model must be able to process the chosen material, and its output plus retained exchanges must fit the continuing work model's context budget. A cheaper maintenance model is an option, not an assumption of equal quality. Extraction, dreaming, and compaction require purpose-specific evaluation.

Alternatives considered: One model for all agent inference is simpler but prevents maintenance-specific tuning. Autonomous selection from an approved list adds inference-driven routing and another behavior to constrain and evaluate. The user selected deterministic configuration with inheritance and optional overrides.

Consequences: Initial configuration requires only the default. Overrides can change cost, latency, output quality, and the provider receiving the operation's context. Configuration changes need explicit activation semantics so running work does not observe an unexplained model switch.

Scope: This decision covers the agent's generative inference purposes. It does not select embedding models or the room coordinator's model, add per-task overrides, choose concrete providers, or define automatic fallback.

Open details: Capability validation; supported model catalog; configuration-change activation; provider-specific parameters; usage accounting; concrete model qualification; embedding selection; coordinator configuration; and detailed retry and recovery mechanics under ADR 0030.

Validation scenarios: All purposes inherit a sole default; an extraction override leaves other purposes unchanged; removing an override restores inheritance; an ungranted connection is rejected; an unavailable explicit override is not silently replaced; compaction respects both models' budgets; and execution records identify the model actually used.
