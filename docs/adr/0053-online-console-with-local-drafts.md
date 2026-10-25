# ADR 0053: Online console operations with local drafts

Status: Accepted

Date: 2026-10-23

The web console and mobile PWA can lose connectivity while a person is composing a message or managing agents. The client needs to preserve unfinished writing without suggesting that disconnected actions have reached the platform.

Decision: Require an online connection for chat delivery and management operations, while preserving local message drafts during connection loss. The user selected this model on 2026-10-23. Offline message delivery queues, queued management changes, and an offline conversation-reading feature are outside the initial design.

Usage: A user composing a message loses connectivity and continues editing a local draft. The interface identifies the disconnected state. After reconnection and any required reauthentication, the user can send the draft. Reconnecting alone does not submit it or start agent work.

Ownership: The frontend owns local drafts and connection-state presentation. The orchestrator owns accepted messages, authoritative resource state, and operation outcomes. Backend authorization remains required when an operation is submitted. Local draft storage does not confer permission to send into a conversation after access has changed.

Drafts must be associated with the installation, user, and intended conversation so account switching or another installation cannot expose or misroute them. Local storage mechanism, retention, size limits, and logout cleanup remain to be specified. Cross-device draft synchronization is not selected by this decision.

The interface must distinguish an unsent draft, an in-flight submission, a server-accepted message, and an unresolved submission outcome. If a connection drops after sending but before acknowledgement, the message may already have been accepted. Reconnection must reconcile that submission using a stable identity before offering or performing a retry that could duplicate work. This recovery is distinct from accepting new offline sends into a delivery queue.

Management controls require current connectivity. A previously visible state must not be presented as live after disconnection, and edits or cancellation requests must not be represented as applied before confirmation. Existing visible content may remain labeled as stale; the initial design does not promise durable offline access to conversation history.

Live browser control follows ADR 0039. Disconnected input must not accumulate for replay, and reconnection must establish current control ownership and page state before further interaction. This decision does not change the separately selected bounded agent execution during orchestrator disconnection in ADR 0012.

Alternatives considered: Cached offline reading with queued chat messages, or a broader queue of offline management operations. Online operations with drafts avoid those additional synchronization contracts while preserving unfinished writing.

Open details: Draft persistence and cleanup; attachment handling; connection detection; submission reconciliation; stale-state presentation; and refresh behavior after reconnection.

Validation scenarios: Preserve a draft through connection loss; avoid automatic sending on reconnect; isolate drafts by installation and user; deny sending after access is revoked; reconcile an acknowledgement lost after message acceptance; identify stale views; and prevent replay of disconnected browser input.
