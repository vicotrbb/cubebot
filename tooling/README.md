# Development tooling

`quality/` contains the repository complexity gate, its regression tests, and the locked Python development environment. It is not part of any deployed Cubebot application.

`guest/` builds the `kvm` backend's guest into `target/guest`, the sandbox image's `guest` build context, with its sources and intermediate files in `target/guest-build`: a pinned Linux kernel from `kernel.config` merged over tinyconfig, with every driver built in, and a read-only ext4 root image exported from [`apps/agent/Dockerfile`](../apps/agent/Dockerfile) whose init is `cubebot-agent guest`. `just guest-boot` builds both and runs the acceptance in `tests/system/guest_boot.py`.

Run checks through the root `justfile`. The gate scans all authored source in `apps/`, `modules/`, `tooling/`, `tests/`, and any new source directories; the per-function cyclomatic limit remains 15. Process smoke checks live in `tests/system/`.
