# Deployment

Installation packaging and Kubernetes configuration belong here as they are implemented. Helm packaging is still pending.

Application image recipes live with their applications:

- [Orchestrator](../apps/orchestrator/Dockerfile)
- [Web console](../apps/web/Dockerfile)
- [Agent sandbox](../apps/sandbox/Dockerfile): `cubebot-sandbox` and `cubebot-agent`, one container per agent

Build all images with the repository root as Docker context. The fixed Pod/PVC compiler and owned controller are implemented, while full Kubernetes security/storage/fencing qualification remains pending. The sandbox image must be built with `--build-context guest=target/guest` after `tooling/guest/build.py`, and fails to build without it (ADR 0084); sandboxes run as root with only `CAP_NET_ADMIN`, so their namespace's Pod Security level must be privileged.
