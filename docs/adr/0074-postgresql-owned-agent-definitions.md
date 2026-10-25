# ADR 0074: PostgreSQL owns agent definitions and lifecycle intent

Status: Accepted

Date: 2026-10-23

Decision: Store authoritative agent definitions and desired lifecycle state in PostgreSQL. Users and authorized agents manage them through Cubebot. The orchestrator reconciles those records into Kubernetes resources and records observed status. No custom Kubernetes Agent resource is required in the initial architecture. The user selected this model on 2026-10-23.

Usage: A user creates an agent through the console. The orchestrator validates permissions and configuration and durably records the definition. When lifecycle policy and pending work require execution, the lifecycle module provisions or reconciles the agent's Kubernetes resources. Delayed scheduling or failed provisioning appears as status on the existing agent; it does not lose the definition or falsely report a running agent.

Ownership: Agent-domain rules own definitions, grants, revisions, and lifecycle intent. Scheduling determines eligible work under those rules. The Kubernetes lifecycle module translates desired state into supported resource specifications, performs cluster operations, and observes their outcomes. Kubernetes remains authoritative about actual cluster resource state. PostgreSQL records that observation with enough identity and freshness information to distinguish it from desired state.

An agent has a stable Cubebot identity independent of a pod name, Kubernetes object UID, or runtime incarnation. Ordinary pod replacement and idle shutdown preserve agent identity, memory, history, pending work, and the persistent workspace under existing decisions. Running, sleeping, retired, and provisioning-blocked states must reflect their distinct domain meanings; absence of a pod alone does not mean an agent was deleted.

Accepting a definition and applying it to Kubernetes are separate operations. Persist desired changes before attempting their external effects. Track revision and operation outcomes so retries, lost acknowledgements, and process restarts can converge without creating competing executors or losing accepted intent. The exact reconciliation queue and Kubernetes workload type remain engineering choices. A PostgreSQL transaction cannot atomically commit a Kubernetes API operation.

On restart, observe existing managed resources and reconcile pending intent before replacing uncertain execution. A missing or failed pod is not proof that previously issued external actions failed. Replacement remains subject to exclusive execution ownership, disconnected-agent limits, and durable recovery under ADRs 0011–0012. Database unavailability must not cause the controller to interpret agents as deleted or perform destructive cleanup from an empty or incomplete read.

Manual edits to managed Kubernetes fields do not become accepted Cubebot configuration. Reconciliation should restore Cubebot-owned fields according to current intent, using the defined safe update procedure. Preserve fields owned by Kubernetes or approved admission machinery. Unexpected ownership conflicts must be visible. Exact field ownership, maintenance controls, and disruptive-change activation require design; direct cluster edits are not a second configuration interface.

Record and validate installation and agent ownership for managed resources. Do not adopt or delete resources solely because a convenient name or label matches. Sleeping, retiring, or replacing a pod does not authorize deleting its retained workspace. Resource cleanup and storage retention must follow their explicit policies, including after backup restoration.

Alternatives considered: A Kubernetes custom Agent resource could make agent definitions declarative through kubectl and GitOps, while PostgreSQL retained application history. The user selected Cubebot-managed definitions in PostgreSQL, keeping configuration, permissions, and lifecycle intent under one application authority. Supporting that alternative later would require an explicit ownership contract, not bidirectional synchronization by default.

Related decisions: ADRs 0008–0009 define durable state and orchestrator-mediated access; ADR 0010 places reconciliation in the single Rust backend process; ADR 0007 defines idle lifecycle; ADR 0018 defines task-bound retirement; ADRs 0064–0065 define coordinated backup and restore.

[ADR 0075](0075-single-installation-namespace.md) places the installation's namespaced resources in one dedicated namespace, with workload-specific permissions and network controls.

Open details: Kubernetes workload objects; resource ownership and field management; scoped RBAC; reconciliation scheduling and backoff; execution fencing; configuration activation; storage provisioning and cleanup; and restore handling for pre-existing resources.

Validation scenarios: Create while the Kubernetes API is unavailable; recover after creation succeeds but its response is lost; preserve an agent across pod replacement; converge after orchestrator restart; show pending and observed revisions honestly; handle manual drift without overwriting unrelated fields; refuse ambiguous resource adoption; and preserve workspaces and existing agents during database failure. Implementation and cluster qualification remain pending.
