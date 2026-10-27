# ADR 0082: One permanent conversation per agent

Status: Accepted

Date: 2026-10-26

Decision: Every agent has exactly one permanent conversation, always. Its context is managed within that single conversation. The user explicitly corrected the MVP's multiple-conversation model on 2026-10-26.

Selecting an agent opens its existing continuous chat. There is no new-conversation action, conversation picker, independent session, or alternate context for that agent. Context-window management does not create a new conversation or erase retained history. Restarting, sleeping or editing an agent does not reset the conversation.

This supersedes earlier references to multiple direct conversations belonging to one agent in ADRs 0004 and 0070 and the first text-chat prototype. Agent management and content permissions remain distinct. One shared context does not authorize a person to read or send messages without the applicable content permission. Installation administrators retain their accepted default content access.

Implementation: the agent's identity is the conversation identity. A database view derives exactly one conversation from every agent, including a newly created agent that has never been opened. Message foreign keys refer to that same identity. No conversation-creation operation remains.

Migration: combine prior histories in their existing message sequence order without changing message/result identities. Retain former conversation IDs, each turn's source and each grant's original scope as migration evidence, not active histories. Old message references resolve to the permanent conversation after current authorization checks, preserving accepted-request retries without creating an alternate context. Preserve permissions for a single prior history. If several histories had separate audiences, revoke their old grants and require an administrator to approve access to the combined transcript. Preserve a prior initiator's rights only if that person already had access to every source history. Never union private audiences and silently expose their histories to each other.

Validation: new agents already have one conversation; concurrent openings and restarts return the same identity; additional conversation creation is unavailable; different authorized participants use the same stream; management alone does not grant content access; migration retains history and fails closed on different audiences; the website opens directly into the chat; inference context comes from that permanent history.
