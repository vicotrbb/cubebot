# Sandbox application

`cubebot-sandbox --backend kvm|process` runs one agent (ADR 0084). The orchestrator starts it as the only container of an agent's pod, or as a Docker container without a cluster.

| Backend   | What runs                                                                                  | Isolation claim |
| --------- | ------------------------------------------------------------------------------------------ | --------------- |
| `kvm`     | A KVM guest with virtio-blk root and workspace disks, virtio-net on a tap, vsock, serial   | Yes, once built |
| `process` | `cubebot-agent` from the same directory as a plain child; reaps orphans as container PID 1 | Never           |

The `kvm` backend is not implemented yet and exits with an error rather than falling back. It will drive KVM through `libc`/`nix` ioctls with no VMM crates; this crate is then the only one allowed `unsafe`, confined to KVM and guest memory.

The image is built from [`Dockerfile`](Dockerfile) with the repository root as context and contains both `cubebot-sandbox` and `cubebot-agent`.
