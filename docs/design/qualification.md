# Cubebot qualification and delivery plan

Date: 2026-10-24. Status: proposed acceptance plan accompanying the [architecture](../../ARCHITECTURE.md) and [software specification](software-specification.md). None of the runtime, provider, browser, security, capacity, backup, or upgrade scenarios below has been executed. Documentation review is not product qualification.

## Evidence classes

| Evidence | Establishes | Does not establish |
| --- | --- | --- |
| Design review and document checks | Traceable requirements, coherent proposed ownership, valid local references | Runtime correctness or feasibility |
| Unit and property checks | Pure state transitions, permission algebra, revision and accounting invariants | Real database, filesystem, provider, or network behavior |
| Integration checks | Real PostgreSQL transactions, supported filesystem publication, actual protocol adapters | Production cluster behavior unless run there |
| Controlled cluster qualification | Deployment, lifecycle, policy enforcement, fault recovery against repeatable fixtures | Useful live model work or subscription authentication |
| Live workflow qualification | Real providers and tools complete inspected organizational deliverables | Unmeasured scale, untested provider/runtime combinations |
| Capacity and recovery runs | Performance and behavior on a recorded hardware/software profile | Universal cluster capacity or thousands-agent support |

Record exact source revision, images, dependency versions, cluster version, CNI and policies, CSI/storage mode, node resources, provider models/accounts, data volume, fixtures, timestamps, raw measurements, and output rubric for every qualification report. Credentials must never appear in the report.

## Hard release gates

| Gate | Required proof | Failure response |
| --- | --- | --- |
| G1 Runtime/tool isolation | Arbitrary tool code cannot read runtime provider credentials, journal, bootstrap token, or act as the runtime | Block release of the affected execution mode |
| G2 Network grants | Installed programs, direct sockets, browsers, local MCP, redirects, IPv4/IPv6, and alternate DNS obey public/private restrictions while runtime-only paths remain protected | Do not claim isolation based on proxy settings; resolve the pod-layout enforcement design |
| G3 Exclusive ownership | Restart and partitions cannot create concurrent current orchestrators or duplicate lane effects; stale credentials/epochs cannot commit | Hold uncertain execution; qualify fencing before automatic replacement |
| G4 Durable outcomes | Accepted messages survive restart; uncertain effects are not blindly replayed; upload readiness never precedes durable bytes | Block claims of reliable recovery and publication |
| G5 Native providers | Each advertised adapter completes tool-using, streaming, image-capability, error, and recovery cases under its stated matrix | Mark unsupported combinations explicitly |
| G6 Subscription access | Every advertised Codex/Claude subscription path works with the native Rust engine and selected authentication lifecycle | Keep integration unqualified; do not substitute a CLI engine or promise access |
| G7 Memory fidelity | Hybrid retrieval, compaction, corrections, source preservation, and visible dreaming changes meet measured rubrics | Fix or narrow support claims; never label narrative-only results committed |
| G8 Backup and upgrade | Restore matched database/files/workspaces/keys in isolation; recover interrupted maintenance; resume without duplicate effects | Do not label backup complete or upgrade safe |
| G9 Useful scale | 100 populated agents and 25 simultaneously progressing agents meet workflow, correctness, and interaction targets | Report the measured supported envelope |
| G10 Human controls | Steering, cancellation, browser takeover, access revocation, review, and mobile reconnect have accurate state and authorization | Block affected control capability |

G1–G3 are prerequisites for uncontrolled tool workloads. G6 is a prerequisite for subscription-support claims specifically; API-key qualification cannot substitute for it. A first release that omits an originally desired subscription integration must make that product gap explicit in release review.

## Initial measurement profile

These are engineering proposals to make the first run reproducible, not accepted hardware requirements or measured results:

- Three Linux worker nodes, each with 16 vCPU and 64 GiB RAM, plus adequate control-plane capacity. Record the actual machines, network, and overcommit rather than relying on nominal totals.
- A NetworkPolicy-enforcing CNI and a supported CSI driver. Use SSD-backed storage, initially at least 1 TiB usable across the required volumes, and record throughput and attachment constraints.
- Orchestrator starting allocation: 2 vCPU and 4 GiB RAM; PostgreSQL: 4 vCPU and 8 GiB RAM; frontend: 1 vCPU and 1 GiB RAM. These are tuning inputs.
- Default working agent starting requests: 250 millicores/256 MiB for runtime and 500 millicores/1 GiB for tools, with higher limits and workload-specific browser/compiler profiles. Admission accounts for requested and measured peak resources.
- 100 agents, each with at least 1,000 retained messages, 500 memories, several installed skills, and representative workspace content. Include individual 50,000-message conversations, large tool outputs within capture limits, and artifacts across small and large sizes.
- 25 distinct active agents, including main and task lanes, shared provider-account contention, at least five simultaneously used browser sessions, and humans browsing, sending, and steering. Do not equate the installation lane cap with active-agent count.

