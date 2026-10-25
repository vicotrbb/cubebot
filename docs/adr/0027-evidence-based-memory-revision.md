# ADR 0027: Revise human-corrected memories from new evidence

Status: Accepted

Date: 2026-10-23

Human corrections under ADR 0026 can fix mistaken knowledge, but the underlying facts may subsequently change. Cubebot needs to distinguish a supported update from repetition of previously rejected evidence.

Decision: Allow the agent and automatic memory maintenance to revise a human-corrected memory when new evidence warrants the change. The user selected this policy on 2026-10-23. A human edit does not make a memory immutable or require approval of each later revision. Protected-by-default memories and an optional per-memory protection mode were considered and not selected.

Usage: A user corrects a project deadline to November 15. Later, a project update explicitly changes it to November 22. The agent can revise the memory, preserving the human correction in its revision history and citing the new update. Merely rereading the old, rejected deadline does not justify changing it back.

Newly retrieved does not necessarily mean new evidence. A later ingestion timestamp, another paraphrase of an old source, or a repeated model interpretation is insufficient by itself. Revision should account for what the evidence actually establishes, its applicability, and whether it supersedes the earlier fact. Recency alone does not establish truth. When evidence does not resolve a contradiction, the memory can retain the uncertainty rather than invent a definitive resolution.

Ownership: Working agents and maintenance jobs propose revisions with evidence references. The memory module enforces authorization, expected revisions, provenance, and recorded suppression rules at commit. Assessing the meaning and reliability of evidence remains an inference task to evaluate; validating the record structure cannot guarantee semantic correctness.

Each revision retains the changed content, actor or process, time, and supporting evidence under the revision-history policy. A concise explanation of the change should be inspectable without exposing hidden reasoning. Revisions made during dreaming appear in its committed-change report under ADR 0025. This decision does not require a separate notification for every revision.

Stale proposals must not overwrite a newer human or agent edit. A proposal built against an obsolete revision must be reconciled before committing. Reprocessing the same rejected evidence remains subject to ADR 0026's correction and removal rules. This decision permits supported revisions to existing memories; it does not grant blanket permission to recreate explicitly removed memories or erase source history.

Memory is knowledge, not an authorization mechanism. Revising a remembered statement cannot change an enforced capability grant, credential scope, resource budget, or configured instruction. Those retain their own owners and permission checks.

Alternatives considered: Protect every human correction until a user changes it, or let users protect selected memories. Both give stronger control over individual values but can leave knowledge outdated or add another user-facing policy. The user selected revisable corrections with evidence and revision history.

Consequences: Human control includes inspection and further correction, rather than a guarantee that an edited value remains unchanged. An agent can still misjudge evidence and make an incorrect revision. Evaluation must cover misleading recent sources, repeated old sources, actual changes, and unresolved disagreement.

Open details: Evidence and revision schemas; source reliability and temporal interpretation; contradiction representation; notification preferences; reconciliation behavior; and revision-quality acceptance criteria.

Validation scenarios: Revise a human-corrected deadline from a supported later update; preserve attribution and source references; refuse to treat re-ingested rejected evidence as new; retain an unresolved contradiction; detect a stale candidate after another edit; expose dreaming revisions in its report; and prevent memory revisions from altering enforced permissions.
