# ADR 0061: Initial qualification at 100 registered and 25 working agents

Status: Accepted

Date: 2026-10-23

Cubebot's long-term ambition includes thousands of agents. Its first production milestone needs a concrete, useful workload against which to evaluate correctness, capacity, and recovery.

Decision: Qualify the first production milestone with 100 registered agents and 25 agents working simultaneously on meaningful goals and workloads. The user supplied these targets on 2026-10-23. These are acceptance targets, not hard product limits or evidence of current capacity. The longer-term ambition remains unchanged.

Registered means durable agent identities and configuration with representative retained history, memories, skills, and workspace data. The remaining agents may sleep or be idle; their records must remain usable through the console. Empty placeholder identities alone are insufficient evidence for the intended workload.

Working means distinct agents executing admitted work, including normal waits for model and tool results within that work. Starting 25 pods or keeping 25 idle connections open does not satisfy the target. Evidence must show sustained overlapping progress toward concrete outcomes. A wholly stalled population does not count as successful sustained work. Duration and acceptable waiting behavior remain to be specified with the workload.

Agent count is distinct from active turn count. ADR 0005 permits multiple lanes per agent, so the qualification must report active agents, admitted lanes, in-flight model calls, and tool and browser concurrency separately. The 25-agent target does not silently set the installation's lane cap to 25 or require every agent to run all lanes continuously.

Meaningful goals need verifiable results: for example, a tested code change, a source-backed research report, or a diagnosed problem with supporting evidence. [ADR 0062](0062-mixed-organizational-qualification-workloads.md) selects mixed organizational work across research, document production, software work, and operational investigation, including specialist collaboration. Proposed coverage includes independent tasks and delegated room work, tool and skill discovery, memory use and compaction, and artifact publication. Sleeping-agent wakeups and idle dreaming need separate phases consistent with their lifecycle rules; dreaming must not run alongside active work in the same agent.

Qualification must exercise the system while users inspect progress and send new work. Record task outcome quality and correctness alongside queue delay, control-plane latency, completion time, storage throughput, resource use, and provider limits. Report hardware, storage class, model and connection configuration, data sizes, and task mix so results are reproducible. Thresholds and evaluation rubrics remain open.

Recovery qualification must include representative pod or orchestrator restarts, lost responses, and provider or tool failures, checking accepted-work preservation, uncertain outcomes, and duplicate effects. Capacity measurement and fault injection can use separate labeled runs rather than pretending all conditions are one benchmark.

Controlled providers and tools can establish repeatable system behavior. Live-provider runs with realistic tools and inspected deliverables are also required before claiming end-to-end production qualification. Report those evidence classes separately; simulated load alone does not establish useful autonomous work or subscription-provider compatibility.

Ownership: Scheduling and runtime modules expose execution measurements; domain modules provide verifiable outcomes and recovery records. The qualification plan ties these measurements to acceptance criteria. No new production service or benchmark framework is selected by this decision.

Alternatives considered: Begin at 1,000 registered agents with 100 or 1,000 concurrently working. The user selected a smaller initial target with meaningful workloads, making useful behavior and recoverability the first acceptance focus.

Open details: Workload proportions; run duration; task fixtures and quality rubric; data-volume baseline; browser and lane mix; hardware profile; latency and recovery targets; provider budget; and benchmark procedure.

Validation scenarios: Maintain 100 representative durable agents; demonstrate 25 distinct agents making overlapping progress; inspect concrete deliverables; retain responsive human interactions; wake sleeping agents; observe dreaming only when eligible; and recover representative interrupted workflows with honest outcome accounting.
