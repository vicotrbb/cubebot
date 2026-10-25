# ADR 0062: Mixed organizational workloads for initial qualification

Status: Accepted

Date: 2026-10-23

Cubebot is intended for many kinds of agent work. Qualification centered only on coding or browsing would leave important execution paths and collaboration patterns untested.

Decision: Use mixed organizational work as the initial qualification emphasis, covering research, document production, software work, and operational investigation, including collaboration between specialists. The user selected this emphasis on 2026-10-23. The scale remains 100 registered agents with 25 working simultaneously under ADR 0061.

Meaningful work is measured by concrete outcomes with evidence. Representative scenario candidates are below; their exact fixtures, proportions, and scoring thresholds remain proposals.

| Workload | Goal | Evidence of completion |
| --- | --- | --- |
| Research | Compare alternatives against a supplied organization's requirements. | A source-backed comparison that addresses the requirements and distinguishes unresolved questions. |
| Document production | Turn source material into an operational guide or project brief. | A usable artifact checked for required content, factual consistency, and referenced inputs. |
| Software work | Diagnose and fix a reproducible issue in a test repository. | A focused change with tests demonstrating the intended behavior and relevant regression coverage. |
| Operational investigation | Investigate an injected fault in a controlled service environment. | A diagnosis supported by observations and a remediation or escalation consistent with the granted authority. |
| Cross-specialist collaboration | Combine delegated research, implementation, or review into a shared deliverable. | Traceable assignments, returned results, and a final integrated outcome with a responsible owner. |

The concurrent population must exercise multiple workload classes with substantive tool use and progress. The benchmark must not count task titles alone as diversity or count 25 delegates waiting on one blocked agent as useful throughput. Include independent work alongside collaboration; exact concurrency allocation remains open and is not fixed to five teams of five.

Use goal-driven scenarios to exercise relevant platform behavior: room routing, delegation, tool and skill discovery, memory retrieval, compaction, browser and terminal tools, and artifact publication. Each individual scenario need not invoke every subsystem. Memory continuity should be assessed across related tasks, and skill reuse should be tested on a later suitable task rather than inferred from successful registration. Idle dreaming runs in separate eligible phases, with reported memory and skill changes checked against actual records.

Controlled services and repositories provide known outcomes for repeatable tests, while live-provider trials establish behavior with actual inference. Keep those evidence classes labeled. Browser and integration actions that mutate external systems belong in authorized test environments; realistic qualification does not require publishing or modifying unrelated production resources.

Quality and system capacity are separate acceptance dimensions. A run with responsive infrastructure but incorrect deliverables fails the relevant quality criteria. A run that produces good outputs while losing accepted work or starving the console fails the relevant system criteria. Report scenario-level results rather than hiding a failed workload class inside an aggregate completion rate.

Ownership: The qualification plan defines fixtures, goals, expected evidence, and scoring. Domain modules expose operational records needed to trace outcomes. The selected workload mix does not narrow Cubebot's product scope to these examples or require a dedicated benchmark service.

Alternatives considered: Software-engineering-first qualification or research-and-browser-first qualification. Mixed work better represents the intended general-purpose installation and reveals competition between different kinds of tools and tasks.

Open details: Scenario selection and proportions; dataset sizes; collaboration topology; run duration; model and tool configurations; output rubrics; human evaluation; resource budget; latency and recovery thresholds; and repeatability requirements.

Validation scenarios: Produce inspectable deliverables across the selected classes; demonstrate overlapping progress by 25 agents; integrate delegated results without losing ownership; reuse relevant memory and skills on later work; keep console interactions responsive; and report workload-specific outcomes through recovery tests.