If this hardware is unavailable, use a different documented profile and report its results honestly. The design must not infer capacity from these suggested allocations. Sleeping agents still consume retained storage; reserved PVC capacity and actual used bytes must both be measured.

## Workload and outcome rubric

Run a repeatable controlled-provider load for 60 minutes and a separate 60-minute live-provider workflow run after warmup. Maintain 25 distinct agents performing useful work through replenishment with independently scored goals. In each five-minute interval, demonstrate progress by all 25 admitted agents through useful model/tool steps or inspected deliverables; investigate intervals where the population is stalled. Record provider waits separately.

Proposed live mix is eight research/document agents, six software agents, five operational-investigation agents, and six agents in delegated multidisciplinary work. This is a scenario distribution, not permanent agent roles. Rotate work so each core capability is exercised.

| Scenario | Deliverable and inspection |
| --- | --- |
| Research and synthesis | Report with traceable sources; verify sampled claims and required coverage |
| Document production | Artifact meeting a supplied outline and constraints; check completeness and readability |
| Software work | Small repository-local change with relevant executable verification and reviewed diff |
| Operations investigation | Diagnosis against controlled telemetry/incidents with evidence and no unauthorized remediation |
| Delegated project | Parent integrates at least two specialist results; induce one contained failure and inspect recovery |
| Continuity | Resume after compaction, sleep, and restart while preserving specific facts and pending obligations |
| Dreaming and skills | Idle-only run yields to work; report matches actual revisions; a generated skill improves or reproduces a defined task |
| Human review | Submit, request changes, resubmit, reject stale acceptance, and accept while the worker is asleep |

Score requirements individually as met, unmet, or unverifiable. Initial proposed gate: at least 90 percent of live goals meet all required rubric items, with zero fabricated successful checks, hidden unresolved required outcomes, unauthorized effects, or lost accepted work. Publish every failure; a high average does not waive safety invariants. A human reviewer checks outputs against fixtures independently of the producing agent.

## Performance targets

All numbers below are draft targets to calibrate through prototypes. Report distributions, worst cases, and resource consumption, not only averages.

| Measurement | Proposed initial target and conditions |
| --- | --- |
| Durable message receipt | p95 under 300 ms of server processing with 25 agents working, excluding upload bytes and external inference |
| Authorized paginated reads | p95 under 250 ms of server processing for the reference data profile |
| State-to-console propagation | p95 under 500 ms from durable transition to receipt by a connected client on the recorded network |
| Warm admitted-work dispatch | p95 under 1 s when a ready lane, provider capacity, and authority are available |
| Hybrid memory search | p95 under 500 ms at 500–10,000 memories per queried agent; report embedding request latency separately |
| UI input responsiveness | p75 interaction latency under 200 ms during streaming and long-history navigation on the recorded mobile profile |
| Initial useful view | p75 under 2.5 s on a documented mid-range mobile device and constrained network profile |
| Visual stability | Layout-shift score under 0.1 for ordinary initial views; stable scroll anchors during history prepend |
| Long-session memory | No unbounded client buffer growth during the 60-minute run; rendered history stays bounded |
| Orchestrator restart | Control interfaces recover within 60 s when database, storage, and Kubernetes are healthy; uncertain effects may remain blocked longer |
| Dream yielding | Stop admission of new dream batches immediately on queued work; finish the current bounded batch within 10 s or expose a blocker |

Cold pod startup, browser startup, model generation, tool duration, and user-perceived task completion have separate measurements. Do not hide them by excluding them from the user journey. Publish warm and cold cases. UI numeric targets are baselines, not proof of exceptional visual quality; inspect actual desktop and mobile flows.

Memory quality fixtures should include exact identifiers, semantic paraphrases, contradictory evidence, human corrections, source deletion, and long gaps. Proposed retrieval gate: relevant evidence appears in the top ten for at least 95 percent of the curated set, with zero cross-agent retrieval. Compaction must preserve every explicitly tagged critical requirement and pending action in the fixture suite.

## Failure and concurrency qualification

Exercise each of these against real persistence and at least one qualified cluster profile:

