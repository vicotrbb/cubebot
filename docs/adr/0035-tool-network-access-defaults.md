# ADR 0035: Public internet access with grants for private destinations

Status: Accepted

Date: 2026-10-23

Agents need public websites, package sources, and external tools to perform general-purpose work. They also run inside infrastructure that can contain private services and privileged endpoints. Network reachability needs a default consistent with autonomous software installation and explicit capability grants.

Decision: Agent tools have public internet access by default. Private destinations require explicit grants. The user selected this policy on 2026-10-23. Installation policy may impose stricter restrictions. This applies to browser requests, terminal commands, package downloads, and agent-installed programs, rather than only Cubebot's built-in HTTP tools.

Usage: An agent can research public documentation and download a project dependency without requesting permission for each destination. Connecting to an internal database or private package registry requires an appropriate network grant and, independently, any application credentials and permissions. Installation does not implicitly grant access to internal infrastructure.

Private and infrastructure destinations must be identified by the actual connection target and installation topology, not merely by the spelling of a requested URL. The policy must cover private and special-use addresses, cluster services, nodes, and infrastructure endpoints such as metadata services. Exact address classification, protocols, port scope, and treatment of infrastructure exposed on public addresses remain to be specified.

Enforcement must evaluate redirect targets and address resolution changes, including IPv4 and IPv6. A public hostname that resolves to a prohibited address must not bypass the private-access rule. Direct sockets, alternate resolvers, and tools that ignore proxy settings must be included in qualification. Setting an HTTP proxy environment variable alone does not establish enforcement for arbitrary programs.

Ownership: Authorization owns network grants and their scope. Deployment and tool-execution controls enforce the resulting policy. Tool interfaces report denied access clearly without granting additional authority. Network reachability does not authorize an application action, supply credentials, or override a read-only integration grant.

Runtime provider inference and required platform communication retain their separately authorized paths under ADRs 0009 and 0031. A runtime connection to the orchestrator is not a tool grant to access the orchestrator's privileged operations. The controlled runtime/tool interface under ADR 0033 remains available under its own authentication contract. Agent-created local development servers and browser access to them need an explicit local-access design; localhost is not a blanket permission to invoke runtime control endpoints.

The runtime and tool containers share a pod network under ADR 0033. Enforcement must preserve the required runtime paths while preventing arbitrary tool programs from using them to escape tool policy. The concrete mechanism remains an open architectural issue. This decision does not claim that ordinary pod-level rules alone distinguish these callers, and it does not silently change the selected one-pod layout.

During orchestrator disconnection, existing private-access grants may be used only within ADR 0012's finite authority and budgets. Agents cannot obtain new private-destination grants while disconnected. Revocation behavior for established connections must be specified together with execution cancellation and offline expiry.

Alternative considered: Require an explicit allowlist for every public destination. That restricts data movement more tightly but can block research and dependency installation on repeated configuration changes. The user selected public internet access by default with explicit private access.

Consequences: Public internet access allows outbound transmission as well as reading and downloading. This policy does not guarantee prevention of data exfiltration to public destinations or prevent use of a publicly reachable relay. Credential isolation, scoped data access, and any stricter installation restrictions remain relevant. Network policy is not a replacement for application authorization.

Open details: Enforcement mechanism compatible with the shared pod network; destination and protocol classification; grant schema; DNS and redirect handling; local development access; proxy and tunnel treatment; connection revocation; installation overrides; and denied-access observability.

Validation scenarios: Browse a public website; install from a public registry; deny an ungranted private endpoint; permit a specifically granted internal service without granting unrelated destinations; reject a public-to-private redirect and changed DNS resolution; apply the policy to installed programs and direct sockets; preserve required runtime communication; and expire private access during disconnected execution.
