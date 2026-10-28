# Product

<!-- impeccable:product-schema 1 -->

## Platform

web

## Stack

Landing site: standalone `site/` folder built with Astro, as few dependencies as possible, static output deployable anywhere (user decision, 2026-10-26). The product console is a separate TanStack Start app in `apps/web`; its current styling is scaffolding and is not design authority.

## Users

Primary: self-hosting developers who run their own Kubernetes cluster or homelab and want persistent AI agents on infrastructure they control. They read architecture docs, judge projects by engineering honesty, and decide whether to follow or contribute.

Secondary: engineering teams and small organizations evaluating a shared agent platform for mixed organizational work (rooms, delegation, shared skills).

## Product Purpose

Cubebot is a Kubernetes-native home for persistent AI agents. Install it in a namespace, create agents, give them tools and responsibilities, and work alongside them through a web console and installable PWA. Each agent keeps a lasting identity: configuration, one permanent conversation, unified memory, a workspace volume, and skills, all surviving restarts, upgrades, and pod replacement.

Success for the landing page: a visitor understands what Cubebot is within seconds, believes the design is serious, knows it is under active development, and stars or watches the GitHub repository.

## Positioning

An agent in Cubebot is a durable Kubernetes workload with its own pod, persistent volume, recovery journal, and finite execution authority, orchestrated by a single Rust control plane with PostgreSQL as the source of truth. The engine is native Rust, not a wrapper around a vendor CLI. Memory is one pool per agent across every conversation and room, with idle "dreaming" that consolidates it visibly. Work survives interruption and uncertain outcomes stay visible instead of being replayed blindly. Neighboring products (desktop-host agent apps such as OpenBot) run on one computer; Cubebot targets infrastructure the user or organization operates.

## Operating Context

Deployment: one namespace per installation, one active Rust orchestrator, one pod per running agent with separate trusted runtime and tool containers, PostgreSQL for structured state, per-agent workspace PVCs, Helm packaging planned. Providers: OpenAI today, Anthropic and OpenAI-compatible endpoints planned, credentials installation-managed and encrypted. Repository: github.com/vicotrbb/cubebot (private as of 2026-10-26; the GitHub CTA assumes it becomes public before the site ships).

## Capabilities and Constraints

Implemented and locally validated (2026-10-26): built-in sign-in, invitations, OIDC, teams and scoped grants; durable agent definitions with revisions and lifecycle intent; Kubernetes pod/PVC reconciliation with guarded drain and termination; runtime identity bootstrap and private recovery journal; encrypted OpenAI connection; one permanent text conversation per agent with queued, retrying, waiting, and committed turns; a minimal console.

Planned (milestone plan, Tasks 2–9): cluster fencing and egress qualification; multi-provider adapters and execution lanes; tools, browser, terminal, and MCP; rooms, mentions, delegation, steering; hybrid memory retrieval, dreaming, shared skill library; integrations and triggers; full console and PWA; backup, restore, and scale qualification (first target: 100 registered agents, 25 progressing).

Constraints the site must respect: never claim unqualified capabilities as shipped; distinguish implemented, in progress, and planned; no pricing, customers, benchmarks, or testimonials exist; no public release yet. Complexity gate: every authored function in the repo, including site scripts, must stay at cyclomatic complexity ≤ 15.

Terminology: agent, orchestrator, runtime, sandbox, room, lane, dreaming, skill, grant, connection, workspace, journal.

## Brand Commitments

Name: Cubebot. Logo/icon: a literal cube with two eyes, cozy and warm with personality, rendered with real 3D depth (user reference: a rounded lilac square with two black scribbled eyes; ours should feel more three-dimensional and characterful). Core color: orange, on a light, cozy ground. The site should carry animation and 3D presence. Inspiration for page ambition and structure: openbot.run. Tone from the manifesto: plain, confident, engineering-honest, warm rather than hype. The current `apps/web` palette and tokens are explicitly not binding. Decisions made for the site are intended to carry into the future console.

## Evidence on Hand

- MANIFESTO.md, ARCHITECTURE.md (with a deployment diagram), 82 accepted ADRs under docs/adr, software specification and qualification plan under docs/design.
- Current state and open qualification gaps at docs/STATUS.md; dated evidence in docs/development/*.md and git history (source for the timeline section).
- No product screenshots worth showing yet, no logo asset, no customers, no benchmarks, no press. The site must not fabricate any of these; illustrative UI must be labeled as illustration.

## Product Principles

- Honesty is the brand: show what runs, what is in progress, and what is planned, with dates.
- Durability over demos: the story is agents that keep identity, memory, and work across failures.
- Infrastructure you control: Kubernetes, your database, your providers, your grants.
- Approachable depth: one person can run it; an organization can grow into it.
- Warm, not hype: the cube has a face because the product is meant to be lived with.