1. Lose responses after message acceptance, child creation, credential rotation, upload publication, and result submission; retries return or reconcile the same operation.
2. Crash a runtime after dispatching an external write and before recording its result. Prevent automatic duplicate execution and expose uncertainty.
3. Partition an agent while it has finite authority. Revoke access or cancel centrally. It must stop new actions by its bound, and replacement must wait for safe settlement.
4. Partition or terminate the active orchestrator during a Kubernetes write. Prove a replacement cannot issue conflicting lifecycle actions while the old process may still act.
5. Race delegation, transfer, steering, completion, human acceptance, and cancellation. Verify ancestry, expected revisions, and terminal states.
6. Run concurrent memory corrections, extraction, dreaming proposals, and skill revisions. Reject stale changes without replaying successful external work.
7. Fill shared storage and workspace/journal capacity; lose filesystem synchronization; corrupt a ready blob. Report faults and retain evidence rather than acknowledging false success.
8. Restart browser or tool containers during takeover, downloads, local MCP invocation, and shell execution. Reconcile actual processes and session ownership.
9. Disable OIDC access, team membership, tool grants, and provider connections during active sessions. Measure revocation behavior and retain accurate offline limitations.
10. Interrupt backup capture, upgrade migration, and restore at each phase. Maintenance ownership and blockers survive restart; work does not resume on mere reconnection.
11. Delete a source or agent while extraction and publication are pending. Prevent stale resurrection and demonstrate reference, index, and backup-copy behavior.
12. Change a required runtime policy while agents are active, then make that runtime unavailable. Show pending application and block unsafe fallback.
13. Create an all-agents connection, use it from an existing and newly created agent without assignments, then switch to specific grants. Verify that only explicitly authorized agents retain new-operation access, discovery and credential delivery agree, disabled connections remain unusable, and policy propagation respects the recorded offline bounds. Approving a new tool must expose it to the current audience; merely discovering an unapproved tool must not.

Test actual egress from arbitrary installed programs, not only native HTTP tools. Test executor credentials from hostile local code. Optional sandbox runtimes need their own browser, storage, enforcement, and recovery results.

## Design consistency review

| Tension found | Draft treatment | Remaining evidence |
| --- | --- | --- |
| Unified memory versus scoped transcript access | Explicitly describe cross-conversation knowledge reuse; separate agents for knowledge compartments | Access and UX tests |
| Concurrent lanes versus one mutable workspace | Conservative resource claims for opaque mutations; separate browser sessions | Process/background-write qualification |
| One pod versus distinct runtime/tool network authority | Keep enforcement as a release gate with concrete candidate mechanisms | G1/G2 technical spike |
| Single active process versus partitions | Epochs plus stop-on-uncertainty; do not equate replica count or DB lock with fencing | G3 partition proof |
| Direct provider calls versus shared budgets | Durable reservations only for enabled limits; unknown usage remains reserved | Accounting and provider tests |
| Infinite-memory ambition versus finite context/storage | Explicit retrieval, compaction, retained sources, quotas and visible pressure | G7 quality and capacity runs |
| Agent autonomy versus human cancellation/review | Durable ancestry and review policy enforced outside model judgment | Lifecycle race suite |
| Retention versus deletion and backup restore | Separate active-data deletion, suppression, physical reclamation, and backup retention | Restore/deletion qualification |

The architecture is reviewable without claiming these engineering risks are solved. The networking and fencing gates are especially important: they may reveal that a chosen deployment constraint needs an explicit ADR amendment.

## Implementation sequence after design review

1. Qualify the highest-risk mechanisms with small disposable experiments: runtime/tool isolation and egress, executor fencing, native provider/auth paths, storage publication and restore. Record results before committing to broader scaffolding.
2. Build one vertical path: identity, one agent definition, durable message receipt, pod lifecycle, native inference, one bounded tool, recorded result, and agent completion. Include real PostgreSQL and a controlled external-effect target.
3. Add room routing, task lanes, delegation, cancellation, failure recovery, budgets, and human acceptance through the same Work interface.
4. Add source-preserving memory, hybrid retrieval, compaction, correction, idle dreaming, and the skill library with provenance and revision checks.
5. Add browser sessions and takeover, both MCP modes, account connections, tool discovery, triggers, and shared artifact flows.
6. Complete the conversation-first desktop/PWA experience, streaming, drafts, notifications, visible dreaming, inspection, and management. Validate incrementally throughout earlier steps.
7. Qualify backup, restore, upgrades, security, and the mixed 100/25 workload. Publish the supported provider/runtime/storage matrix and remaining limitations.

This sequence is not an estimate or an authorization to implement, deploy, or publish. It makes dependencies explicit so the first coding phase can be scoped from the reviewed design.
