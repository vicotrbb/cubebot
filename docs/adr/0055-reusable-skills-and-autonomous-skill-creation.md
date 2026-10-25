# ADR 0055: Reusable skills created by humans, agents, and dreaming

Status: Accepted

Date: 2026-10-23

Skills are a core part of Cubebot's ecosystem. Agents need reusable procedures, references, and scripts alongside their standing instructions and unified memory. Successful work should improve how later work is performed.

Decision: Support reusable skills inspired by OpenBot's package and revision model. Agents can create their own skills during work. Idle dreaming can create and refine skills to improve future work. Authorized humans can create, edit, and import skills. The user explicitly selected these capabilities on 2026-10-23. Skill creation is an ordinary agent capability within its scope, without a new per-creation approval queue.

Usage: After solving a recurring diagnostic problem, an agent saves a procedure with verification steps. During a later idle dream, it identifies a repeated successful workflow and creates or improves a skill, linking the change to supporting history. A human can inspect the result, edit it, or import an existing package for an agent.

Package model: Use a directory with a `SKILL.md` entrypoint containing YAML metadata and Markdown instructions, with optional scripts, references, and assets. Follow the Agent Skills format used by OpenBot; exact compatibility and import validation rules need qualification. References resolve relative to the package. Cubebot's Rust engine loads skills itself, without requiring provider CLI-specific directory copies.

Agents can discover eligible skills and load instructions and supporting files as needed. Full package contents do not belong permanently in every model request. Skills describe how to perform work; memories retain agent knowledge, and tools perform operations. Each has its own lifecycle.

Revision model: Keep immutable, identifiable revisions and per-agent installations selecting a revision. Creation registers the skill and installs it for the creating agent. Publishing a revision does not silently replace installed copies, including those of other agents. Updating an installation is a distinct operation permitted within the actor's scope. Existing executions must not unknowingly mix package revisions. If registration succeeds but installation fails, recovery uses the saved identity instead of creating another skill.

Ownership: A skills module owns catalog metadata, package validation, revisions, installation state, and provenance through orchestrator operations. Runtime adapters make selected package files available in the agent environment. PostgreSQL remains authoritative for durable platform records under ADR 0008; bundle storage and materialization are open. Workspace edits are working copies until registered. Humans, agents, and dreaming use the same revision contract with conflict detection.

Dreaming remains idle-only and yields under ADR 0025. Reports include skills created or revised, revision identities, supporting source references, readable changes, and installation outcomes. Committed results are separate from unfinished proposals. A claimed improvement is not proof of better performance: reports distinguish package validation, executed checks, and untested expectations. Dreaming does not acquire additional external-action authority to validate skills.

Importing or registering a package does not execute its scripts. Validate structure, paths, file types, and size before acceptance. Scripts run in the tool environment under existing execution, network, and credential restrictions. Skill metadata cannot grant tools, override platform policy, or register integrations. Integrity checks do not establish that instructions or scripts are safe or correct. Generated skills should generalize procedures without embedding credentials or unnecessary private source content.

[ADR 0056](0056-shared-installation-skill-library.md) makes registered skills shared and discoverable across the installation by default, with each agent independently selecting its installations. [ADR 0057](0057-collaborative-shared-skill-revisions.md) permits any active agent or admitted active user to contribute revisions. Removal and changes to another agent's installed skills retain separate authority. This does not introduce a public marketplace or authority to publish outside the installation.

Alternative considered: Instructions and memory alone, with ad hoc procedure files. The selected model adds an explicit reusable package, import, revision, and installation lifecycle.

[ADR 0058](0058-skill-imports-and-agent-skills-compatibility.md) selects file, folder, archive, and Git imports with Agent Skills format compatibility. Ecosystem packages are imported from their actual source locations.

Open details: Management of another agent's installations; import mechanics and compatibility qualification; search and activation interface; bundle storage; update activation points; disable and removal behavior; and quality evaluation.

Validation scenarios: Human creation and import; agent creation and installation; idle dreaming with visible skill results; stale-revision conflicts; unchanged installations after publication; partial-installation recovery; unsafe package rejection; bounded loading; and denied authority expansion through skill content.

Evidence: OpenBot revision `cfee2971f29cdb7c16272e60ae912de7fbb30693`, inspected locally on 2026-10-23: [skill architecture](https://github.com/nightly-labs/openbot/blob/cfee2971f29cdb7c16272e60ae912de7fbb30693/docs/architecture/skills-and-plugins.md), [creation and installation tools](https://github.com/nightly-labs/openbot/blob/cfee2971f29cdb7c16272e60ae912de7fbb30693/src/main/local-skill-tools.ts), [immutable revisions](https://github.com/nightly-labs/openbot/blob/cfee2971f29cdb7c16272e60ae912de7fbb30693/src/main/local-skill-library.ts), and [package validation](https://github.com/nightly-labs/openbot/blob/cfee2971f29cdb7c16272e60ae912de7fbb30693/src/main/skill-package.ts). [Agent Skills specification](https://agentskills.io/specification) reviewed for the package format. Dreaming-generated skills are a Cubebot requirement, not a verified OpenBot dreaming feature.
