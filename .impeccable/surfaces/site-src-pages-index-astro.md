---
version: 1
slug: 'site-src-pages-index-astro'
primary_target: 'site/src/pages/index.astro'
related_targets: ['site/src/pages/architecture.astro']
---

# Surface: Cubebot landing page (site/src/pages/index.astro)

Mode: Persuade. Scope: the public homepage of the standalone Astro site in `site/`. Related: future `/architecture` page inherits this world.

Audience: self-hosting developers first, engineering teams second. Job: understand what Cubebot is in seconds, believe the engineering is serious, see that it is under active development, and star/watch github.com/vicotrbb/cubebot. Proof on hand: manifesto, architecture, 82 ADRs, dated milestone ledger. No screenshots, customers, benchmarks, or pricing; illustrative UI is labeled as illustration.

Constraints: static Astro, minimum dependencies, no 3D library (CSS 3D only), every script function at cyclomatic complexity <= 15, honest implemented / in progress / planned status, light cozy orange world, a 3D cube with eyes as logo.

## Direction contract

THESIS: Agents are blocks you stack into a cluster. The page is a warm desk where one painted orange block with eyes lives, and the product is explained by stacking more blocks. It refuses the dark glowing hero, the framed product screenshot, and the three-column feature-card grid.

OWN-WORLD: Ground is a pale warm desk (#F6EFE4) with a birch-wood band (#E9CFA5) and soft contact shadows; the only saturated color is painted orange (#F26A1B) on blocks, buttons, and underlines; ink is warm near-black (#2B2420); one cool relief is a muted sage (#9BB7A4) for sleeping/secondary states. Type is Rubik, block-stamped rounded letterforms, Black for display, Regular for body. Components are blocks: tiles with 3D painted faces, rounded 10px corners, stamped letters, and a 2px darker bottom edge; sections sit on the desk, not in cards.

STORY: "Cubebot gives each AI agent a lasting home on Kubernetes you control." The visitor sees the cube, reads one sentence, scrolls to watch blocks stack into pod / volume / journal / memory, learns honestly what runs today and what is next, and stars the repo.

FIRST VIEWPORT: Header: block logo + wordmark left, four links, orange GitHub block button right. Below, a two-column hero on the desk. Left column (55%): small stamped tag "Under development · entry 26 Oct 2026", headline "A home for your agents." in Rubik Black at ~80px, subline two lines, then primary "Star on GitHub" orange block button and secondary "Read the architecture" pencil link. Right column (45%): a large CSS-3D orange cube with two eyes resting on the wood band with a contact shadow, slowly idling, turning toward the pointer, blinking every few seconds; two smaller blocks behind it hint at the stack.

FORM: Wooden Blocks on the Desk, candidate 1 of my grounded list (the IMPECCABLE'S PICK card), chosen over the assigned The Open Notebook. Seed key abceb911. Signature interaction: scroll stacks blocks into the agent's anatomy and the cluster tower. Motion grammar: blocks land with one soft settle (translateY + slight overshoot, 420ms), eyes blink, cube turns toward the pointer; reduced motion gives static poses.

FINISH: unreviewed and undocumented is unfinished; this build ends with the finish review, the verdict, DESIGN.md, and every shipping raster carrying its provenance.
