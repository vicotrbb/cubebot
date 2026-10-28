# Sandbox application

`cubebot-sandbox --backend kvm|process` runs one agent (ADR 0084). The orchestrator starts it as the only container of an agent's pod, or as a Docker container without a cluster.

| Backend   | What runs                                                                                  | Isolation claim |
| --------- | ------------------------------------------------------------------------------------------ | --------------- |
| `kvm`     | A KVM guest with virtio-blk root and workspace disks, virtio-net on a tap, vsock, serial   | Yes, once built |
| `process` | `cubebot-agent` from the same directory as a plain child; reaps orphans as container PID 1 | Never           |

The `kvm` backend (x86_64 Linux) drives KVM through `nix` ioctls with no VMM crates; `src/kvm` is the only module in the workspace allowed `unsafe`. Today it boots the bzImage in `CUBEBOT_GUEST_KERNEL` with the initramfs in `CUBEBOT_GUEST_INITRAMFS` (built by `tooling/guest`) in one 256 MiB memory region on one vCPU, and relays a 16550 serial console to stdout. It has no virtio devices yet, so the guest agent has no network, disks or vsock and cannot reach the orchestrator: booting is not an isolation or agent qualification. `just guest-boot` requires the agent's startup line within two seconds and the VM to exit when the guest shuts down.

The image is built from [`Dockerfile`](Dockerfile) with the repository root as context and contains both `cubebot-sandbox` and `cubebot-agent`.
