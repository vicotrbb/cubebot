# ADR 0069: Dedicated TanStack Start frontend with Bun and Vite

Status: Accepted

Date: 2026-10-23

The web console and installed PWA are primary ways people work with Cubebot. Their architecture must support an exceptionally responsive, polished experience across desktop and mobile.

Decision: Run a dedicated frontend server using the React and TypeScript TanStack Start stack, Bun, and Vite. The user selected this deployment model on 2026-10-23 and explicitly required exceptional speed, performance, and visual quality. Bun is the intended frontend runtime and package tooling; Vite handles the framework's development and build integration. Exact versions and production hosting adapter require qualification.

Usage: A person opens a room and sees an initial rendered view, then follows streaming agent activity, sends messages, inspects artifacts, and opens task or memory details without losing conversation position. The same application adapts to mobile navigation and installed PWA use. Drafts, disconnection behavior, and notifications follow ADRs 0053 and 0054.

Ownership: The frontend owns presentation, routing, rendering, client interaction state, and frontend-specific request adaptation. The Rust orchestrator retains authoritative identity, permissions, task and agent state, memory, skills, storage, and Kubernetes operations. TanStack server functions may compose authorized orchestrator requests but must not become another implementation of these domain rules. The frontend receives no direct platform database access, Kubernetes lifecycle authority, installation encryption key, or provider credential store.

The dedicated server is an additional deployed frontend process; ADR 0010 still selects one Rust orchestrator process for backend application handling and Kubernetes reconciliation. Frontend restart must not cancel accepted agent work. Orchestrator unavailability must appear honestly in the UI instead of presenting cached data as live. Initial replica counts and independent frontend scaling remain open.

Rendering and data-loading design should use server rendering where it improves the initial experience, followed by efficient client navigation and live updates. Browser-dependent editors, terminals, and live viewers can load when needed. Avoid fetching the same initial data twice during hydration or creating serial request chains for independent data. Exact per-route rendering policy, query integration, stream transport, and ingress routing remain engineering choices.

Authentication remains platform-wide under ADR 0049. Server-rendered requests must preserve verified user context and pass through orchestrator authorization. Personalized response caches and query state must not cross users or installations. Hydrated state must contain only data the client is authorized to receive. Frontend presentation and disabled controls do not replace backend access checks.

Performance is an acceptance requirement, not an inference from framework choice. Establish measured budgets for initial rendering, route navigation, input responsiveness, stream update latency, long-conversation scrolling, memory use, and bundle size. Test representative mobile hardware and network conditions, large retained histories, and the 100-registered/25-working-agent workload from ADRs 0061 and 0062. Separate model response latency from UI and platform latency.

Engineering direction: split code by route and heavy feature; bound rendered history and event buffers; batch streaming updates so they do not rerender the whole console; preserve scroll anchors; paginate large collections; and isolate live browser rendering from chat interaction. Local pending feedback can be immediate, but must remain distinct from server acceptance. Do not optimistically claim success for consequential management operations or browser takeover.

Visual quality requires a coherent design system for typography, spacing, color, iconography, and interaction states. Desktop layouts and mobile navigation must be intentionally designed. Keyboard navigation, focus, contrast, touch targets, reduced motion, readable long-form content, and loading, empty, error, paused, dreaming, and disconnected states are part of the product contract. Choose a visual direction through concrete designs rather than assume a component library guarantees quality.

Alternatives considered: Serve a browser-rendered static frontend from the Rust orchestrator. The user selected a dedicated TanStack Start server despite the additional deployment process, prioritizing the frontend experience and this stack.

Related decision: [ADR 0070](0070-conversation-first-workspace.md) selects a conversation-first workspace with contextual access to goals, artifacts, memory, and skills.

Open details: Detailed navigation and visual direction; runtime adapter and version pins; typed client generation; query and rendering policy; streaming and ingress; session forwarding; caching; UI libraries; frontend replicas; measured performance thresholds; and desktop/mobile acceptance scenarios.

Validation scenarios: Render an authorized initial view; hydrate without duplicate requests or leaked state; keep typing and scrolling responsive during concurrent streaming; handle long histories with bounded memory; reconnect without duplicate messages; preserve drafts; enforce current access; maintain agent work through frontend restart; and demonstrate accessible, polished desktop and mobile flows.

References reviewed on 2026-10-23: [Bun's TanStack Start guide](https://bun.sh/guides/ecosystem/tanstack-start) documents production hosting options, and [TanStack selective SSR](https://tanstack.com/start/latest/docs/framework/react/guide/selective-ssr) documents per-route rendering control. [Core Web Vitals](https://web.dev/articles/vitals) supplies baseline loading, responsiveness, and stability measurements; Cubebot-specific thresholds remain to be set. No runtime or frontend implementation has been benchmarked.
