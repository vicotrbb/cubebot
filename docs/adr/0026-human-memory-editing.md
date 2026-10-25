# ADR 0026: Direct memory editing and conversational correction

Status: Accepted

Date: 2026-10-23

Users can inspect memories and dreaming results under ADR 0025. They also need a reliable way to correct a mistaken interpretation or remove an unwanted memory.

Decision: Support both direct memory editing in the web console and mobile PWA and conversational correction through the agent. The user selected this model on 2026-10-23. Authorized users can revise or remove a memory directly, without requiring the agent to interpret the requested change. They can also ask the agent to perform the correction in chat.

Usage: A dreaming report reveals that an agent inferred the wrong project deadline. A user opens the affected memory, changes the deadline, and saves the correction. Alternatively, the user tells the agent which deadline is wrong and requests a correction. Both routes update the same agent-owned knowledge pool through the memory module.

Direct edits are authenticated domain operations through the orchestrator. They can complete while the agent sleeps. They record the human actor and become authoritative when the operation commits. Conversational changes retain the requesting user and source-message provenance and follow the agent's successful-turn commit rules under ADR 0005. A proposed correction must not be presented as persisted before its commit succeeds.

Reading memory and editing memory are distinct permissions. The detailed role-to-permission mapping remains open; administrator read access under ADR 0003 does not itself settle mutation rights. Conversational tools must not turn an unauthorized request into an authorized human correction. Ordinary agent learning remains governed by its own grants and memory policy.

Ownership: The memory module owns mutation validation, revision checks, provenance, and suppression rules for both routes. The console and conversational tools are callers of that interface. No separate editable copy of memory is introduced. A stale edit or background candidate must not silently overwrite a newer committed revision.

Corrections retain an inspectable revision trail with actor, time, source, and changed content, subject to explicit retention and deletion rules. Removing a memory excludes it from active retrieval. It does not by itself erase original conversation history or promise erasure of all audit records, summaries, and already-loaded contexts. The interface must communicate the scope of removal; broader erasure remains a separate design topic.

Later extraction or dreaming must respect recorded correction and removal intent when processing the same rejected evidence. Replaying old source material must not simply recreate the rejected memory as a new record. The representation of that intent and handling of paraphrased duplicates remain implementation details requiring evaluation. [ADR 0027](0027-evidence-based-memory-revision.md) permits revision of human-corrected memories when genuinely new evidence warrants it, with attribution and revision history preserved.

Editing durable memory does not retroactively change a running model request or an already-performed action. Propagation to active contexts, correction notifications, and revision history retention remain open. This decision does not add automatic saved-memory injection contrary to ADR 0023.

Alternatives considered: Conversational correction alone would reduce console functionality but make every correction depend on model interpretation and successful execution. Direct editing plus conversation gives users an exact edit path and a convenient natural-language path.

Consequences: The console needs revision-aware editing and clear conflict feedback. Automatic extraction and dreaming must reconcile their candidates against human corrections. Reliable suppression requires more than deleting a search-index entry.

Related decision: [ADR 0025](0025-idle-dreaming-and-visible-results.md) provides the dreaming reports from which users can inspect affected memories. Revision history does not imply a whole-run rollback feature.

Open details: Editing permissions; evidence assessment and contradiction handling; suppression representation; conversational target disambiguation; active-context propagation; revision retention; broader erasure; and rollback support.

Validation scenarios: Edit while the agent sleeps; correct through chat and verify commit; remove a memory from retrieval; reject an unauthorized direct or conversational mutation; detect a stale editor revision; reject an outdated extraction candidate; prevent replay of rejected source evidence from recreating the memory; and preserve accurate attribution for both correction paths.
