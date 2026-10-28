# ADR 0084: In-house VMM as the agent sandbox

Status: Accepted

Date: 2026-10-28

ADR 0083 made one VM-class sandbox per agent the trust boundary and named a Kubernetes RuntimeClass, such as Kata Containers, as the way to get it. That ties every qualified installation to a cluster-level runtime handler Cubebot does not control, rules out clusters where the administrator cannot install one, and leaves no VM path at all for cluster-less development.

Decision: Cubebot ships its own thin KVM virtual machine monitor as a fourth application, `apps/sandbox`, built as the `cubebot-sandbox` binary. It is Rust, drives KVM through ioctls using the `libc` and `nix` crates already in the dependency graph, and uses no VMM crates (no rust-vmm, Firecracker or Cloud Hypervisor components). The user selected this on 2026-10-28.

- **One container per agent.** On Kubernetes the agent pod runs one container, `cubebot-sandbox`, with access to `/dev/kvm`. The agent runs inside the guest VM, not in the container.
- **Guest devices.** The guest gets virtio-blk disks for a read-only root image and the agent's workspace, virtio-net backed by a tap device inside the pod's network namespace, virtio-vsock for control between the sandbox process and the guest, and a serial console whose output becomes the container log.
- **Network policy still applies.** Guest traffic leaves through the tap device in the pod network namespace, so pod-level NetworkPolicy governs exactly one agent, as ADR 0083 requires.
- **Two backends, selected by configuration.** `kvm` (Linux only) boots the guest and is the only backend that carries the isolation claim. `process` runs the agent as a plain child process of `cubebot-sandbox` for development; it makes no isolation claim and must never be reported as qualified.
- **The orchestrator sees one interface.** A `Sandbox` interface with create, start, stop, exec channel, workspace and status operations hides where sandboxes run. Kubernetes (one pod and one workspace PVC per agent) and Docker (cluster-less) implement it; both launch `cubebot-sandbox` with the configured backend.
- **Docker sandboxes are attempts too.** Each Docker container is recorded as a pod attempt with its own epoch, so sessions, drains and recovery rules match Kubernetes. Without a Kubernetes service-account token, each container receives a random bootstrap token minted by the orchestrator, which stores only its digest; the token logs in only while that container is the agent's current running attempt.

Usage: An administrator configures the sandbox image and backend. The orchestrator creates the agent's workspace and starts its sandbox. On a KVM node `cubebot-sandbox` boots the guest, attaches the workspace disk and tap device, and relays the guest serial console to the container log. Stopping the agent stops the container, which ends the VM.

Rationale: A VM boundary is what ADR 0083 needs; a RuntimeClass was only one way to obtain it. Owning a small VMM keeps the boundary inside Cubebot's release and qualification, works wherever `/dev/kvm` is reachable, and avoids depending on a cluster-scoped runtime handler. The device set is deliberately minimal: block, network, vsock and serial cover the agent, its terminal, its browser and its journal. Avoiding VMM crates keeps the device model small enough to review in full.

Relationship to other decisions: This amends the sandbox mechanism of ADR 0083; its ownership, authority and ledger rules are unchanged. ADR 0076 RuntimeClass selection is no longer the path to a qualified agent sandbox; a cluster may still schedule the sandbox pod on any runtime that exposes `/dev/kvm`. Durable attempt records and the rule that an uncertain effect is never blindly replayed remain in force.

Consequences: The separate tool container, privileged network initializer, in-pod kernel egress baseline and two-container recovery checks are removed; the agent runs its tools in its own sandbox. Cubebot now owns VMM code, so the guest-facing device emulation is the attack surface to review and fuzz. The `kvm` backend needs `unsafe` for ioctls and guest memory; it is confined to `apps/sandbox` and every other crate keeps `unsafe_code = "forbid"`. Nodes need `/dev/kvm`, normally through a device plugin rather than a privileged pod, so managed clusters without nested virtualization still cannot qualify. A guest kernel and root image become release artifacts.

Alternatives considered: Keep a RuntimeClass such as Kata, which leaves the boundary outside Cubebot and unavailable without cluster-level installation. Embed rust-vmm, Firecracker or Cloud Hypervisor crates, which brings a far larger device model and dependency surface than four devices need. Use gVisor or plain containers, which do not give a hardware virtualization boundary.

Open details: Guest kernel and root image build and signing; vsock control protocol; how `/dev/kvm` reaches the pod on each qualified cluster; tap and address setup inside the pod network namespace; guest memory and vCPU sizing; the qualification matrix for the `kvm` backend.

Validation scenarios: A sandbox on a KVM node boots the guest, mounts the workspace disk and logs through the serial console; a command in the guest cannot reach another agent, the database or the Kubernetes API, and NetworkPolicy changes take effect on guest traffic; stopping the agent ends the VM; the `process` backend is never reported as isolated; a sandbox lost mid-command recovers from its journal and the orchestrator's attempt records without re-running the command.
