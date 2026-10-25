# ADR 0045: Explicit tool search with on-demand discovery

Status: Accepted

Date: 2026-10-23

An agent may have access to many integrations. Including every tool definition in every model request would consume context even when most tools are irrelevant to the current work. Agents need a way to find available capabilities themselves.

Decision: Expose a small core tool set and provide explicit tool search that remains available throughout execution. Agents search the eligible catalog and load relevant tool definitions on demand. The user selected this model and explicitly required tool search analogous to memory search on 2026-10-23.

Usage: An agent needs to find open issues for a project. It searches for that capability, receives bounded results identifying eligible tools and their configured connections, and selects the relevant tool. Cubebot makes its current definition available for invocation. The agent can refine its search when results are ambiguous or report that no eligible capability exists.

The analogy to [memory search](0023-tool-initiated-memory-retrieval.md) is explicit retrieval when needed. The tool catalog is configuration and capability metadata, not the agent's saved knowledge pool. This decision does not select the same index, embedding model, or ranking algorithm for both searches.

Ownership: The tool module owns catalog lookup and definition resolution across native tools and registered MCP integrations. Authorization determines eligibility and checks each invocation. The runtime controls which resolved definitions enter the execution lane's model context. MCP adapters obtain server capabilities under ADR 0040; an agent's catalog search does not itself register servers or create connections.

Search returns only tools eligible under the requesting agent's effective access, including the all-agents connection baseline or specific grants under [ADR 0081](0081-installation-wide-default-connection-access.md). Results must distinguish native tools, integrations, and account connections, including tools with identical names. Compact results provide enough description and scope information to choose a capability; resolving a tool supplies its current argument schema and relevant restrictions. Discovering or loading a definition never expands permissions, and a newly advertised server tool does not inherit approval merely through discovery. Broad connection availability does not load every schema into the model context.

Execution rechecks current authority and validates arguments against the applicable definition. Revoked grants, changed schemas, unavailable servers, or missing authentication produce explicit outcomes rather than silent account substitution or broader access. Descriptions and search results remain untrusted metadata and cannot change runtime instructions. If the catalog cannot be reached during disconnection, the runtime reports that limitation; previously resolved tools remain subject to ADR 0012's existing-authority limits.

Loaded definitions are bounded context material that can be released and resolved again. They are not automatically written into long-term memory. Tool search itself must remain accessible so an agent cannot lose the means to rediscover capabilities after compaction.

Alternative considered: Include all granted tool schemas upfront. That simplifies initial exposure but grows context with the installation's integrations. On-demand discovery adds a lookup step while keeping ordinary requests smaller.

Open details: Exact core set; search interface and ranking; catalog freshness; definition-loading mechanics for each provider; per-lane definition budgets and eviction; and discovery diagnostics.

Validation scenarios: Find and use an eligible tool absent from initial context; refine an ambiguous search; distinguish identical tool names and accounts; exclude ungranted tools; reject revoked authority at invocation; handle schema changes; report no match or unavailable discovery; and rediscover a released definition after compaction.
