# Deployment

Installation packaging and Kubernetes configuration belong here as they are implemented.

## Docker Compose

`install.sh` installs one machine's installation into `~/.cubebot` from a release: it checks for Docker Compose 2.23 or later, picks the `kvm` runtime on x86_64 Linux with `/dev/kvm` and `container` elsewhere, downloads the release's `compose*.yaml` and `cubebot`, writes `.env`, pulls, starts, waits for `/api/health/live` and prints the address, where the first account created becomes the administrator (`cubebot admin` claims it from the terminal instead). It never replaces an existing `.env`. `CUBEBOT_REGISTRY` and `CUBEBOT_SOURCE` point it at other images and files; `just install-check` uses them to install this checkout from a local registry. The site build copies it to `site/public/install.sh`.

`cubebot` runs the installation: `compose.yaml` always; `compose.domain.yaml` with `CUBEBOT_DOMAIN`; `compose.agents.yaml` unless `CUBEBOT_RUNTIME=off`, plus, without a domain, `compose.control.yaml`, the HTTPS listener on the Docker bridge (on macOS on loopback, reached as `host.docker.internal`) with a development control CA in `~/.cubebot/dev-ca`. Its sandboxes run with the terminal and the desktop (computer use, [Desktop as a VMM device](../docs/adr/sandbox.md#desktop-as-a-vmm-device)) on. In a checkout `just up` runs the same script with `compose.dev.yaml` added, which builds the orchestrator and web images instead of pulling them.

## Helm chart

`deploy/chart` installs one installation into one namespace ([One namespace for the entire installation](../docs/adr/packaging.md#one-namespace-for-the-entire-installation)): the orchestrator (one replica, `Recreate`), the web console, bundled PostgreSQL or an external database, the shared-files volume, an Ingress for `/api` and `/`, the sandboxes' NetworkPolicy and, for the `kvm` runtime, the device plugin. `values.yaml` documents every value; images must be pinned by digest, as a release's `digests.json` lists them.

```sh
kubectl create namespace cubebot
kubectl label namespace cubebot pod-security.kubernetes.io/enforce=baseline  # privileged for kvm
helm install cubebot deploy/chart --namespace cubebot --values my-values.yaml
kubectl -n cubebot exec -it deploy/orchestrator -- sh -c \
  'read -rs CUBEBOT_BOOTSTRAP_PASSWORD && export CUBEBOT_BOOTSTRAP_PASSWORD && CUBEBOT_BOOTSTRAP_LOGIN=admin cubebot-admin bootstrap'
```

The chart needs an ingress controller with TLS, and `sandbox.networkPolicy.ingressController` must select its pods: sandboxes reach the orchestrator only through it, at `controlOrigin`. `sandbox.runtime` is `container` by default; `kvm` needs nodes with `/dev/kvm` and a privileged namespace. The credential key, the database password Secret and both volumes outlive `helm uninstall`.

The chart generates the credential key and the bundled database password once, and reads them back with `lookup` on later upgrades. `lookup` finds nothing under Argo CD or any pipeline that renders with `helm template`, so there every sync would generate new ones: a new key cannot decrypt the stored provider and vault credentials, and a new password no longer matches the initialized database. Under those tools set both `credentials.existingSecret` (a Secret whose `key` holds 32 random bytes) and `postgres.existingSecret` (a Secret with `password` and `database_url`, as `postgres://cubebot:<password>@postgres:5432/cubebot`); the chart then emits neither Secret and uses the named ones. Flux's helm-controller and plain `helm install` and `helm upgrade` run `lookup` against the cluster and are unaffected. The orchestrator refuses to start with a key that cannot decrypt a stored credential, saying to restore the previous `CUBEBOT_CREDENTIAL_KEY_FILE`.

```sh
head -c 32 /dev/urandom > key && kubectl -n cubebot create secret generic cubebot-credentials --from-file=key && rm key
password=$(head -c 24 /dev/urandom | base64 | tr -d '/+=')
kubectl -n cubebot create secret generic cubebot-database --from-literal=password="$password" \
  --from-literal=database_url="postgres://cubebot:$password@postgres:5432/cubebot"
```

`tests/system/cluster_chat_setup.py` installs the chart into kind behind Traefik.

Application image recipes live with their applications:

- [Orchestrator](../apps/orchestrator/Dockerfile)
- [Web console](../apps/web/Dockerfile)
- [Agent sandbox](../apps/sandbox/Dockerfile): `cubebot-sandbox` and `cubebot-agent`, one container per agent

Build all images with the repository root as Docker context. The fixed Pod/PVC compiler and owned controller are implemented, while full Kubernetes security/storage/fencing qualification remains pending. The sandbox image must be built with `--build-context guest=target/guest` after `tooling/guest/build.py`, and fails to build without it ([In-house VMM as the agent sandbox](../docs/adr/sandbox.md#in-house-vmm-as-the-agent-sandbox)); sandboxes run as root with only `CAP_NET_ADMIN`, so their namespace's Pod Security level must be privileged.

The orchestrator's shared files ([Orchestrator-managed file storage on a persistent volume](../docs/adr/memory.md#orchestrator-managed-file-storage-on-a-persistent-volume)) need a volume only the active orchestrator mounts, set as `CUBEBOT_STORAGE_DIRECTORY`: in compose the `shared-files` volume, on Kubernetes a PVC with access mode `ReadWriteOncePod` on a driver that supports it. The access mode does not replace the single-active-owner contract of [One active orchestrator with automatic restart](../docs/adr/agents.md#one-active-orchestrator-with-automatic-restart). Staging and blobs share that one filesystem, since publication is a hard link.
