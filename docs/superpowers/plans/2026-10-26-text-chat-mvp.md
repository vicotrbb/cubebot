# Minimal text-chat MVP

Scope: the user request in this chat. Existing accepted ADRs remain authoritative; roadmap features outside text chat are excluded. Work locally on `codex/end-to-end`; do not deploy or publish.

1. Provider and durable chat boundary. Add authenticated encryption in orchestrator connections with a separately supplied installation key. Add one administrator-managed OpenAI connection/model, revisioned settings, and no plaintext secret read endpoint. Add PostgreSQL conversations and turns with exact human permissions, transactional message acceptance, request IDs, immutable committed replies, and bounded context. Test denied access, concurrent duplicate sends, mismatched retries, credential tampering and restart persistence.
2. Connected runtime. Extend existing machine authentication with finite, one-inference turn claims, direct provider requests in the Rust agent, and durable journaled outcomes. Commit results idempotently through orchestrator work. Reconcile journal evidence before admission; fence stale attempts and never start inference from recovered local intent. Bound request duration, output, context and retries. Test crash boundaries, expired authority, provider failure and duplicate result reports.
3. Website. Reuse browser cookies and CSRF protection. Implement sign-in/out, authorized agent listing/create/edit, administrator provider settings and each agent's permanent chat/history/send. Keep retry request IDs stable, preserve pending drafts, poll for queued/running/failed/committed state, and support narrow screens and accessible forms.
4. Acceptance. Run `just check`, `just integration`, `just smoke`; execute browser/runtime workflows with controlled provider and separately with authorized live API credentials. Exercise refresh, process restart, denied access, provider failure and retry. Update README and a results record with commands and actual evidence. Review the final diff. Stop after this scope.

## Decisions

- One native OpenAI API-key connection and explicitly selected model. No automatic model fallback.
- PostgreSQL is authoritative. Acknowledgement follows transaction commit; the runtime journal is recovery evidence only.
- Agent management does not confer access to another person's transcript. Administrators retain accepted default content access.
- Fixed polling and fixed execution limits are sufficient for this MVP; no generic scheduling framework.

## Progress

- Inspected existing identity, agent definitions, machine sessions and journal foundations. No functional chat/provider implementation exists at baseline `ab03414`.
- Credential decision resolved: user explicitly authorized using their provided key for live tests. Never include it in tracked artifacts or test output.

- Completed provider configuration, optional explicit-grant access, durable chat, scoped admission/recovery, the native adapter/runtime, and responsive website slices.
- Ruling: lifecycle dispatch uses a runtime-only projection of the existing fixed pod layout for this MVP. Excluded tool processes and privileged tool-network initialization are not launched; retained legacy attempts still use their existing guarded recovery. Cost: this text-only dispatch path needs separate real-cluster qualification, which was not requested or claimed.
- Final review found and resolved retry acknowledgement state, rejected Unicode input locking and conversation-picker pagination. The full local gates passed, as did controlled browser/native recovery and a two-turn credential-backed OpenAI workflow. Evidence and limits are recorded in `docs/development/text-chat-mvp.md`.
- MVP work stopped at this scope. Nothing was deployed, pushed or published.

- Completion audit enforced ADR 0030: explicit 429/5xx responses now receive one bounded, jittered retry under the original 60-second authority. Retrying progress is durable. Ambiguous network failures do not automatically repeat inference. A native-binary regression reproduced the missing retry, then passed after the fix.

- Recovery expiry also runs in the existing owner loop, independently of runtime polling, so crashed executors cannot leave expired turns visibly running forever. The expired-authority test invokes this recovery path without a returning runtime.

- User correction, 2026-10-26: one agent has exactly one conversation, always; context is managed within it. ADR 0082 records the invariant. Remove independent conversation creation/listing and the picker, migrate history without broadening access, and make agent selection open the permanent stream directly.
