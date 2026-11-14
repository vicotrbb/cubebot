# Security policy

## Supported versions

Only the latest release receives security fixes. `cubebot update` moves an installation to it.

## Reporting a vulnerability

Please do not open a public issue. Report it privately through [GitHub's private vulnerability reporting](https://github.com/vicotrbb/cubebot/security/advisories/new).

Include the affected version, the runtime (`kvm` or `container`), the steps to reproduce and the impact you observed.

You will get an acknowledgement within 3 days and an assessment within 7. Once a fix is released, the advisory is published, crediting you unless you ask otherwise.

## Scope

In scope: escaping an agent sandbox, reaching another agent's or account's data, reading vault secrets or provider credentials, bypassing authentication, grants or approvals, and code execution in the orchestrator.

Out of scope: running the orchestrator with the host's Docker socket, which is root on that machine by design (see the [README](README.md)); a `container` sandbox being weaker than `kvm`, which is declared; actions an administrator is allowed to take; and findings that need an already compromised host.
