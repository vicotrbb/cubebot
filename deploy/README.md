# Deployment

Installation packaging and Kubernetes configuration belong here as they are implemented.

## Helm chart

`deploy/chart` installs one installation into one namespace (ADR 0075): the orchestrator (one replica, `Recreate`), the web console, bundled PostgreSQL or an external database, the shared-files volume, an Ingress for `/api` and `/`, the sandboxes' NetworkPolicy and, for the `kvm` runtime, the device plugin. `values.yaml` documents every value; images must be pinned by digest, as a release's `digests.json` lists them.

```sh
kubectl create namespace cubebot
kubectl label namespace cubebot pod-security.kubernetes.io/enforce=baseline  # privileged for kvm
helm install cubebot deploy/chart --namespace cubebot --values my-values.yaml
kubectl -n cubebot exec -it deploy/orchestrator -- sh -c \
  'read -rs CUBEBOT_BOOTSTRAP_PASSWORD && export CUBEBOT_BOOTSTRAP_PASSWORD && CUBEBOT_BOOTSTRAP_LOGIN=admin cubebot-admin bootstrap'
```

The chart needs an ingress controller with TLS, and `sandbox.networkPolicy.ingressController` must select its pods: sandboxes reach the orchestrator only through it, at `controlOrigin`. `sandbox.runtime` is `container` by default; `kvm` needs nodes with `/dev/kvm` and a privileged namespace. The credential key, the database password Secret and both volumes outlive `helm uninstall`. `tests/system/cluster_chat_setup.py` installs the chart into kind behind Traefik.

Application image recipes live with their applications:

- [Orchestrator](../apps/orchestrator/Dockerfile)
- [Web console](../apps/web/Dockerfile)
- [Agent sandbox](../apps/sandbox/Dockerfile): `cubebot-sandbox` and `cubebot-agent`, one container per agent

Build all images with the repository root as Docker context. The fixed Pod/PVC compiler and owned controller are implemented, while full Kubernetes security/storage/fencing qualification remains pending. The sandbox image must be built with `--build-context guest=target/guest` after `tooling/guest/build.py`, and fails to build without it (ADR 0084); sandboxes run as root with only `CAP_NET_ADMIN`, so their namespace's Pod Security level must be privileged.

The orchestrator's shared files (ADR 0059) need a volume only the active orchestrator mounts, set as `CUBEBOT_STORAGE_DIRECTORY`: in compose the `shared-files` volume, on Kubernetes a PVC with access mode `ReadWriteOncePod` on a driver that supports it. The access mode does not replace the single-active-owner contract of ADR 0011. Staging and blobs share that one filesystem, since publication is a hard link.
