# Deployment

Installation packaging and Kubernetes configuration belong here as they are implemented.

## Docker Compose

`install.sh` (bash 3.2 or later; `curl -fsSL https://cubebot.io/install.sh | bash`) installs one machine's installation into `~/.cubebot` from a release: it checks for curl, Docker and Compose 2.23 or later, finds the latest release, asks the questions declared in its `questions` table (where agents run, how the console is reached, the directory, whether to restore from a backup, the administrator; keys are read from `/dev/tty`, and a question whose `CUBEBOT_*` variable is set, every one under `CUBEBOT_YES=1` or without a terminal, takes its value or default), shows a summary, then downloads the release's `cubebot`, writes `.env` and hands off to `cubebot update`, which downloads the compose files, pulls, starts and waits for `/api/health/live`. A spinner follows each phase from the `Pulling release`, `Starting Cubebot` and `Waiting for` lines `cubebot` prints; the output is shown only when a phase fails. With the administrator created now it runs `cubebot admin`; otherwise the first account created at the console becomes the administrator. It never replaces an existing `.env`, and run again it updates the installation in `~/.cubebot` (or `CUBEBOT_HOME`). Colors, the cube and the spinners are off with `NO_COLOR`, `TERM=dumb`, output that is not a terminal or a terminal under 60 columns. `CUBEBOT_REGISTRY` and `CUBEBOT_SOURCE` point it at other images and files; `just install-check` uses them to install this checkout from a local registry. `tests/system/install_widgets.py` (in `just check`) drives its picker and confirmation through a pty. The site build copies it to `site/public/install.sh`.

