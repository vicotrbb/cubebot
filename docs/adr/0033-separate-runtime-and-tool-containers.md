# ADR 0033: Separate runtime and tool containers within each agent pod

Status: Accepted

Date: 2026-10-23

The native agent runtime holds provider authentication and Cubebot execution authority. Agent-directed commands and browser workloads need workspace access without inheriting that authority. The deployment model calls for one running pod per agent.

Decision: Separate the trusted Rust runtime and tool execution into distinct containers within each agent pod. The user selected this layout on 2026-10-23. The baseline pod has a runtime container and a tool container for terminal commands, browser workloads, and agent-directed programs. They remain one agent with one identity and persistent workspace.

The runtime container owns the model/tool loop, orchestrator communication, provider calls, supplied provider credentials, and execution recovery state. It receives authentication updates under ADRs 0031 and 0032. Agent-directed shell commands and arbitrary workspace programs execute in the tool container, not in the credential-holding runtime container.

The tool container accesses the agent's persistent workspace and executes authorized requests through a controlled interface. It receives only the authority and integration credentials explicitly required by its granted operations. Provider inference credentials and the runtime's platform identity must not be exposed through its filesystem mounts, command environments, logs, or model-visible results. A terminal grant does not authorize reading runtime secrets.

Usage: The runtime receives a model-requested command, checks the current execution grant and budgets, and submits a bounded invocation to the tool executor. The executor runs it in the workspace and returns attributed output and outcome. The runtime records progress and supplies the appropriate result to the model. The tool executor does not independently create agent turns or gain access to every orchestrator operation.

Ownership: The runtime's tool-execution module owns invocation authorization and execution identity. The tool executor owns process and browser lifecycle under the supplied limits. Orchestrator lifecycle management owns the pod specification, mounts, container resources, and recovery coordination. Transport and wire types remain open; callers should use one tool-execution interface rather than manage container lifecycle themselves.

Container separation must be enforced by deployment and execution policy. Keep runtime credentials and recovery journals outside tool-accessible volumes. Do not load executable runtime code from the writable agent workspace. Avoid shared process namespaces and privilege paths that let a tool inspect or control the runtime. The exact security context, permitted mounts, and sandbox controls remain to be specified and tested.

Containers in the same pod share networking. The internal interface must therefore authenticate requests and constrain operations rather than trust localhost. Container separation alone does not provide independent pod-level network policy or a complete sandbox for hostile code. [ADR 0035](0035-tool-network-access-defaults.md) selects public internet access by default for tools and grants for private destinations. [ADR 0076](0076-standard-containers-and-optional-sandbox-runtimes.md) selects standard containers by default with optional administrator-approved sandbox runtimes and enforceable installation requirements. Enforcement within the shared pod network remains open.

The agent's workspace remains shared across its execution lanes under ADR 0005. This decision creates no per-task container, workspace isolation, or additional agent identity. Browser and concurrent workspace coordination still require their own rules. The runtime may access selected workspace content through the tool interface or deliberate mounts; exact mount direction and permissions remain open. Workspace content is untrusted input in either case.

Container restarts must reconcile active tool invocations and runtime records before replaying work. A runtime restart does not prove a command stopped, and a tool-container failure does not prove an external action failed. Persistent files survive according to the workspace contract; process and browser state are not promised to survive replacement.

Alternatives considered: A single container with internal process isolation reduces container count but places more isolation work inside Cubebot. Separate runtime and tool pods allow independent networking and scheduling but add lifecycle and storage coordination and change the chosen pod model. The user selected separate containers within one pod.

Consequences: Cubebot maintains a tool-execution protocol and separate container images or image roles. Runtime and tool resources can be configured separately while sharing pod placement and lifecycle constraints. Credential isolation must be demonstrated against actual tool behavior, not inferred from container count.

Related decisions: [ADR 0034](0034-agent-software-installation.md) allows agent-directed software installation in writable workspace or user environments within granted permissions. The managed base image and runtime remain outside that installation authority. [ADR 0036](0036-structured-and-visual-browser-interaction.md) selects structured browser tools and visual actions within the same session in the tool environment.

MCP execution: [ADR 0041](0041-local-and-remote-mcp-servers.md) places local third-party MCP subprocesses in the tool container. They do not inherit the runtime's provider credentials or platform identity.

Open details: Tool-execution transport and authentication; workspace mounts; container images and dependency persistence; security contexts; network enforcement; resource allocation; browser sessions; background process management; and restart reconciliation.

Validation scenarios: A command runs only in the tool container; runtime secrets and journals are inaccessible from that command; a tool cannot impersonate the runtime to the orchestrator; unauthorized local requests are rejected; workspace changes persist across replacement; concurrent lanes obey existing limits; and restart during a tool invocation does not blindly repeat its effects.

Platform references reviewed during design: [Kubernetes pod networking](https://kubernetes.io/docs/concepts/workloads/pods/#pod-networking) and [restricting a Secret to one container](https://kubernetes.io/docs/concepts/configuration/secret/#use-case-secret-visible-to-one-container-in-a-pod). These document platform capabilities, not verification of Cubebot's future implementation.
