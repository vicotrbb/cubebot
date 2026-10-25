# ADR 0044: Tool allowlists with supported resource restrictions

Status: Accepted

Date: 2026-10-23

Permission to invoke a tool does not necessarily authorize every repository, project, mailbox, or other resource that the connected account can access. Cubebot needs a precise contract for narrowing integration authority without claiming unsupported enforcement.

Decision: Combine named tool allowlists with Cubebot-enforced resource restrictions where an integration supports them, alongside external credential scopes. The user selected this model on 2026-10-23. Each integration must expose which restrictions Cubebot can actually enforce. Unsupported restrictions require an adequately scoped external credential or configuration rejection; they must not be represented as enforced prompt instructions.

Usage: An administrator grants an agent an issue-reading tool on a selected account connection, restricted to two repositories. Cubebot checks the intended resource before dispatch. A request for a third repository is denied even if the connected account could otherwise access it. If the integration cannot enforce repository selection, that narrow policy requires external credentials or server configuration that demonstrably limit access to those repositories.

Ownership: Authorization owns grant evaluation and the intersection of applicable restrictions. An integration-specific adapter or qualified policy mapping identifies resources and implements supported checks. Tool execution invokes those checks before dispatch and records the applied policy and outcome. The generic MCP adapter does not infer authorization semantics from parameter names or model judgments.

Grant identity includes the agent or applicable execution scope, registered integration, selected account connection, permitted tools, and supported resource restrictions. [ADR 0081](0081-installation-wide-default-connection-access.md) adds the connection's all-agents baseline as a source of use authority. Its approved tool and resource scope applies without separate per-agent assignment; specific-grants mode requires explicit grants within that scope. Managers may assign only within their delegated authority under ADR 0042. A resource restriction cannot authorize an unlisted tool, another account, a private network destination, or a capability missing from the external credential.

Restriction support must describe its limits. Checks must account for relevant aliases, default resources, indirect identifiers, bulk operations, and caller-supplied URLs before claiming enforcement. Read operations that can return multiple resources need an appropriately scoped query or upstream enforcement; silently discarding unauthorized results after broad retrieval does not establish that the data was never accessed. Exact resource identifiers, schema, and qualification cases remain integration-specific.

An MCP tool description or read-only annotation is not proof of a restriction. A schema change that invalidates a policy mapping must not silently broaden access. Unknown or unresolved resource identity must not pass a restrictive grant by default. Behavior for changed capabilities and configuration revisions must be explicit and tested.

The restriction must hold across the execution paths available to the agent. A check on an MCP call is insufficient if the agent can use the same broad credential through an unrestricted terminal program, browser session, or alternate endpoint. Qualification must account for credential placement and reachable bypass paths. Use narrower upstream credentials, an enforcing execution path, or reject a claimed scope that the deployment cannot uphold. This decision does not assume that a generic proxy can enforce every external service's semantics.

Within a valid grant, execution remains autonomous under ADR 0013. There is no per-call approval queue. During orchestrator disconnection, resource checks may use only valid existing authorization and sufficient trusted state within ADR 0012's limits; missing required state does not permit a broader operation.

Alternative considered: Allowlist named tools and rely entirely on external credential scopes for resource limits. That is simpler but cannot add narrower Cubebot policy when credentials are broader. The user selected additional resource restrictions where enforceable, retaining upstream scopes as a complementary control.

Consequences: Fine-grained restrictions require integration-specific support and validation. Arbitrary MCP servers can still be used within their honestly represented authority, but cannot claim unsupported resource controls. The console must distinguish Cubebot-enforced restrictions, external restrictions, and unavailable controls.

Open details: Grant schema; resource identity and normalization; supported integration mappings; capability discovery and validation; delegation representation; policy activation and revocation; credential placement; bypass-resistant execution paths; and denial diagnostics.

Validation scenarios: Permit an allowed tool and resource; deny an unlisted tool or resource; reject an unsupported narrow policy; prevent account substitution; handle aliases and implicit defaults; reject stale schema mappings; verify bulk-read scope; and demonstrate that alternate tool paths cannot reuse broader credentials to bypass a claimed restriction.
