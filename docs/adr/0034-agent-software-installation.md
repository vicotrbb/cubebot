# ADR 0034: Agent-directed software installation within grants

Status: Accepted

Date: 2026-10-23

Agents need dependencies, command-line tools, and language environments that cannot all be anticipated in a base image. Requiring an administrator image change for every missing dependency would interrupt useful autonomous work.

Decision: Allow agents to install additional software into their writable workspace or user environment within granted permissions. The user selected this behavior on 2026-10-23 and emphasized that installation is important for getting work done. Installation is a supported capability, not an exceptional workflow requiring approval for each package.

Usage: A document-processing task requires a missing library. An authorized agent installs it into an environment in its workspace, verifies that it can use it, and continues the task. A coding agent can similarly install project dependencies or a user-local CLI. The operation runs in the tool container under ADR 0033; it does not modify the Rust runtime container or its credentials.

The administrator-managed base image remains the foundation. This decision grants neither root access nor authority to change the managed image, host, or Kubernetes configuration. Requirements that genuinely need unavailable system dependencies must be reported accurately and can be addressed through administrator-managed images. Selection of base images and their included tools remains open.

Installation uses ordinary ecosystem tooling through the existing tool-execution interface. Cubebot does not need a custom package manager or an additional package-by-package approval queue. Resource budgets, allowed network access, filesystem permissions, and available integration credentials still apply. Installation grants do not create new external access or expand an agent's authority.

Ownership: Authorization defines permitted execution and resource access. The tool executor runs installation commands, captures outcomes, and applies execution limits. Workspace lifecycle owns persistent paths and quotas. Image administration owns the base environment. The agent owns selecting and using dependencies appropriate to its task.

Grant enforcement must reflect the actual powers of a terminal. An environment that permits arbitrary code execution, downloads, and writes already permits many forms of software installation. A package-manager command denylist is not an enforceable installation restriction. The exact grant vocabulary and underlying filesystem, network, and execution controls remain to be designed together.

Supported installation locations must be clear to the agent. Software placed in designated persistent workspace or user-environment paths survives ordinary sleep and pod replacement as files under ADR 0008. Changes in disposable container storage have no durability guarantee. Persistence does not guarantee binary compatibility after a base-image or architecture change; environment repair and rebuild behavior must be specified separately.

Installation is an external tool effect, not a transactional memory write. A failed or interrupted command can leave partial files. The agent must inspect the outcome before declaring the software ready or blindly repeating installation. Concurrent lanes share the workspace under ADR 0005, so shared environment mutations require coordination; project-local environments can reduce unnecessary sharing.

Alternative considered: Permit only preinstalled software and require image updates for missing dependencies. That improves environmental uniformity but blocks work on administrative intervention. The user selected agent-directed installation within existing permissions.

Consequences: Environments can evolve independently across agents. Dependency records and lockfiles should be retained where supported to aid reproduction and repair. Downloads, build artifacts, caches, and installed software consume bounded storage and compute. Exact tool images, persistence paths, and cleanup behavior remain implementation choices.

Related decision: [ADR 0035](0035-tool-network-access-defaults.md) permits public internet access by default, including package downloads, while requiring grants for private destinations and allowing stricter installation policy.

Open details: Base images and included toolchains; persistent home and environment layout; grant vocabulary; network enforcement and package-source restrictions; workspace quotas; concurrent installation coordination; environment repair after image changes; and dependency observability.

Validation scenarios: Install a missing library and complete the task without another approval; install and invoke a user-local CLI; reuse a persistent environment after sleep and replacement; report a missing system dependency honestly; enforce network and storage limits; preserve runtime credential isolation; recover from partial installation; and coordinate concurrent edits to a shared environment.
