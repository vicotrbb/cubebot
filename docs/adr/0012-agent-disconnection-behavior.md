# ADR 0012: Bounded continuation during disconnection

Status: Partially superseded by ADR 0083

Date: 2026-10-23

Superseded in part by [ADR 0083](0083-agent-centric-execution-in-a-sandbox-boundary.md): there is no finite per-turn authorization window. The agent works under its long-lived sandbox authority, may start queued work while disconnected, and stops only for work that needs the orchestrator, revocation or a stopped sandbox. The local journal and idempotent reconciliation remain.

The orchestrator has one active instance with recovery downtime under [ADR 0011](0011-orchestrator-availability.md). Agent pods may remain healthy while their orchestrator connection is unavailable. They cannot directly access PostgreSQL under [ADR 0009](0009-agent-state-access.md).

Decision: Agents may continue already-authorized work within limits while disconnected. The user selected this model on 2026-10-23. Each active lane may continue its current turn using available context and valid existing authority. Agents accept no new assignments and start no new queued turns while disconnected.

Continuation must have a finite authorization window and respect the turn's remaining execution budgets. Disconnection grants no new tool, credential, data, or spending authority. Operations requiring unavailable state or permission changes wait for reconnection. [ADR 0013](0013-tool-autonomy.md) establishes autonomous invocation within existing grants, without a Cubebot per-action approval workflow. Exact time limits, budget accounting, and authorization representation remain open.

The agent keeps a durable local journal on persistent storage for unsynchronized execution progress and results. This is recovery evidence for assigned execution, not a second independently mutable copy of the platform database. PostgreSQL remains authoritative for platform state. Local memory changes remain staged until the orchestrator validates and commits them; other lanes or agents must not assume those changes are centrally visible.

Usage: A turn already researching with available context may continue model and permitted tool work during an orchestrator restart. Under [ADR 0031](0031-direct-inference-and-propagated-authentication.md), model requests go directly to the provider using already-propagated valid authentication. It records progress locally. On completion it retains its result and waits to synchronize. It cannot claim another task, directly deliver a room message, or obtain new memories from the unavailable orchestrator.

On reconnection, the agent authenticates, reconciles execution identity and authorization, and submits journaled progress through idempotent orchestrator operations. Central acknowledgement determines when records can be removed from the journal. Known results and uncertain outcomes must be reconciled without blindly repeating tool effects. A rejected or stale execution report does not erase evidence of actions that actually occurred.

When authorization expires, a budget is exhausted, or durable journaling cannot continue, the engine stops starting further actions and retains recovery state. Already-issued actions may still finish; pausing cannot undo external effects. The design must define handling for those in-flight operations. A restarted agent does not gain permission to resume merely because a journal exists.

Rationale: Bounded continuation preserves useful progress through short coordination outages while limiting work performed without current central state.

Alternative considered: Pausing at the next safe point immediately upon disconnection would simplify recovery but interrupt progress during even short outages. The user selected continuation with durable buffering and reconciliation.

Consequences: Permissions and cancellation changes cannot be assumed immediately visible to a disconnected agent. Offline authority must expire, and reassignment must account for an old executor that may still be acting. Local journal durability, capacity, replay, and cleanup become required runtime behavior. Persisted records must distinguish local completion from centrally acknowledged completion.

Open details: Authorization duration and renewal; clock and expiry handling; local journal format and limits; action-intent recording; idempotency and reconciliation protocol; spending estimates and budgets; in-flight tool handling; and safe reassignment after expiry.

Validation scenarios: Model and tool work during disconnection; turn completion before reconnection; expiration with a tool in flight; a permission revoked centrally during the outage; journal storage exhaustion; pod failure before synchronization; repeated journal submission; and reconnection after the orchestrator has marked execution uncertain.
