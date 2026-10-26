# CSI-backed startup compatibility fixture

`just cluster-startup` builds the shipped runtime and tool images, loads them into an owned disposable Kubernetes 1.36.4 cluster, and runs the real Pod compiler and reconciler against a CSI-backed workspace. It uses the same explicit kubeconfig, loopback endpoint, ownership checks and source-fingerprint guard as the [admission fixture](cluster-admission.md).

This is a startup and clean-replacement compatibility check. It is not a working-agent demonstration. The runtime's orchestrator endpoint is deliberately unavailable; the tool runner must fail with its current explicit unimplemented message. No placeholder executor or synthetic drain acknowledgement is substituted.

## Workflow

1. Build the existing runtime/tools Docker targets in an owned builder. Load them into the owned node, read their actual containerd target digests, and pass those content references to the real `Layout` constructor. No image is published to a registry.
2. Install a test-only CSI hostpath backend from the recorded upstream manifests, using native Kustomize for the driver image selection. Configure it as the disposable cluster's default storage class. No existing cluster or storage class is changed.
3. Create agent identity and wake intent in isolated PostgreSQL. Let `Reconciler` prepare, create and bind the actual PVC and Pod.
4. Require all three initialization steps to succeed, the trusted runtime to remain running, and the shipped tool runner to exit with its unimplemented error. Check that the claim is bound to the selected CSI class/driver and the runtime directory is owned by UID 10001 with mode 0700.
5. Write known bytes in the runtime's workspace subpath. Create a separate harmless mount contender and require a real scheduler `FailedScheduling` event mentioning ReadWriteOncePod, tied to the contender's UID while the incumbent still owns the claim.
6. Request sleep and require `AwaitingDrain` while runtime control is unavailable. The fixture operator then explicitly requests deletion with UID and resource-version preconditions. Reconciliation must retain actual termination evidence, finalize the owned Pod and observe its removal. This does not prove disconnected authority, acknowledged drain or partition fencing.
7. Wake the same agent, require a different Pod UID and the same PVC UID, and read the retained bytes. Settle the second Pod through the same explicit operator-stop path. Cleanup removes only the owned cluster, builder, images, database and private kubeconfig.

For each running Pod, the fixture operator also reads the projected token into memory and calls the production `RuntimeIdentity` exchange against actual Kubernetes TokenReview. The issued session must authenticate as the exact agent/attempt/epoch. A different process instance must be rejected. After the first Pod is removed, its session and projected credential must be rejected by Cubebot; the replacement must receive a higher epoch. Tokens are never saved in reports. This exercises the backend exchange through an operator harness, not the runtime process's HTTPS connection or drain acknowledgement.

## Backend and evidence limits

The [upstream CSI hostpath driver](https://github.com/kubernetes-csi/csi-driver-host-path/blob/v1.19.0/README.md) is a test/demo driver. It cannot establish production storage durability, node-loss recovery, physical attachment fencing, capacity guarantees or backup/restore correctness. This fixture uses it to expose actual Kubernetes/CSI mount and lifecycle behavior before selecting and qualifying production storage profiles.

The driver manifest source is commit-pinned to `6fab18ae4fc4f486193b618114eeb817d9929242`; the upstream Kubernetes 1.36 path resolves to its 1.35 manifest. Native Kustomize selects hostpathplugin v1.19.0. Sidecar RBAC/snapshot manifests use recorded version tags, and CSI images use upstream version tags. Downloaded content hashes and observed image IDs provide provenance for a run; those tags are not immutable dependency resolution. The agent images themselves use the actual imported manifest digests.

Public diagnostics stay under the run's ignored `target/cubebot-startup-*` directory. They include Pod/PVC/events and generated CSI manifests, not private kubeconfig content or token values. Diagnostic errors are reported without masking the primary workflow failure. A source change during compilation, image building or the workflow invalidates the success report.

The [recorded run](evidence/2026-10-26-cluster-startup.json) passed on Kubernetes 1.36.4 with the shipped arm64 images: all initializers succeeded, the scheduler rejected the competing RWOP mount, and the same PVC retained the expected bytes across different Pod UIDs. Both epochs settled after explicit fixture-operator stop, and owned-resource cleanup completed. The initial run failed because native Kustomize requires a supported filename; the fixture now writes `kustomization.yaml`.

G1–G3, authenticated runtime operation, tool execution, finite authority and grant renewal remain open.

The later [real-cluster identity run](evidence/2026-10-26-cluster-identity.json) passed those projected-token and epoch assertions as well as the original storage/startup workflow. Rejection of the removed Pod is an end-to-end Cubebot identity result; it does not establish immediate Kubernetes JWT revocation or physical executor fencing.
