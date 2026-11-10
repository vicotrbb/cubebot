# An agent creation skill: research and proposal

Researched 2026-11-09. Nothing here is decided. The questions at the end are for the user.

The goal is a default skill, shipped beside `coding` and `design` in `apps/orchestrator/skills/`, that teaches a Cubebot agent to create another agent well, and later rooms and teams. This note covers what the community actually uses, which candidate to build on, and an outline of the skill mapped to Cubebot's real fields.

## What the names in the request turned out to be

- **"open bot"** is OpenBot, [nightly-labs/openbot](https://github.com/nightly-labs/openbot): "a local-first desktop workspace for persistent AI teammates" that runs Codex, Claude and Grok. Cubebot's skill model was copied from it: [Reusable skills created by humans, agents, and dreaming](../adr/memory.md#reusable-skills-created-by-humans-agents-and-dreaming) cites revision `cfee2971f29cdb7c16272e60ae912de7fbb30693`. This note covers both that revision and `main` at `22cd028e7c99c41e49bbab434dbda63ff9e56304` (2026-11-10 00:50 +0100). The agent design skill below is on `main` only. It was added in [#2888 on 2026-10-23](https://github.com/nightly-labs/openbot/commit/41444f7b2037410b6c6ae7ecad52058f5ebd4f51), and `cfee2971` has no `marketplace/production-catalog` at all. OpenClaw is covered separately as its own ecosystem. The request's "formerly Clawdbot/Moltbot" lineage is plausible from ClawHub listing names such as `clawdbot-skill-creator` ([awesomeskills.dev](https://www.awesomeskills.dev/en/skill/clawdbot-clawdbot-skill-creator)), but I did not confirm it against an OpenClaw source.
- **"grok bot"** is a real product, xAI's [Grok Bot](https://x.ai/bot), and not a misremembered name. The docs call it "your proactive AI teammate", "an agent with access to a real computer in the cloud", where "multiple Bots can work together" ([overview](https://docs.x.ai/grok-bot/overview.md)). OpenBot ships an export skill that runs inside Grok Bot and migrates its agents ([resources/agent-import/grok-bot/SKILL.md](https://github.com/nightly-labs/openbot/blob/22cd028e7c99c41e49bbab434dbda63ff9e56304/resources/agent-import/grok-bot/SKILL.md); [README](https://github.com/nightly-labs/openbot/blob/22cd028e7c99c41e49bbab434dbda63ff9e56304/README.md#import-agents-from-grok-bot)). The agent designer users reach for there is **dr eggbot**, which Grok Bot's own docs recommend: "To set up several Bots at once, start from dr eggbot" ([How Grok Bot works](https://docs.x.ai/grok-bot/how-grok-bot-works.md)). The [marketplace page](https://x.ai/bot/marketplace/bots/dr-eggbot-v2) credits it to Lauren Tan. OpenBot's Dr. Eggbot agent and its Agent Designer skill are the same design: the listing texts match almost word for word.
- **Hermes** is Nous Research's [Hermes Agent](https://github.com/NousResearch/hermes-agent), inspected at `74dde239c8a9a5ae72af1c68b9e74d0b6aa798a5` (2026-11-09).

## What a Cubebot agent definition contains today

The skill has to map onto these fields, so here is what the code actually accepts.

- **Create** (`CreateAgent` in `apps/orchestrator/src/agents/definitions.rs`): `name` (non-empty, at most 255 bytes, no control characters), `instructions` (at most 65,536 bytes, no NUL), an optional `avatar` (`paint`, `eyes`, `accessory`, optional `#rrggbb` `color`) and an optional `voice`. Creation is idempotent by `request_id`, and every `installs_by_default` skill is installed in the same transaction.
- **Update** (`UpdateAgent` in `apps/orchestrator/src/agents/revisions.rs`, `POST /api/agents/{id}/definition`): `name`, `instructions`, `idle_policy`, `idle_delay_seconds` (60–86,400), `dreaming_model`, `task_lanes` (1–4), `broadcast` (default off), `knowledge_base` (default on) and `avatar`, all checked against `expected_revision`.
- **Attached elsewhere**, through their own routes in `apps/orchestrator/src/api/`: skill installations (`/api/agents/{agent}/skills`), connection access (`/api/agents/{id}/connections`), MCP servers, vault items, routines, budget and speech.
- **Model.** The ADR says each agent has a default model plus optional overrides ([Default agent model with optional purpose overrides](../adr/agents.md#default-agent-model-with-optional-purpose-overrides)). In the code, the main model comes from the installation's provider connection. The definition carries only `dreaming_model`, and neither `CreateAgent` nor `UpdateAgent` has a per-agent default model.
- **Access.** Connections default to every eligible agent, with explicit grants available per connection ([Installation-wide connection access by default](../adr/identity.md#installation-wide-connection-access-by-default)). The grant is the only safety control, because there is no approval queue ([Autonomous tool use within granted permissions](../adr/agents.md#autonomous-tool-use-within-granted-permissions)).
- **Agent-initiated creation is decided but not built.** [Agent creation as a granted capability](../adr/agents.md#agent-creation-as-a-granted-capability) and [Persistent and task-bound created agents](../adr/agents.md#persistent-and-task-bound-created-agents) are accepted. The "creation interface" is still an open detail, and the agent runtime has no `agent_create` or agent-listing tool: its tools include `skill_*`, `routine_*` (which acts on the agent itself: "Create a routine for yourself"), `team_member`/`team_remove`/`team_update`, `delegate`, `send_message`, `broadcast` and `people_lookup`. New agents start with fresh memory and an explicit briefing ([Fresh memory with an explicit briefing](../adr/memory.md#fresh-memory-with-an-explicit-briefing)). Teams exist, but agents creating or deleting teams is an open detail ([Agent teams](../adr/agents.md#agent-teams)).

So the skill's prerequisite is a creation tool, at minimum `agent_create` plus a way to read agents back (`agent_list` or `agent_read`). Without one, the skill can only draft a definition for a person to paste into the console.

## Candidates

Star and download counts were read from the GitHub API and the ClawHub API on 2026-11-09.

### 1. OpenBot "Agent Designer" skill, with the Dr. Eggbot agent

- **Source:** [marketplace/production-catalog/skills/agent-designer/SKILL.md](https://github.com/nightly-labs/openbot/blob/22cd028e7c99c41e49bbab434dbda63ff9e56304/marketplace/production-catalog/skills/agent-designer/SKILL.md). It is paired with [eggbot-getting-started](https://github.com/nightly-labs/openbot/blob/22cd028e7c99c41e49bbab434dbda63ff9e56304/marketplace/production-catalog/skills/eggbot-getting-started/SKILL.md) and the Dr. Eggbot entry in [catalog.json](https://github.com/nightly-labs/openbot/blob/22cd028e7c99c41e49bbab434dbda63ff9e56304/marketplace/production-catalog/catalog.json). The same design ships as Grok Bot's [dr eggbot](https://x.ai/bot/marketplace/bots/dr-eggbot-v2).
- **What it does:** an agent designs and creates another agent through tools, then verifies it.
- **Structure and techniques:**
  - An agent is four things: **one job** (one sentence), **anti-jobs** ("what it never does, even if asked. Adjacent work goes to a different agent"), **voice**, and **wake** (on demand, a routine, or both, "quiet when there is nothing to report").
  - The intake rule: "Ask only preference questions that no experiment can settle." It does not ask for tools, skills or model "if you can copy a working sibling: call `list_agents` and `read_agent`". It does not ask "should I create it?" once the job is clear. A reversible detail is picked and announced.
  - Non-coding agents get "the ONLY job, named in the first sentence", "never do the adjacent verb (a mentions scout does not post, a drafter does not send)", and "the concrete how (which service, which inbox, which channel), not 'use whatever tools you have'".
  - After creating, it reads the live profile back with `read_agent`: "Do not trust the tool result alone." If the job needs a routine, it creates the routine for the new agent and confirms it with `list_routines`.
  - "There is no tool to delete an agent. Create one only when the job is real."
- **Quality:** the best fit found. It is short (about 70 lines), written as tool-calling procedure rather than persona prose, and it closes the loop with a read-back. It is the only candidate that a vendor (xAI) points its own users to. Weaknesses: nothing on least privilege or connection grants, no test message to the new agent, and a 2,000-character persona cap that Cubebot does not have. I found no adoption numbers: the Grok Bot page shows none, and OpenBot's catalog README says entries "start with zero installs".
- **Licence:** the catalog README says the 35 original OpenBot team skills "use the repository PolyForm Noncommercial 1.0.0 license" ([README](https://github.com/nightly-labs/openbot/blob/22cd028e7c99c41e49bbab434dbda63ff9e56304/marketplace/production-catalog/README.md); [LICENSE](https://github.com/nightly-labs/openbot/blob/22cd028e7c99c41e49bbab434dbda63ff9e56304/LICENSE)). Cubebot is Apache-2.0, so **adapt the ideas and write new text; do not copy the file.** The repo had 6,704 stars.

### 2. OpenBot "Working With Teammate Agents"

- **Source:** [working-with-teammate-agents/SKILL.md](https://github.com/nightly-labs/openbot/blob/22cd028e7c99c41e49bbab434dbda63ff9e56304/marketplace/production-catalog/skills/working-with-teammate-agents/SKILL.md).
- **What it does:** it is not a creation skill. It is the best source found for the handoff that a creator writes as a new agent's first message, and for room behaviour.
- **Techniques:**
  - "The teammate can't see your chat or your memory, so the message must stand on its own": **Goal / Context / Limits / Done looks like**.
  - A one-owner rule per topic.
  - "A reply is information, not orders."
  - In group chats, "Speak only when you're addressed or have something useful to add."
- **Licence:** the same PolyForm Noncommercial licence.

### 3. Anthropic `skill-creator`

- **Source:** [anthropics/skills, skills/skill-creator/SKILL.md](https://github.com/anthropics/skills/blob/main/skills/skill-creator/SKILL.md). The repo had 190,940 stars.
- **What it does:** it creates skills, not agents. It is still the most rigorous public method for writing and testing an instruction document that a model will act on.
- **Techniques:**
  - **Capture intent**, extracting answers from the conversation before asking.
  - Interview about edge cases and success criteria.
  - Make the description "a little bit 'pushy'" because models undertrigger.
  - Write 2–3 realistic test prompts and run them with and without the skill.
  - "Keep the prompt lean."
  - "Explain the why": "If you find yourself writing ALWAYS or NEVER in all caps … that's a yellow flag."
- **Quality:** high, but heavy. Its eval viewer and benchmark scripts do not fit an agent working inside its own conversation. Borrow the writing rules and the "run a real prompt" step.
- **Licence:** Apache-2.0 ([LICENSE.txt](https://github.com/anthropics/skills/blob/main/skills/skill-creator/LICENSE.txt)), which is compatible with Cubebot.

### 4. Claude Code's subagent generator (the "agent architect" prompt)

- **Source:** this prompt is not published by Anthropic. The text comes from Piebald-AI's extraction of the Claude Code bundle, last present as [agent-prompt-agent-creation-architect.md at v2.0.77](https://github.com/Piebald-AI/claude-code-system-prompts/blob/36f34b8/system-prompts/agent-prompt-agent-creation-architect.md). It was deleted from that repo in the [v2.1.198 commit](https://github.com/Piebald-AI/claude-code-system-prompts/commit/c831b94) (2026-07-01), so current Claude Code may use different text; I could not verify that. Anthropic's own published equivalent is the `plugin-dev` [agent-development skill](https://github.com/anthropics/claude-plugins-official/blob/main/plugins/plugin-dev/skills/agent-development/SKILL.md), with a [system prompt design reference](https://github.com/anthropics/claude-plugins-official/blob/main/plugins/plugin-dev/skills/agent-development/references/system-prompt-design.md) and a copy of the creation prompt under `references/` (Apache-2.0; the repo had 46,634 stars).
- **What it does:** it turns a description into `identifier`, `whenToUse` and a second-person `systemPrompt`.
- **Techniques:**
  - Steps: extract core intent, design an expert persona, write the instructions "with boundaries, methodology, edge cases, output format", then add "quality control mechanisms and self-verification steps" and "clear escalation or fallback strategies".
  - Identifier rules: 2–4 hyphenated words, "avoids generic terms like 'helper' or 'assistant'".
  - `whenToUse` "starting with 'Use this agent when...'", with examples.
  - The plugin-dev skill adds "Limit tools to minimum needed (principle of least privilege)", `model: inherit` as the recommended default, and specific-not-vague rules ("✅ Provide file:line references … ❌ Show where issues are").
- **Quality:** good structural checklist. It is one-shot with no interview, no duplicate check and no verification of the created agent, and its "elite … expert" persona framing is the style OpenClaw and Hermes now argue against (below).
- **Licence:** the extracted prompt is Anthropic's proprietary text; use it as a reference only. plugin-dev is Apache-2.0.

### 5. Grok Bot's documented guidance

- **Source:** [Create and manage Bots](https://docs.x.ai/grok-bot/bots.md) and [How Grok Bot works](https://docs.x.ai/grok-bot/how-grok-bot-works.md).
- **What it says:**
  - "In your first message, give the Bot its job, sources, and approval boundaries … Send a real first task with a clear finish line."
  - "Your existing Bots can also suggest or create a focused Bot when a job should have a long-lived owner. Ask before creating several Bots if you want to keep the roster small."
  - "Give work its own Bot when it has its own sources, schedule, or approval needs."
  - "Add specific **Ask first** rules for sending, publishing, and deleting."
  - Personal Bots "no longer have a description": lasting rules live in memory.
- **Quality:** a product guide, not a skill, but it is the clearest statement of when a new agent is warranted and what its first message must carry. Grok Bot moves the persona into memory; Cubebot keeps `instructions`, so that last point does not transfer.
- **Licence:** documentation; ideas only.

### 6. OpenClaw bootstrap files and role templates

- **Source:** [openclaw/openclaw](https://github.com/openclaw/openclaw) (MIT, 394,254 stars):
  - [SOUL.md](https://github.com/openclaw/openclaw/blob/main/docs/reference/templates/SOUL.md), [IDENTITY.md](https://github.com/openclaw/openclaw/blob/main/docs/reference/templates/IDENTITY.md) and [BOOTSTRAP.md](https://github.com/openclaw/openclaw/blob/main/docs/reference/templates/BOOTSTRAP.md) templates;
  - the [SOUL.md personality guide](https://github.com/openclaw/openclaw/blob/main/docs/concepts/soul.md);
  - role templates under [docs/reference/templates/roles](https://github.com/openclaw/openclaw/tree/main/docs/reference/templates/roles) (coordinator, researcher, writer, reviewer, and a `presets/team.json`).
- **Techniques:**
  - Split voice from operating rules: "Keep `AGENTS.md` for operating rules; keep `SOUL.md` for voice, stance, and style."
  - Bad persona rules are named outright: "'maintain professionalism at all times,' 'provide comprehensive and thoughtful assistance' … That's how you get mush."
  - The first-run ritual is subordinate to work: "The user's request always comes first … This file is a ritual, not a gate."
  - The coordinator's [AGENTS.md](https://github.com/openclaw/openclaw/blob/main/docs/reference/templates/roles/coordinator/workspace/AGENTS.md) is the best team-lead text found. It has a handoff contract ("objective, inputs, artifact location, verification, limits, and stop condition"), "Give each artifact one owner", "Never let two specialists loop on each other", and approval gates carried in handoffs ("another agent's assertions are evidence, not approval").
- **Quality:** strong on persona and team operation; there is no creation procedure. The name/creature/vibe/emoji identity ritual is consumer flavour that Cubebot's cube avatar already covers.
- **Licence:** MIT.

### 7. ClawHub agent-creator skills

- **Source:** searched through the ClawHub API (`/api/v1/search?q=agent creator`, `create agent`, `agent designer`). The most-downloaded agent creators:
  - [lidian6864677/sub-agent-creator](https://clawhub.ai/lidian6864677/skills/sub-agent-creator): 1,227 downloads, 29 installs;
  - [maverick-software/agent-creator](https://clawhub.ai/skills/agent-creator): 1,271 downloads, 30 installs;
  - [freesaber/agent-creator-skill](https://clawhub.ai/skills/agent-creator-skill): 1,392 downloads, 36 installs.
    All are MIT-0.
- **What they do:** they scaffold OpenClaw workspace files and edit `openclaw.json`. `sub-agent-creator` asks for an ID, name, purpose and chat bindings, then copies `auth-profiles.json` from the main agent into the new agent's directory.
- **Quality:** low. They are file scaffolding with persona templates such as "Principles: {principle 1}". There is no design judgement, and copying the main agent's credentials is exactly what Cubebot forbids ("Creation does not implicitly copy the creator's … credentials"). Search results flag some agent-creator listings on ClawHub as suspicious ([example](https://clawhub.ai/skills/agent-creator-skill)); I did not confirm the flag through the API. Adoption is small: no agent creator passed 40 installs. **Nothing to borrow except the anti-pattern.**

### 8. Hermes Agent

- **Source:** [Which file does what](https://github.com/NousResearch/hermes-agent/blob/main/website/docs/user-guide/which-file-does-what.md), [Personality & SOUL.md](https://github.com/NousResearch/hermes-agent/blob/main/website/docs/user-guide/features/personality.md), [Profiles](https://github.com/NousResearch/hermes-agent/blob/main/website/docs/user-guide/profiles.md), [Bot Mode](https://github.com/NousResearch/hermes-agent/blob/main/website/docs/user-guide/bot-mode.md), [Delegation](https://github.com/NousResearch/hermes-agent/blob/main/website/docs/user-guide/features/delegation.md) and the [role archetypes](https://github.com/NousResearch/hermes-agent/blob/main/optional-skills/creative/kanban-video-orchestrator/references/role-archetypes.md). MIT, 262,070 stars.
- **What it has:** no agent-creation skill. Profiles and Bots are created in the CLI or desktop dialog (name, clone-from, description, model, skills, toolsets, MCP servers, SOUL.md).
- **Techniques worth taking:**
  - Clean separation: "SOUL.md is who the agent _is_ … USER.md is who _you_ are … MEMORY.md is what the agent has _learned_ … AGENTS.md is what the _project_ needs."
  - "A good SOUL file is stable across contexts … focused on communication and identity, not task-specific instructions."
  - The delegation guide's rule that a child starts with "a completely fresh conversation", so goal and context must be complete, with a BAD/GOOD example.
  - The role archetypes "Compose a team from this list, don't clone a fixed roster". Each role lists toolsets, skills and outputs, and the director's "SOUL.md rules **forbid** execution."
- **Quality:** the best guidance on what belongs in the persona and what belongs in memory, and the role-archetype file is a good model for create-team. It is not a creation procedure.

### 9. CrewAI and Letta persona guidance

- **CrewAI** [Crafting effective agents](https://docs.crewai.com/en/guides/agents/crafting-effective-agents): a role should be "specific and specialized", a goal "clear and outcome-focused", and a backstory should form a "cohesive persona". It says "80% of your effort should go into designing tasks, and only 20% into defining agents", and recommends testing on representative tasks and refining. It has nothing on tool privilege. This is useful as a check that `instructions` name an outcome and not only a role.
- **Letta** [memory blocks](https://docs.letta.com/guides/agents/memory-blocks): `persona` and `human` blocks, and "it is crucial to provide a good `description` field". This is marginal for Cubebot, whose memory model is already decided.

## Recommendation

1. **Base: OpenBot's Agent Designer** (candidate 1), adapted in new words because of its licence. It is the only candidate that is a procedure for an agent with creation tools, which is exactly Cubebot's situation. Its shape (one job, anti-jobs, voice, wake), its intake rule (ask only what no experiment can settle; copy a working sibling), and its "read it back, don't trust the tool result" check are the core. Grok Bot's own docs point users at the same design.
2. **Borrow from the runners-up:**
   - **Grok Bot docs:** the test for whether a new agent is warranted ("its own sources, schedule, or approval needs"); a first message carrying the job, sources and approval boundaries plus a real first task with a finish line; asking before creating several agents.
   - **OpenBot Working With Teammate Agents:** Goal / Context / Limits / Done looks like for the briefing, matching Cubebot's [Fresh memory with an explicit briefing](../adr/memory.md#fresh-memory-with-an-explicit-briefing).
   - **Claude Code architect and plugin-dev:** self-verification and escalation steps inside the new agent's instructions; specific-not-vague wording; least privilege; inheriting the model unless there is a reason.
   - **Anthropic skill-creator:** explain the why instead of capital-letter MUSTs, keep it lean, and test with a realistic prompt.
   - **OpenClaw and Hermes:** voice versus operating rules versus memory, and banned "mush" lines. The OpenClaw coordinator and Hermes role archetypes feed create-team.
3. **Reject:** the ClawHub agent creators (scaffolding, credential copying) and the "elite expert" persona framing.

## Outline of the Cubebot `create-agent` SKILL.md

This is an outline in the house style of `apps/orchestrator/skills/coding/SKILL.md`: second person, plain prose, short sections, and a closing evidence table. It is not the final text.

**Front matter.**

- `name: create-agent`.
- `description`: what it does and when to load it ("Load this before creating an agent, or when someone asks for a new specialist, assistant or worker, or for an agent that does X. Not needed to message or delegate to an agent that already exists.").

**1. Decide whether a new agent is warranted.**

- List the agents you can see and read the closest one first. If one already owns the job, recommend it or propose changing its instructions instead. Cubebot agent names are not unique, so a duplicate name is legal but confusing.
- A new agent earns its place when the work has its own sources, schedule, permissions or voice, and will recur (Grok Bot's test).
- A one-off piece of work goes through `delegate`, not a new agent.
- With no creation tool or grant, draft the definition for a person to create in the console, and say that is what you did.

**2. Interview, or infer.**

- Take everything the conversation already says.
- Ask only what you cannot find out or safely default, one question at a time. That usually means the one job, whom it serves, whether it acts on a schedule or on demand, and what it may send or change.
- Do not ask about model, skills or tools when a working sibling can be copied.
- A reversible detail (name, cube colour, voice) you pick and announce.
- An irreversible or authority-widening choice you ask about: write access, sending, broadcast, persistent lifetime.

**3. Write the definition.** This maps onto `CreateAgent` and `UpdateAgent`.

- **`name`:** short and functional, not "Assistant" or "Helper". At most 255 bytes, one line.
- **`instructions`:** second person, in this order:
  - (a) the one job, in the first sentence, with the outcome it produces;
  - (b) anti-jobs: the adjacent verbs it never does, and who does them instead;
  - (c) the concrete how: which connection, inbox, repository or room, not "use your tools";
  - (d) approval boundaries: what it drafts versus sends. Grants are the real control, and these lines only describe them;
  - (e) how it checks its own work and when it stops and reports;
  - (f) the output shape;
  - (g) voice in two or three lines, with no "mush" lines (OpenClaw's list).
  - Keep facts about people and projects out of `instructions`; they belong in the briefing and in memory.
  - Keep it to what changes behaviour. The 64 KiB limit is not a target.
- **`avatar` and `voice`:** pick them, give them a reason, and say what you picked.
- **Settings:**
  - `task_lanes` at the default unless the job is parallel;
  - `broadcast` off;
  - `knowledge_base` on only if the job reads it;
  - `idle_delay_seconds` and `dreaming_model` at their defaults unless there is a stated reason.

**4. Choose the model.** Inherit the installation's provider connection. Today that is the only option for the main model, since `dreaming_model` is the only per-agent override. Name a different model only when the job needs a capability the default lacks, and say why.

**5. Skills.** Default skills install automatically. Add a library skill only when the job uses it on most turns. If none fits, do the work first and save a skill later; do not write one speculatively.

**6. Connections, MCP servers, vault: least privilege.**

- Installation-managed connections reach every eligible agent unless a connection is in explicit-grant mode. Say which connections the new agent will be able to use, and ask before granting anything outside the job.
- The lowest permission level that does the job: Read before Read and write before Full.
- Never pass on your own credentials or memories, and never grant creation rights onward unless the creation grant allows it.

**7. Routines.** If the job is scheduled, give the coarsest useful schedule and an instruction that stays quiet when there is nothing new. `routine_create` only creates routines for the calling agent, so either the new agent creates its own routine on its first turn (put that in the briefing), or this needs a creation-time routine field.

**8. Create, read back, brief, test.**

- Create with a stable `request_id`, so a retry does not create a duplicate.
- Read the agent back and compare it with what you intended.
- Send the first message as a briefing: Goal / Context / Limits / Done looks like, plus a real first task with a finish line.
- Read its reply. A wrong reply means the instructions are wrong: fix them with an update, not with more briefing.

**9. Report.** Report the name, the one-job line, its access, what you picked without asking, what the test showed, and how to change or retire it. Use the coding skill's evidence table adapted:

| Claim           | What proves it                    |
| --------------- | --------------------------------- |
| Created         | Read back, this turn              |
| Works           | Its reply to the test task        |
| Access is right | The connection list you read back |

**Tools the skill needs that do not exist yet:**

- `agent_create` (name, instructions, avatar, voice, settings, lifetime, skills, connection grants, all validated against the creation grant);
- `agent_list` and `agent_read`;
- optionally, routine creation for another agent.

## Notes for `create-room` and `create-team`

- **Teams** already have a data model and agent tools for managers ([Agent teams](../adr/agents.md#agent-teams)). A `create-team` skill would mostly teach composition and the shared text. The best sources:
  - the OpenClaw coordinator `AGENTS.md`: one owner per artifact, all results through the lead, a handoff contract, approval carried in handoffs, and escalation after one clarified follow-up;
  - the OpenClaw `presets/team.json`: one coordinator plus researcher, writer and reviewer;
  - the Hermes role archetypes: compose roles from the brief rather than cloning a roster; a director whose persona forbids executing; a list of toolsets, skills and outputs per role;
  - Grok Bot's primary-Bot pattern: one point of contact that routes to single-job Bots.
    The skill should create members with `create-agent`, write each member's title and role in one or two lines (the team text already carries shared context), and keep teams small. Agents creating teams is still an open ADR detail, so the tools come first.
- **Rooms.** No source has a room-creation skill. Grok Bot and Hermes rooms are made by people in the UI. The OpenBot Grok Bot export maps a group chat to a channel with `name`, `title`, shared `instructions`, `members` and an optional `lead` ([format](https://github.com/nightly-labs/openbot/blob/22cd028e7c99c41e49bbab434dbda63ff9e56304/resources/agent-import/grok-bot/SKILL.md#format)). That is a reasonable checklist for what a room needs. The behaviour rule worth writing into room instructions is OpenBot's "speak only when addressed or useful; do not repeat another agent".

## Open questions for the user

1. **Who may create?** Should the skill be installed by default on every agent (`installs_by_default`), or only on agents holding the creation grant, once that grant exists? Default-installing it on agents that cannot create would teach a procedure they cannot run.
2. **Interface first?** Should this work start by building `agent_create`, `agent_list` and `agent_read` (the ADR's open "creation interface"), or should the skill ship first in draft-for-a-human mode?
3. **Lifetime:** keep the ADR's default of task-bound workers for agent-initiated creation? The code has no lifetime field yet.
4. **Grants at creation:** may a creating agent narrow the new agent's connection access only, or also widen it within its own grant? The ADR forbids widening beyond the creation grant, but the default for installation-wide connections is "all eligible agents".
5. **Model:** should a per-agent default model, which is in the ADR but not in `CreateAgent`, be part of this work?
6. **A designer agent:** should Cubebot also ship a "Dr. Eggbot"-style default agent that holds this skill, or only the skill?

## What I could not verify

- Whether current Claude Code still uses the architect prompt. The extraction stops at v2.0.77, and it was removed from the extract repo on 2026-07-01.
- Usage numbers for Dr. Eggbot. Neither Grok Bot nor OpenBot shows install counts.
- ClawHub's own scan verdicts. They were only seen through search results.
- The OpenClaw "formerly Clawdbot/Moltbot" lineage, which was not confirmed from an OpenClaw source.
