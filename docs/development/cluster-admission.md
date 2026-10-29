# Kubernetes admission compatibility fixture

Run `just cluster-admission` to verify Cubebot's prepared-manifest contract against an owned, disposable Kubernetes API server and PostgreSQL. This tests admission, defaulting and persistence. It does not start an agent, provision a workspace, execute tools, qualify a CSI driver or establish G1–G3.

## Dependencies and isolation

The runner requires Docker, kubectl and the checksum-verified kind v0.33.0 binary at `.tools/kind-v0.33.0`. Install the appropriate binary from the [official kind release](https://github.com/kubernetes-sigs/kind/releases/tag/v0.33.0) and verify its published checksum. The recorded macOS arm64 binary has SHA-256 `0c8c7dbe5e23594a198b786c4bc13dacc101fa6196b0cb0b23a1ca44e61f4b4f`. Existing global kind installations are not changed.

The node image is pinned to `kindest/node:v1.36.4@sha256:099e049362a1526b2db71494e1947aae99bd16290d7c895f2b7ea312e3cbfaed`, the Kubernetes 1.36 image published with that kind release. A UUID-named, single-node dual-stack cluster uses its own temporary kubeconfig and a loopback API endpoint. The runner refuses to reuse an existing cluster name. The Rust fixture also checks the explicit context, loopback endpoint, certificate validation and namespace ownership label before writing anything. Existing clusters and the default kubeconfig are not used.

Each namespace has its own synthetic installation database and ConfigMap anchor. The two namespaces enforce Pod Security `privileged` and `baseline` at v1.36. This changes only the disposable fixture namespaces. The test creates dedicated runtime service accounts, then sends the actual library's Pod and PVC server dry runs through `Registry::record_prepared`. The supplied image digests are schema fixtures and are never pulled or executed.

Prepared manifests are persisted twice from independent server previews. Direct API reads verify that no physical Pod or PVC was created. Under ADR 0084 the sandbox pod runs as root with `NET_ADMIN`, so it must be admitted by `privileged` and rejected by `baseline`, showing a visible admission failure rather than an unfiltered fallback. The fixture does not test a controller's running-Pod behavior under that rejection.

## Evidence

The [accepted report](evidence/2026-10-26-cluster-admission.json) records the successful run at 2026-10-29T17:04:27Z on baseline `32223f7` with the uncommitted fixture change, source fingerprint `224094af814b8eb8151223cbeb367a152c0165e1f56a94d618537f99b6df9a4f`. The fingerprint is captured before compilation, checked after compilation and checked again after the workflow. Changed source prevents accepting a report. A fresh run removes the previous success report before starting.

Observed profile: kind v0.33.0, Kubernetes v1.36.4, containerd 2.3.4, Linux arm64, Debian 13 node image and the recorded OrbStack kernel. The node advertised supplemental-groups policy support and both IPv4 and IPv6 Pod CIDRs. That advertisement does not prove container UID isolation or CNI enforcement. Preflight Docker capacity was three CPUs and 4,181,233,664 bytes of memory; this is not the production scale profile.

The real server exposed compatibility failures absent from the controlled HTTP fixtures:

- PVC admission appends `kubernetes.io/pvc-protection`. Preparation now permits that exact standard retention finalizer, retaining the Cubebot finalizer and exact checks for other metadata. Arbitrary, duplicate or replacement finalizers remain rejected.
- The Pod response omits ordinary false-valued `hostNetwork`, `hostPID`, `hostIPC` and PVC-volume `readOnly` fields. The compiler now relies on their standard false defaults instead of emitting redundant values that cannot survive the wire round trip.
- The server copies the `kvm` pod's extended-resource limits (`devices.kubevirt.io/kvm` and `tun`) into requests. The layout now states both, so the preview matches the intent.
- The server includes the deprecated `serviceAccount` alias. Preparation permits it only when it exactly matches the explicit `serviceAccountName`; a different account or enabled host networking remains rejected.

Focused regressions first reproduced both acceptance failures and passed after correction. `just check` and `just integration` verify the broader retained-specification, identity and lifecycle contracts. The raw public intent/preview artifacts remain under the run's ignored `target/cubebot-admission-*` directory for diagnosis; kubeconfigs and PostgreSQL data directories are removed.

## Cleanup and remaining work

The runner deletes only its generated cluster, verifies that its node containers are absent, and removes its disposable database container. Private kubeconfig and database data directories are temporary. Shared dependency images and Docker's shared kind network are retained. Cleanup failures are visible and cannot turn a failed workflow into success. Database cleanup attempts both container stop and data-owner restoration; regressions cover lost startup acknowledgement, simultaneous workflow/cleanup failure, and cleanup failure after a successful workflow.

Real agent startup with a suitable CSI backend, filesystem ownership, RWOP exclusion, network enforcement under the CNI, runtime identity, lifecycle recovery, partitions and physical fencing still need qualification. Finite turn authority and a trusted grant-update actuator also remain incomplete. The upstream CSI hostpath driver was researched as a possible compatibility fixture, but was not installed; its [documented test-only scope](https://github.com/kubernetes-csi/csi-driver-host-path/blob/v1.19.0/README.md) cannot establish production durability or fencing.
