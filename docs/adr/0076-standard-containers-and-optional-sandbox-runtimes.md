# ADR 0076: Standard containers with optional sandbox runtimes

Status: Accepted

Date: 2026-10-24

Decision: Run agent pods on the cluster's standard container runtime by default. Support administrator-approved alternative sandbox runtimes through Kubernetes RuntimeClass selection. Installation policy may require a sandbox for agent execution. If a required or explicitly selected runtime is unavailable, block execution rather than silently falling back. The user selected this policy on 2026-10-24.

Usage: An individual installs Cubebot on a compatible cluster and starts agents using its default runtime. An organization with an approved sandbox runtime configures Cubebot to use it and can require that selection. If compatible nodes or the runtime are unavailable, accepted work remains durable and the console reports why its agent cannot start.

Ownership: The cluster administrator installs and configures node runtime handlers and RuntimeClass resources. Cubebot installation administration controls the approved runtime choices and requirements. The lifecycle module resolves the effective policy and applies it to agent pod specifications. Ordinary agent creation, software installation, and task execution do not authorize installing node runtimes, altering RuntimeClass resources, or weakening installation policy.

RuntimeClass selects the runtime for the pod's containers. The trusted Rust runtime and tool container remain in one agent pod under ADR 0033. Alternative runtimes must be qualified against that layout and the required browser, terminal, local MCP, storage, and networking behavior. No particular sandbox implementation is selected or certified by this decision.

The standard mode retains requirements for scoped permissions, separated credentials and mounts, resource limits, and enforced network controls. Optional sandboxing adds an isolation mechanism; it does not replace those requirements or prove that they work. In particular, it does not resolve the distinction between trusted runtime traffic and arbitrary tool traffic inside the shared pod network under ADR 0035.

Record the requested runtime policy and observed pod configuration, including a clear provisioning or compatibility failure. Check policy on creation and replacement. Runtime unavailability, pod failure, or a retry must not clear an explicit selection or downgrade a required sandbox. An installation policy change must not be displayed as applied to existing agents until the affected pods actually comply. The safe activation and replacement procedure remains open.

Keep agent identity, memory, history, and persistent workspace across a runtime change. Reconcile active execution and external effects before replacement. File persistence does not promise that installed software, live browser sessions, or running processes remain compatible with another runtime. Compatibility failures require visible recovery rather than silent workspace deletion.

RuntimeClass resources are cluster-scoped infrastructure. Referencing an administrator-provided class is consistent with ADR 0075's single namespace for Cubebot's namespaced resources and does not authorize Cubebot to manage the cluster's runtime configuration.

Alternatives considered: Require a sandbox runtime for all installations; support only standard containers initially. The user selected standard operation with optional, enforceable sandbox policy to preserve installation flexibility.

Open details: Supported runtime implementations and versions; installation-wide defaults and any per-agent selection interface; policy activation; node scheduling constraints; overhead accounting; compatibility diagnostics; and the qualification matrix. Support must be claimed per tested configuration rather than for every RuntimeClass implementation.

Validation scenarios: Start on a compatible default runtime; use an approved alternative; reject unapproved selection; block when a required runtime is unavailable; preserve the selected policy during retries and replacement; prevent agent-driven downgrades; show pending policy application honestly; and qualify browser, terminal, local MCP, volumes, network enforcement, and recovery under each supported mode. Implementation and runtime qualification remain pending.

Platform reference: [Kubernetes RuntimeClass](https://kubernetes.io/docs/concepts/containers/runtime-class/), reviewed on 2026-10-24, documents per-pod runtime selection, administrator-managed runtime configuration, scheduling constraints, and overhead accounting.
