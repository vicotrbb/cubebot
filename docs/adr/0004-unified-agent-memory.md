# ADR 0004: Unified memory per agent

Status: Accepted

Date: 2026-10-22

An agent can participate in private conversations, shared rooms, direct agent messaging, and tasks. Its memory model must define whether knowledge learned in one context is available in another.

Decision: Each agent has one unified knowledge pool used across all its scopes and scenarios. The user selected this model on 2026-10-22 and explicitly confirmed that each agent's own memory is used everywhere it operates. Memory is not partitioned by user, room, conversation, or task audience.

The agent may retrieve and use relevant memories acquired in any of its contexts. Summaries, extracted knowledge, and memories produced during dreaming follow the same model. Unified availability does not require loading all memory into every model call; context selection and compaction remain runtime responsibilities.

Rationale: The selected model gives each agent continuous knowledge and experience across its work, with one memory ownership rule.

Alternative considered: Audience-scoped knowledge would restrict retrieval according to the current conversation's permissions and require explicit sharing across contexts. This was declined in favor of unified knowledge per agent.

Consequences: Access controls on conversation transcripts do not guarantee that information from those conversations remains confined to their original audience. An agent may use what it learned in a private conversation when answering in a shared room. The console must explain this distinction. Work requiring separate knowledge pools should use separate agents; transfers between agents still require their own communication and access rules.

Scope: This decision does not create an installation-wide memory pool or automatically grant one agent access to another agent's memories. It does not grant additional tool, credential, or resource permissions. Human access to stored transcripts, memories, and files remains governed by the permission model, including administrator access under [ADR 0003](0003-administrator-content-access.md).

Related decision: [ADR 0020](0020-memory-consolidation-and-source-history.md) preserves source history during consolidation. Dreaming optimizes derived memory and context; source removal follows separate retention or deletion policy.

Related decision: [ADR 0021](0021-memory-creation.md) combines explicit saves and automatic extraction through one memory module with shared commit rules.

Open details: Memory representation, retrieval, extraction mechanics, consolidation mechanics, retention, deletion, concurrent updates, and the mechanics of inter-agent knowledge transfer will be specified separately.

Validation scenarios: An agent can recall a fact from a direct conversation in a room and in a later task; consolidation preserves knowledge without introducing audience partitions; another agent cannot automatically retrieve that memory; direct transcript access still follows permissions. Retrieval correctness and bounded context use will receive separate acceptance criteria.
