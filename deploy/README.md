# Deployment

Installation packaging and Kubernetes configuration belong here as they are implemented.

## Docker Compose

`install.sh` (bash 3.2 or later; `curl -fsSL https://cubebot.io/install.sh | bash`) installs one machine's installation into `~/.cubebot` from a release: it checks for curl, Docker and Compose 2.23 or later, finds the latest release, asks the questions declared in its `questions` table (where agents run, how the console is reached, the directory, the administrator; keys are read from `/dev/tty`, and a question whose `CUBEBOT_*` variable is set, every one under `CUBEBOT_YES=1` or without a terminal, takes its value or default), shows a summary, then downloads the release's `cubebot`, writes `.env` and hands off to `cubebot update`, which downloads the compose files, pulls, starts and waits for `/api/health/live`. A spinner follows each phase from the `Pulling release`, `Starting Cubebot` and `Waiting for` lines `cubebot` prints; the output is shown only when a phase fails. With the administrator created now it runs `cubebot admin`; otherwise the first account created at the console becomes the administrator. It never replaces an existing `.env`, and run again it updates the installation in `~/.cubebot` (or `CUBEBOT_HOME`). Colors, the cube and the spinners are off with `NO_COLOR`, `TERM=dumb`, output that is not a terminal or a terminal under 60 columns. `CUBEBOT_REGISTRY` and `CUBEBOT_SOURCE` point it at other images and files; `just install-check` uses them to install this checkout from a local registry. `tests/system/install_widgets.py` (in `just check`) drives its picker and confirmation through a pty. The site build copies it to `site/public/install.sh`.

`cubebot` runs the installation: `compose.yaml` always; `compose.domain.yaml` with `CUBEBOT_DOMAIN`; `compose.agents.yaml` unless `CUBEBOT_RUNTIME=off`, plus, without a domain, `compose.control.yaml`, the HTTPS listener on the Docker bridge (on macOS on loopback, reached as `host.docker.internal`) with a development control CA in `~/.cubebot/dev-ca`. Its sandboxes run with the terminal and the desktop (computer use, [Desktop as a VMM device](../docs/adr/sandbox.md#desktop-as-a-vmm-device)) on. In a checkout `just up` runs the same script with `compose.dev.yaml` added, which builds the orchestrator and web images instead of pulling them. `cubebot uninstall` (confirmed by typing `uninstall`, or `--yes`/`CUBEBOT_YES=1`) stops the agents, runs `docker compose down -v` with the files `up` uses, removes the agent containers and volumes that run this release's images, the release's Cubebot images and the installation directory; in a checkout it refuses and points at `just down`.

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
