# Deployment

Installation packaging and Kubernetes configuration belong here as they are implemented. Helm packaging is still pending.

Application image recipes live with their applications:

- [Orchestrator](../apps/orchestrator/Dockerfile)
- [Web console](../apps/web/Dockerfile)
- [Agent runtime and tool runner](../apps/agent/Dockerfile), selected with `--target runtime` or `--target tools`

Build all images with the repository root as Docker context. The agent runtime image runs the native identity/control/recovery loop; the separate tool-runner executable remains a placeholder. The fixed Pod/PVC compiler and owned controller are implemented, while full Kubernetes security/storage/fencing qualification remains pending. The agent Dockerfile's `qualification` target supports [disposable Linux isolation mechanism probes](../docs/development/container-isolation.md); it does not enable production network enforcement. ADR 0033 still requires independently enforced runtime/tool credentials, mounts and execution authority.
