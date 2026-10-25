# ADR 0058: File and Git imports with Agent Skills compatibility

Status: Accepted

Date: 2026-10-23

Cubebot's shared library must accept skills authored outside the installation, including packages from the Agent Skills ecosystem. Imported packages need the same revision, validation, and installation behavior as locally authored skills.

Decision: Support skill imports from files, folders, archives, and Git repositories, with compatibility for the Agent Skills format documented at agentskills.io. The user selected these sources and explicitly included agentskills.io on 2026-10-23.

Source clarification: The reviewed agentskills.io home page and documentation index describe the open format, authoring guidance, and client integration. They do not document a hosted skill registry or package-download API. Cubebot therefore imports ecosystem skills from their actual package or repository locations. Importing the documentation home page itself is not an installation operation. A dedicated remote catalog adapter would require an actual catalog contract and a further decision.

Usage: A human uploads a standalone `SKILL.md`, selects a skill folder or archive, or supplies a Git repository and skill subdirectory. Cubebot inspects and validates the selected package, records its origin and exact content, then registers a revision in the shared library. Installation on an agent uses the existing revision-selection flow. A repository containing several skills must expose explicit package selection rather than silently importing every directory.

Ownership: Source adapters obtain candidate files; the skills module owns normalization, validation, provenance, and revision publication. Folder import means user-selected uploaded content or an authorized workspace source, not arbitrary access to the orchestrator's filesystem. The frontend and PWA may offer different source-selection controls while using the same backend contract.

Git imports record the resolved commit and selected path as well as the source reference. Uploaded packages retain content identity and available origin metadata. Imports create durable local snapshots: a moving branch, deleted upstream repository, or subsequent source edit must not silently change a registered revision. Later upstream imports must account for local contributions rather than overwrite them. Exact update and deduplication behavior remains to be specified.

Use the Agent Skills structure: `SKILL.md` with metadata and instructions, plus optional supporting files. Preserve declared compatibility, attribution, and license metadata where present. Format compatibility does not promise every imported procedure can run: provider-specific tools, missing software, and unavailable integrations must be reported. Skill metadata, including tool declarations, cannot expand Cubebot grants. A standalone entrypoint with missing referenced files must surface that limitation rather than appear to be a complete package.

Fetch and validation use existing network and credential restrictions. Private repositories require applicable installation-managed access. Reject unsafe paths and unsupported file forms, bound transfers and expanded content, and do not execute package scripts or repository hooks during import. Importing a skill does not register external integrations, install all its dependencies, or publish local changes upstream.

Alternatives considered: Direct imports alone without an explicit ecosystem compatibility target; or adding external catalog browsing or a Cubebot-operated public marketplace. The selected scope is direct imports plus Agent Skills compatibility. No public marketplace is selected.

Open details: Supported archive and Git URL forms; repository access mechanics; multi-skill selection; import size limits; format compatibility policy; duplicate imports; upstream update reconciliation; and import authority for agents.

Validation scenarios: Import a standalone entrypoint, complete folder, archive, and selected Git subdirectory; preserve supporting files; record a resolved commit; detect missing references; reject path escape and oversized packages; preserve a local snapshot after upstream changes; and prevent import-time execution or authority expansion.

References reviewed on 2026-10-23: [Agent Skills overview](https://agentskills.io/home), [documentation index](https://agentskills.io/llms.txt), and [format specification](https://agentskills.io/specification).
