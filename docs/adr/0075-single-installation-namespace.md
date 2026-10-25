# ADR 0075: One namespace for the entire installation

Status: Accepted

Date: 2026-10-24

Decision: Place the installation's namespaced Kubernetes resources in one dedicated namespace. This includes the orchestrator, frontend, bundled PostgreSQL, shared-storage claim, agent pods, and per-agent workspace claims. The user selected this layout on 2026-10-24. An externally managed PostgreSQL service retains its existing location under ADR 0063; cluster-scoped infrastructure such as storage classes and persistent volumes is outside the namespaced-resource layout.

Usage: An administrator installs Cubebot into a chosen namespace. The orchestrator provisions agent resources there and distinguishes them by stable installation and agent identity, recorded resource ownership, and managed labels. Creating agents, rooms, or teams does not create additional namespaces.

Ownership: Installation configuration selects the namespace. The Kubernetes lifecycle module reconciles managed agent resources within it under ADR 0074. Platform deployment owns the orchestrator, frontend, and bundled dependencies. The orchestrator must distinguish those resources from agent resources during reconciliation and cleanup. Merely sharing the namespace does not authorize adoption or deletion.

Keep platform and agent privileges distinct through service accounts, scoped RBAC, credential mounts, application authorization, and enforced network policies. Agent tools do not inherit the orchestrator's Kubernetes credentials or database access. Prefer namespace-scoped permissions for namespaced operations; any required cluster-level installation permissions must be separately identified. Exact RBAC rules remain engineering work.

Network rules must select the intended workloads and preserve ADR 0035's public-internet default and explicit grants for private destinations. Namespace membership must not implicitly allow agent-to-agent access or access to PostgreSQL, shared-storage mounts, or platform management endpoints. Required runtime-to-orchestrator communication still uses authenticated, authorized interfaces. NetworkPolicy enforcement requires a compatible cluster network implementation.

One namespace does not solve the runtime/tool distinction inside an agent pod. ADRs 0033 and 0035 retain that enforcement problem explicitly: the containers share a pod network, and ordinary pod-level policies alone cannot distinguish their callers. The concrete mechanism must be designed and qualified before claiming the intended isolation works.

Agents retain individual resource requests and limits and their own workspace claims. Namespace quotas, if configured, cover the shared namespace and therefore also affect platform workloads. Admission must account for platform capacity and report provisioning failures without losing accepted work. Exact capacity reservation and priority policies remain open.

Deleting the namespace affects the entire installation's namespaced resources. Sleeping, retiring, or removing an agent must never delete the namespace or unrelated platform resources. Persistent data cleanup, uninstall behavior, and restore follow explicit retention and backup policies rather than treating namespace deletion as ordinary agent lifecycle management.

Alternatives considered: Separate platform and agent namespaces; a platform namespace plus one namespace per agent. The user selected a single namespace to keep the installation layout simple. Per-agent permissions and network restrictions remain requirements within that layout.

Open details: Namespace naming and installation validation; resource labels and ownership records; RBAC and admission controls; network enforcement; quota and platform-capacity protection; workload objects; and uninstall and restore behavior.

Validation scenarios: Install into the selected namespace; create and replace agents without additional namespaces; prevent agents from accessing other workspaces and platform credentials; enforce network restrictions between selected workloads; retain platform operation under agent admission pressure; and remove one agent without changing unrelated resources. Implementation and cluster qualification remain pending.

Platform references: [Kubernetes namespaces](https://kubernetes.io/docs/concepts/overview/working-with-objects/namespaces/) and [NetworkPolicy](https://kubernetes.io/docs/concepts/services-networking/network-policies/), reviewed during the preceding architecture discussion. These describe platform mechanisms, not tested Cubebot behavior.
