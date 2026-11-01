# Deployment

Installation packaging and Kubernetes configuration belong here as they are implemented. Helm packaging is still pending.

Application image recipes live with their applications:

- [Orchestrator](../apps/orchestrator/Dockerfile)
- [Web console](../apps/web/Dockerfile)
- [Agent sandbox](../apps/sandbox/Dockerfile): `cubebot-sandbox` and `cubebot-agent`, one container per agent

Build all images with the repository root as Docker context. The fixed Pod/PVC compiler and owned controller are implemented, while full Kubernetes security/storage/fencing qualification remains pending. The sandbox image must be built with `--build-context guest=target/guest` after `tooling/guest/build.py`, and fails to build without it (ADR 0084); sandboxes run as root with only `CAP_NET_ADMIN`, so their namespace's Pod Security level must be privileged.

The orchestrator's shared files (ADR 0059) need a volume only the active orchestrator mounts, set as `CUBEBOT_STORAGE_DIRECTORY`: in compose the `shared-files` volume, on Kubernetes a PVC with access mode `ReadWriteOncePod` on a driver that supports it. The access mode does not replace the single-active-owner contract of ADR 0011. Staging and blobs share that one filesystem, since publication is a hard link.
