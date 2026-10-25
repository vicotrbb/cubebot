# Deployment

Installation packaging and Kubernetes configuration belong here as they are implemented. Helm packaging is still pending.

Application image recipes live with their applications:

- [Orchestrator](../apps/orchestrator/Dockerfile)
- [Web console](../apps/web/Dockerfile)

Build both with the repository root as Docker context. The agent app owns its runtime and tool-runner executables, but no working agent images or pod specification exist yet. ADR 0033 still requires separate runtime and tool containers with independently enforced credentials, mounts, and execution authority.
