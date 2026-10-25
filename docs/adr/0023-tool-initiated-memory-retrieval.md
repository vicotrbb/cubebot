# ADR 0023: Tool-initiated memory retrieval

Status: Accepted

Date: 2026-10-23

Hybrid search under [ADR 0022](0022-hybrid-memory-retrieval.md) defines how memories are retrieved. A separate choice determines when saved knowledge enters an agent's working context.

Decision: Search saved memory only when the agent explicitly requests it through a memory tool. The user selected this model on 2026-10-23. The runtime does not automatically search or inject saved memories before a turn. There is no automatically injected essential-memory tier.

The agent decides when it needs prior knowledge and supplies a query and any supported filters. The memory module performs bounded hybrid retrieval through the orchestrator, enforces ownership, and returns results with provenance. Returned material enters the requesting execution lane as a tool result, subject to its context budget. Other lanes do not automatically receive those results.

The current conversation, assigned task, instructions, and an explicit creation briefing remain ordinary execution inputs. Maintaining a bounded representation of that conversation is distinct from searching the long-term knowledge pool. Context maintenance must not introduce implicit saved-memory retrieval. [ADR 0024](0024-rolling-context-compaction.md) selects automatic rolling compaction for active context.

Automatic extraction and dreaming under [ADR 0021](0021-memory-creation.md) may still process recorded material to maintain derived memory. They do not automatically inject the resulting knowledge into working turns. Memory maintenance and working-agent recall have separate triggers.

During orchestrator disconnection, an agent can use already loaded context within the limits of ADR 0012. New central memory queries are unavailable until connectivity returns; the runtime must report that condition rather than fabricate results or silently substitute an unapproved local search path.

Ownership: The agent engine exposes memory tools and decides when to call them. The memory module owns search, filtering, provenance, and result limits. Context assembly owns fitting requested results into the active context.

Alternatives considered: Automatic bounded recall before each turn, supplemented by explicit tools, could surface relevant knowledge without the agent asking. Tool-only retrieval avoids routine search and unsolicited context additions, while making recall depend on the agent recognizing its need for prior knowledge.

Consequences: An agent may overlook useful saved information because it never searches. Automatically extracting more memories does not solve that failure by itself. Instructions and evaluation should cover when to search, how to refine an unsuccessful query, and how to distinguish an unavailable search from an empty result. These instructions guide agent behavior without adding a hidden automatic retrieval path.

Open details: Tool names and schemas; result and query budgets; search guidance; unavailable-search behavior; and evaluation targets for appropriate search initiation and useful recall.

Validation scenarios: No search or saved-memory injection without a tool call; explicit search returns only eligible results; automatic extraction does not inject memories into a later turn; current-conversation compaction introduces no implicit recall; unrelated lanes receive no search results; empty results and connectivity failure remain distinguishable; and an agent can follow provenance to supported evidence through an authorized interface.
