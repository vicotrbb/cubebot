---
name: create-agent
description: How to design a new agent for this platform and ask for it with agent_create. Load this before creating an agent, and whenever someone asks for a new agent, specialist, assistant, bot or worker, wants help setting up the platform's agents, or asks for "an agent that does X". Also covers renaming an agent or changing its cube with agent_update. Not needed to message or delegate to an agent that already exists.
---

# Create an agent

An agent is a long-lived colleague: its own conversation, computer, memory and standing instructions. A good one does one job so well that people stop thinking about it. A bad one is a vague helper nobody knows when to call, so most of this skill is about deciding what the agent is _for_ before you write a word of it.

Nothing you ask for exists until a person approves it in the console. Approval creates it as that person, so it gets exactly the access an agent they created themselves would get. Your job is to hand them a request so clear they can approve it without editing it.

## 1. Check whether a new agent is the answer

1. **Call `platform_list`** and read every agent's name and what it does. If one already owns this job, say so and suggest using it, or changing its instructions, instead. Two agents doing the same job split the memory and confuse the people who talk to them.
2. **A new agent earns its place** when the work recurs and has something of its own: its own sources, its own schedule, its own audience, or its own limits on what it may send or change. A one-off piece of work is not an agent; do it yourself or delegate it to an agent that exists.
3. **Several agents at once:** if the person wants a whole setup, plan the roster first (step 2 of the create-room-or-team skill), show it in one message, and ask once before asking for all of them.

## 2. Find out what you need, ask only the rest

Take everything the conversation already says. Then fill the gaps yourself where you safely can, and ask the person only what no amount of looking will tell you, usually:

- **The one job and its outcome:** "triages new GitHub issues into labels and a daily summary", not "helps with GitHub".
- **Whom it serves**, and where its work lands: which person, room or team.
- **On demand or on a schedule.** If scheduled: how often, and what it should do when there is nothing new (usually: stay quiet).
- **What it may do on its own and what it must only draft:** sending messages, posting, merging, paying, deleting.

Ask in one short message, never one question per turn. Do not ask about models, skills or tools: the platform's defaults apply and a person can change them later. Do not ask "shall I create it?" once the job is clear; the approval is that question. Pick small reversible details, such as the exact name, yourself and mention them.

## 3. Write the name

Short, specific and easy to address: "Triage", "Inbox Scout", "Release Notes". Never "Assistant", "Helper" or "Agent 2". Check `platform_list` so it does not collide with an existing name.

Give it a cube too: a paint, eyes and an accessory that fit the job and set it apart from the agents already there, so people tell them apart at a glance in the sidebar and the org chart. Agents of one team can share a paint and differ in accessory.

## 4. Write the instructions

The instructions are the agent's standing orders, read at the start of every turn for as long as it lives. Write them to it, in the second person, in Markdown, in this order:

1. **The job, in the first sentence**, with the outcome it produces. "You keep the support inbox at zero: every new email is answered, routed or flagged within the hour."
2. **What it never does**, even when asked, and who does it instead. Name the adjacent verb: a researcher does not publish, a drafter does not send, a monitor does not fix. Adjacent work belongs to another agent or a person.
3. **The concrete how:** which connection, inbox, repository, room or knowledge base page it works from, and in what order it does things. "Use your tools" tells it nothing.
4. **Limits:** what it does alone and what it drafts and asks about first. Its access is what the platform grants; these lines say how to use that access, they cannot widen it.
5. **How it checks its own work** before reporting, and when it stops and asks a person instead of guessing.
6. **What it reports and where:** the shape of its output and who reads it. If it runs on a schedule, tell it to create its own routine on its first turn and to stay quiet when there is nothing new.
7. **Voice, in two or three lines:** how it should sound to the people it serves. Concrete ("short, plain sentences; lead with the answer; no greetings") beats adjectives. Skip lines that change nothing, such as "be professional and thorough" or "you are an elite expert".

Keep facts that will change, such as people's names, current projects and deadlines, out of the instructions; they belong in the first message and in the agent's memory. Every line should change what the agent does. Most good instructions fit in 150 to 400 words.

## 5. Ask for it

Call `agent_create` with the name, instructions and cube. Then end your turn with a short note to the person: what you asked for, the one-line job, and that it waits for their approval in the console. At most five new agents can wait for approval at once; plan a larger roster in waves.

Do not ask again while a request is pending. Approved requests are made without telling you. Once a person has settled every request from your turn, one message lists any that were declined or failed. If it fails, the message says why: fix exactly that and ask once more. If it is declined, read the reason and change the design rather than resubmitting the same thing.

A new agent starts asleep and boots on its first message, so the platform never starts a crowd of computers at once.

## 6. After it exists

1. **Read it back** with `platform_list`: confirm the name and the start of its instructions are what you wrote. A message saying it was created is not the same as seeing it.
2. **Brief it with a real first task.** You can only message agents that share a room with you, so if the new agent should hear from you, ask for a room with both of you (create-room-or-team skill) before or alongside it. Otherwise, write the first message for the person to send. Either way the message stands on its own, because the new agent knows nothing of this conversation:
   - **Goal:** the first concrete task and why it matters.
   - **Context:** the people, sources and current state it needs.
   - **Limits:** anything off-limits for this task.
   - **Done looks like:** the finish line, so it knows when to stop and report.
3. **Read its first answer.** If it misunderstands its job, the instructions are wrong, not the agent: tell the person what to change in its settings, rather than patching it with ever longer briefings.

## Before you report

| Claim                  | What proves it                                            |
| ---------------------- | --------------------------------------------------------- |
| It is needed           | `platform_list` showed no agent that already owns the job |
| It was created         | You saw it in `platform_list` after the approval message  |
| It understands its job | Its answer to a real first task, not its greeting         |
| Its access is right    | The person confirmed what it may send or change           |

Report plainly: the agent's name and one-line job, what you chose without asking, what its first answer showed, and what is left for the person, such as granting a connection that is limited to specific agents, or a setting only the console changes.

## Renaming or recolouring an agent

Call `agent_update` with the agent's exact current name and a new name, a new cube, or both; null keeps what it has. It waits for a person's approval like a creation and needs a person who may manage that agent. Rename only when the job changed or the name confuses people, and say why in your note.
