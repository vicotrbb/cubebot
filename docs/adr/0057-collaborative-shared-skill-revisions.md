# ADR 0057: Shared skill revisions by any agent or user

Status: Accepted

Date: 2026-10-23

The shared skill library should accumulate improvements from everyone using it. Revision authority must support that collaboration while preserving history and each agent's selected installed behavior.

Decision: Any active agent or admitted, active user in the installation can contribute a new revision to an existing shared skill. The user explicitly selected both agents and users on 2026-10-23. Contributions do not require creator ownership, a maintainer assignment, or a separate approval step before publication to the installation library.

Usage: Agent B improves a procedure created by agent A and publishes a new revision. A human then corrects the procedure using the same revision interface. Both contributions retain attribution and prior versions. Agents using an earlier revision continue using it until their installation is updated under ADRs 0055 and 0056.

Ownership: The skills module owns revision publication, immutable content, provenance, and conflict handling. Platform identity supplies the authenticated actor. Authorization enforces active installation membership or agent identity and existing operational restrictions. Shared-package reading needed to contribute is available to installation members; this does not expose protected source history referenced by the package.

Each revision records its contributor, parent revision, time, and inspectable changes. Publication validates the package and expected current revision. Concurrent contributors must not silently overwrite each other's work: a stale contribution receives a conflict and can be reconciled against the current version. Retrying publication after a lost response must not duplicate a committed contribution.

Agents may contribute during ordinary work and idle dreaming. Dream reports identify revisions actually saved and distinguish them from drafts or validation failures. Publication remains separate from installation, and neither contribution nor installation grants permission to execute otherwise unauthorized tools or access referenced resources.

This authority allows new revisions; it does not authorize rewriting old revisions, deleting the skill or its history, changing another agent's installed revision, or publishing externally. Those operations retain their separate scopes. Imported skills can receive local revisions while retaining their recorded origin; a local contribution does not update the upstream source.

Package validation establishes structural acceptability, not correctness or improved performance. Contributor identity, history, and any verification evidence remain visible so consumers can assess a revision before adopting it. Shared-content handling under ADR 0056 still applies.

Alternative considered: Restrict revisions to creators and assigned maintainers. The user selected open contribution across installation agents and users to support collective improvement.

Open details: Revision presentation; contribution metadata; conflict reconciliation interface; publication idempotency; removal and withdrawal authority; and update-adoption experience.

Validation scenarios: A non-creator agent publishes a revision; an ordinary active member publishes a correction; a suspended member is denied; stale publication reports a conflict; previous revisions remain readable; existing installations remain unchanged; and contribution rights do not permit deletion or access to protected provenance.