`cubebot` runs the installation: `compose.yaml` always; `compose.domain.yaml` with `CUBEBOT_DOMAIN`; `compose.agents.yaml` unless `CUBEBOT_RUNTIME=off`, plus, without a domain, `compose.control.yaml`, the HTTPS listener on the Docker bridge (on macOS on loopback, reached as `host.docker.internal`) with a development control CA in `~/.cubebot/dev-ca`. Its sandboxes run with the terminal and the desktop (computer use, [Desktop as a VMM device](../docs/adr/sandbox.md#desktop-as-a-vmm-device)) on. In a checkout `just up` runs the same script with `compose.dev.yaml` added, which builds the orchestrator and web images instead of pulling them. `cubebot key` prints the credential key a restore needs ([Backup and restore](#backup-and-restore)). `cubebot uninstall` (confirmed by typing `uninstall`, or `--yes`/`CUBEBOT_YES=1`) removes the containers and workspace volumes of every agent the installation's database lists (or, when it cannot be read, of agents running this release's images), runs `docker compose down -v` with the files `up` uses, then removes the Cubebot images of every release in the registry it pulled from and the installation directory; in a checkout it refuses and points at `just down`.

Agents in Docker cannot reach each other or Cubebot's other containers directly, but this machine's private networks and cloud metadata stay reachable from them; only the Helm chart blocks those, where the cluster enforces NetworkPolicy ([reference](../docs/reference.md#docker-sandboxes-without-a-cluster)). `CUBEBOT_HOME` installs a separate installation with its own Compose project and data; give it its own `CUBEBOT_PORT`, and without a domain only one installation at a time can hold the agents' control port 8443.

## Helm chart

`deploy/chart` installs one installation into one namespace ([One namespace for the entire installation](../docs/adr/packaging.md#one-namespace-for-the-entire-installation)): the orchestrator (one replica, `Recreate`), the web console, bundled PostgreSQL or an external database, the shared-files volume, an Ingress or a Gateway API HTTPRoute for `/api` and `/`, network policies and, for the `kvm` runtime, the device plugin. `values.yaml` documents every value. Each release publishes the chart to GHCR with its image digests filled in:

```sh
kubectl create namespace cubebot
kubectl label namespace cubebot pod-security.kubernetes.io/enforce=baseline  # privileged for kvm
kubectl -n cubebot create secret generic cubebot-admin --from-literal=password="$(openssl rand -base64 24)"
helm install cubebot oci://ghcr.io/vicotrbb/charts/cubebot --version 0.2.8 --namespace cubebot --values my-values.yaml
```

```yaml
publicOrigin: https://cubebot.example.com
admin: { login: admin, existingSecret: cubebot-admin }
ingress:
  { className: traefik, host: cubebot.example.com, tlsSecret: cubebot-tls }
sandbox:
  dns: [10.96.0.10] # kubectl -n kube-system get service kube-dns
```

`helm install cubebot deploy/chart` from a checkout needs the four `images` set by digest, as a release's `digests.json` lists them. With `admin`, the orchestrator creates that administrator at its first start; without it, the first person to open the console becomes the administrator, so create it with `cubebot-admin bootstrap` (the chart's notes print the command) before the console is reachable by others. `installationName`, `storage.quotaBytes` and `uploads.maxBytes` (25 MiB) are defaults an administrator may change under Platform › General; single sign-on is set up under Platform › Sign-in.

### Reaching the console

The chart needs an ingress controller or a Gateway with TLS in front of `publicOrigin`; it does not install one. `ingress` writes an Ingress for any controller, and `httpRoute` (with `parentRefs`) a Gateway API HTTPRoute, whose `/api` rule turns the request timeout off for live updates, calls and the desktop. The console's live updates are server-sent events the orchestrator marks unbuffered, and calls and the desktop are WebSockets, so the proxy must keep long requests open:

- **Traefik** has no body limit. Its entrypoints close requests after 60 s by default; set `ports.websecure.transport.respondingTimeouts.readTimeout: 0` in its Helm values.
- **Envoy Gateway, Cilium, Istio** and other Gateway API implementations: use `httpRoute`.
- **NGINX-based controllers** (the retired ingress-nginx and its successors) refuse bodies over 1 MiB by default. Raise the limit to `uploads.maxBytes` and keep streams open with `ingress.annotations`, such as `nginx.ingress.kubernetes.io/proxy-body-size: 25m` and `nginx.ingress.kubernetes.io/proxy-read-timeout: "3600"`.

### Agents and the network

Agents never pass through the public ingress: each sandbox reaches the orchestrator at `orchestrator-control.<namespace>.svc.<clusterDomain>:8443`, a TLS listener whose certificate authority the chart creates once and keeps in the `control-tls` Secret; sandboxes see only its `ca.crt`. `clusterDomain` is `cluster.local` unless your cluster says otherwise. `sandbox.runtime` is `container` by default; `kvm` needs nodes with `/dev/kvm` and a privileged namespace. `sandbox.resources` replaces each agent's CPU and memory (requests 100m and 256Mi, or 2304Mi for `kvm` so a node never holds more guests than its memory; limits 2 CPUs and 3 GiB). An agent that fits on no node waits, shown in the console as waiting for room, instead of overcommitting one; a change replaces running agents once their work settles.

The `sandboxes` NetworkPolicy lets sandboxes reach cluster DNS, the orchestrator's control port and the internet, never a private address (`sandbox.networkPolicy.blockedRanges`) unless you grant one in `privateDestinations`, a `cidr` with optional TCP `ports`, such as the Home Assistant a connection uses. Public addresses are reachable on every TCP port; a private database, SFTP or MinIO server needs its own port granted (5432 for PostgreSQL, 3306 for MySQL, 22 for SFTP, 9000 for MinIO), and a server inside the cluster is a private address too. The `postgres` policy admits only the orchestrator, and the `orchestrator` policy admits only sandboxes to the control port.

These policies only work where the cluster's network plugin enforces NetworkPolicy. Calico, Cilium, Antrea, kube-router (k3s's default) and kindnet do; plain flannel does not, and EKS's VPC CNI and GKE do only once network policy is turned on. Without enforcement, agents' code can reach the cloud metadata address (and the node's cloud credentials), PostgreSQL and every other service in the cluster. Check it after installing:

```sh
kubectl -n cubebot run policy-check --rm -it --restart=Never --labels=cubebot.dev/agent=check \
  --image=busybox:1.37 -- sh -c 'nc -zw3 postgres 5432 && echo REACHABLE || echo blocked; wget -qT3 -O- http://169.254.169.254/ >/dev/null 2>&1 && echo REACHABLE || echo blocked'
```

Both lines must say `blocked`. Nodes with public addresses are outside `blockedRanges`, so add their addresses there; whether a policy governs traffic to a node's own addresses depends on the network plugin, so check the kubelet port (`nc -zw3 <node address> 10250`) the same way.

### Storage, secrets and Argo CD

Both volumes use `ReadWriteOncePod`, which needs a storage class whose driver supports it, as CSI drivers do; with one that does not, the volumes stay `Pending`. The credential key, the control certificate, the database password Secret and both volumes outlive `helm uninstall`. The orchestrator never applies a storage quota above 90% of the shared-files volume.

The chart generates the credential key, the bundled database password and the control certificate once, and reads them back with `lookup` on later upgrades. `lookup` finds nothing under Argo CD or any pipeline that renders with `helm template`, so there every sync would generate new ones: a new key cannot decrypt the stored provider, vault and single sign-on credentials, a new password no longer matches the initialized database, and a new certificate authority is not the one running agents trust. Under those tools set `credentials.existingSecret` (a Secret whose `key` holds 32 random bytes), `postgres.existingSecret` (a Secret with `password` and `database_url`, as `postgres://cubebot:<password>@postgres:5432/cubebot`) and `control.existingSecret` (a `kubernetes.io/tls` Secret with `tls.crt`, `tls.key` and `ca.crt` for `orchestrator-control.<namespace>.svc.<clusterDomain>`, such as one cert-manager issues from a fixed CA); the chart then emits none of them. Flux's helm-controller and plain `helm install` and `helm upgrade` run `lookup` against the cluster and are unaffected. The orchestrator refuses to start with a key that cannot decrypt a stored credential, saying to restore the previous `CUBEBOT_CREDENTIAL_KEY_FILE`.

```sh
head -c 32 /dev/urandom > key && kubectl -n cubebot create secret generic cubebot-credentials --from-file=key && rm key
password=$(head -c 24 /dev/urandom | base64 | tr -d '/+=')
kubectl -n cubebot create secret generic cubebot-database --from-literal=password="$password" \
  --from-literal=database_url="postgres://cubebot:$password@postgres:5432/cubebot"
```

### Pods, probes and resources

The orchestrator and the web console meet the `restricted` Pod Security profile: non-root, no privilege escalation, all capabilities dropped, a read-only root filesystem. Bundled PostgreSQL keeps its image's root entrypoint, which prepares the data directory and drops to the `postgres` user, without privilege escalation. The namespace itself stays `baseline` (or `privileged` for `kvm`) for the agents. The orchestrator's startup probe allows ten minutes for migrations and for proof that its predecessor stopped; its liveness probe never depends on the database, whose outage only takes it out of its Services. `orchestrator.resources`, `web.resources` and `postgres.resources` set requests and memory limits.

### Upgrading from 0.1.15

The chart no longer takes `controlOrigin` or `controlCA`; drop them from your values. After the upgrade, every running agent drains and restarts once onto the control port, keeping its workspace; until all have, keep `sandbox.networkPolicy.ingressController` so agents started before still reach the orchestrator through the ingress, then remove it. The orchestrator, the web console and PostgreSQL each restart once.

### Operations

- **A node that dies with the orchestrator.** A new orchestrator starts only after Kubernetes proves the old one stopped, so a pod on an unreachable node stays `Terminating` and the new one waits. Once you are sure the node is gone (powered off or deleted), `kubectl -n cubebot delete pod <old orchestrator pod> --force --grace-period=0`; a force delete while the old one still runs breaks the one-orchestrator guarantee.
- **Backups.** See [Backup and restore](#backup-and-restore); keep the `credentials` Secret apart from the bucket.
- **Uninstalling.** `helm uninstall` leaves agent pods and workspaces, which the orchestrator made, and the kept Secrets and volumes. Their `cubebot.dev/*` finalizers keep them until the orchestrator releases them, so after uninstalling, `kubectl delete namespace cubebot` waits until you remove those finalizers (`kubectl -n cubebot patch <object> --type=merge -p '{"metadata":{"finalizers":null}}'` on the agent pods, workspace claims and the anchor ConfigMap).
- **Rotating the control certificate.** Keep its certificate authority: agents trust only the one they started with. After replacing the CA itself, delete the agent pods (`kubectl -n cubebot delete pod -l cubebot.dev/agent`) so they start again with the new one.
- **Logins.** Built-in passwords are limited per account (ten tries a minute); limit requests per client address at your ingress controller or Gateway.

## Backup and restore

Cubebot backs itself up to any S3-compatible bucket (AWS S3, Cloudflare R2, Backblaze B2, MinIO) without pausing anything ([Backups without pauses](../docs/adr/packaging.md#backups-without-pauses)): `pg_dump` reads one consistent snapshot of the database, shared files go up once each, and every running agent streams its workspace through the orchestrator at low priority. The parts of one backup can differ by minutes; a stopped agent's workspace is listed as missing. A backup is verified before it counts: its manifest is read back, every part is found at its size and a sample is re-read to compare checksums. Give each installation a bucket of its own.

Backups are **not encrypted**: whoever can read the bucket can read the conversations, files and workspaces in it, so protect it with its own access policy. The credential key that encrypts stored credentials is never in a backup, and a restore cannot start without it: Platform › Backup offers administrators one audited download, `cubebot key` prints it on a Compose machine, and on Kubernetes it is the `credentials` Secret (or your `credentials.existingSecret`).

Settings: bucket (`endpoint`, `region`, `bucket`, access key), `schedule` (`daily` by default, or `off`; Back up now always works), retention (`keepDaily` 7 and `keepWeekly` 4: each of the newest days and weeks keeps its newest backup), `bandwidthBytes` (20 MiB a second) and `concurrency` (2 transfers). Each can be set in Helm values under `backup:`, in `.env` as `CUBEBOT_BACKUP_*`, or on Platform › Backup; a value set by the deployment shows there read-only, with its source. The orchestrator image carries the PostgreSQL 18 client, so an external database must be PostgreSQL 18 or older.

```yaml
backup:
  endpoint: http://minio.storage:9000
  bucket: cubebot-backups
  existingSecret: cubebot-backup-bucket # keys access-key-id and secret-access-key
```

```sh
# .env on a Compose machine; `cubebot up` applies it.
CUBEBOT_BACKUP_ENDPOINT=http://minio.lan:9000
CUBEBOT_BACKUP_BUCKET=cubebot-backups
CUBEBOT_BACKUP_ACCESS_KEY_ID=...
CUBEBOT_BACKUP_SECRET_ACCESS_KEY=...
```

Restore happens only into a **fresh** installation: when the orchestrator first starts on an empty database with a restore source set, it restores the newest verified backup (or the one named) before anything else runs. It checks the credential key first and stops at once if it is wrong, then loads the database and the shared files; meanwhile the console shows only the restore's progress, and a failure stops it with the reason. Each agent then fetches its own workspace before it takes work, and Platform › Backup lists each one. Connections and sign-ins come back as they were, but their external tokens may have expired. Once the installation holds data the restore source is ignored, so it can stay in place.

- **Compose:** `install.sh` asks "Restore from a backup?", then for the bucket, its key, which backup and the line `cubebot key` printed on the old machine, and writes them to `.env` as `CUBEBOT_RESTORE_*` and `CUBEBOT_RESTORE_KEY`.
- **Helm** (Argo CD included): give the old key as `credentials.existingSecret` and set `restore:`. The chart refuses `restore.enabled` without `credentials.existingSecret`.

```sh
kubectl -n cubebot create secret generic cubebot-credentials --from-file=key=<(base64 -d < cubebot-credential-key.txt)
kubectl -n cubebot create secret generic cubebot-restore-bucket --from-literal=access-key-id=... --from-literal=secret-access-key=...
```

```yaml
credentials: { existingSecret: cubebot-credentials }
restore:
  enabled: true
  endpoint: http://minio.storage:9000
  bucket: cubebot-backups
  existingSecret: cubebot-restore-bucket
  backup: '' # newest verified when empty, or an id such as 20261107T030000Z
```

`tests/system/backup_check.py` (`just backup-check`) backs up Compose and kind installations to MinIO while an agent works, then restores each into a fresh installation and checks the data and workspace files came back.

`tests/system/cluster_chat_setup.py` installs the chart into kind behind Traefik.

Application image recipes live with their applications:

- [Orchestrator](../apps/orchestrator/Dockerfile)
- [Web console](../apps/web/Dockerfile)
- [Agent sandbox](../apps/sandbox/Dockerfile): `cubebot-sandbox` and `cubebot-agent`, one container per agent

Build all images with the repository root as Docker context. The fixed Pod/PVC compiler and owned controller are implemented, while full Kubernetes security/storage/fencing qualification remains pending. The sandbox image must be built with `--build-context guest=target/guest` after `tooling/guest/build.py`, and fails to build without it ([In-house VMM as the agent sandbox](../docs/adr/sandbox.md#in-house-vmm-as-the-agent-sandbox)); sandboxes run as root with only `CAP_NET_ADMIN`, so their namespace's Pod Security level must be privileged.

The orchestrator's shared files ([Orchestrator-managed file storage on a persistent volume](../docs/adr/memory.md#orchestrator-managed-file-storage-on-a-persistent-volume)) need a volume only the active orchestrator mounts, set as `CUBEBOT_STORAGE_DIRECTORY`: in compose the `shared-files` volume, on Kubernetes a PVC with access mode `ReadWriteOncePod` on a driver that supports it. The access mode does not replace the single-active-owner contract of [One active orchestrator with automatic restart](../docs/adr/agents.md#one-active-orchestrator-with-automatic-restart). Staging and blobs share that one filesystem, since publication is a hard link.
