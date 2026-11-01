---
name: Cubebot
description: Painted wooden blocks on a warm desk. A 3D orange cube with eyes is the agent; the site explains the product by stacking more blocks, and the console is the same desk cleared for work.
colors:
  desk: '#f6efe4'
  desk-deep: '#efe3d1'
  wood: '#e9cfa5'
  wood-deep: '#d4b380'
  wood-grain: '#c9a26b'
  orange: '#f26a1b'
  orange-top: '#ff8a3d'
  orange-deep: '#c9530f'
  orange-base: '#a6420a'
  ink: '#2b2420'
  ink-soft: '#5e5249'
  sage: '#9bb7a4'
  sage-top: '#b4cbbb'
  sage-deep: '#6f8f7a'
  paper: '#fffdf9'
  paper-deep: '#e8e1d4'
  # Console (apps/web): the desk world in its Operate register. Light values
  # from src/styles/tokens.css :root; -dark variants from its dark theme block.
  console-ground: '#f6efe4'
  console-sidebar: '#efe3d1'
  console-surface: '#fffdf9'
  console-raised: '#ffffff'
  console-inset: '#f3ebdf'
  console-ink-faint: '#776a5f'
  console-line: 'rgb(43 36 32 / 0.12)'
  console-line-strong: 'rgb(43 36 32 / 0.22)'
  console-orange-wash: '#fde9dc'
  console-danger: '#c23b22'
  console-danger-wash: '#fbe4df'
  console-success: '#6f8f7a'
  console-sleep: '#9bb7a4'
  console-code-bg: '#f3ebdf'
  console-ground-dark: '#201a16'
  console-sidebar-dark: '#120e0b'
  console-surface-dark: '#26201b'
  console-raised-dark: '#2b2420'
  console-inset-dark: '#1f1a16'
  console-ink-dark: '#f3ece2'
  console-ink-soft-dark: '#cfc3b6'
  console-ink-faint-dark: '#a3968b'
  console-line-dark: 'rgb(255 240 225 / 0.1)'
  console-line-strong-dark: 'rgb(255 240 225 / 0.2)'
  console-orange-wash-dark: '#3a2416'
  console-danger-dark: '#e86a52'
  console-danger-wash-dark: '#3b1f19'
  console-success-dark: '#8fb09c'
  console-code-bg-dark: '#1a1512'
  console-sleep-dark: '#7f9a88'
  # Avatar paints beyond the site's four (each a top / face / deep / base block triad)
  sage-base: '#587865'
  paper-top: '#ffffff'
  paper-base: '#d2c9b8'
  wood-top: '#f2dfbf'
  wood-base: '#b8955d'
  sky: '#8db9d6'
  sky-top: '#a9cde3'
  sky-deep: '#5f90b0'
  sky-base: '#4a7591'
  rose: '#e5a0b4'
  rose-top: '#f0bccb'
  rose-deep: '#c2708a'
  rose-base: '#a0586f'
  lemon: '#f0cf5a'
  lemon-top: '#f7df86'
  lemon-deep: '#c9a62f'
  lemon-base: '#a4861f'
  plum: '#9a86b8'
  plum-top: '#b6a6cd'
  plum-deep: '#6f5d8f'
  plum-base: '#574872'
typography:
  display:
    fontFamily: 'Rubik, system-ui, sans-serif'
    fontSize: 'clamp(2.8rem, 7.2vw, 5.6rem)'
    fontWeight: 900
    lineHeight: 0.98
    letterSpacing: '-0.035em'
  headline:
    fontFamily: 'Rubik, system-ui, sans-serif'
    fontSize: 'clamp(2rem, 4.2vw, 3.1rem)'
    fontWeight: 800
    lineHeight: 1.05
    letterSpacing: '-0.02em'
  title:
    fontFamily: 'Rubik, system-ui, sans-serif'
    fontSize: '1.2rem'
    fontWeight: 800
    lineHeight: 1.05
    letterSpacing: '-0.015em'
  lede:
    fontFamily: 'Rubik, system-ui, sans-serif'
    fontSize: 'clamp(1.1rem, 1.6vw, 1.3rem)'
    fontWeight: 400
    lineHeight: 1.5
    letterSpacing: 'normal'
  body:
    fontFamily: 'Rubik, system-ui, sans-serif'
    fontSize: '1.0625rem'
    fontWeight: 400
    lineHeight: 1.55
    letterSpacing: 'normal'
  label:
    fontFamily: 'Rubik, system-ui, sans-serif'
    fontSize: '0.9rem'
    fontWeight: 600
    lineHeight: 1.45
    letterSpacing: 'normal'
  stamp:
    fontFamily: 'Rubik, system-ui, sans-serif'
    fontSize: '0.92rem'
    fontWeight: 700
    lineHeight: 1.25
    letterSpacing: '-0.005em'
  # Console: fixed rem scale (--t-xs … --t-3xl in tokens.css), no fluid type
  console-h1:
    fontFamily: 'Rubik, system-ui, sans-serif'
    fontSize: '1.5rem'
    fontWeight: 800
    lineHeight: 1.15
    letterSpacing: '-0.015em'
  console-h2:
    fontFamily: 'Rubik, system-ui, sans-serif'
    fontSize: '1.25rem'
    fontWeight: 800
    lineHeight: 1.15
    letterSpacing: '-0.015em'
  console-h3:
    fontFamily: 'Rubik, system-ui, sans-serif'
    fontSize: '1.0625rem'
    fontWeight: 800
    lineHeight: 1.15
    letterSpacing: '-0.015em'
  console-body:
    fontFamily: 'Rubik, system-ui, sans-serif'
    fontSize: '0.9375rem'
    fontWeight: 400
    lineHeight: 1.5
    letterSpacing: 'normal'
  console-label:
    fontFamily: 'Rubik, system-ui, sans-serif'
    fontSize: '0.8125rem'
    fontWeight: 600
    lineHeight: 1.2
    letterSpacing: 'normal'
  console-micro:
    fontFamily: 'Rubik, system-ui, sans-serif'
    fontSize: '0.75rem'
    fontWeight: 700
    lineHeight: 1.4
    letterSpacing: 'normal'
  console-mono:
    fontFamily: 'ui-monospace, SF Mono, Menlo, Consolas, monospace'
    fontSize: '0.875em'
    fontWeight: 400
    lineHeight: 1.5
    letterSpacing: 'normal'
rounded:
  focus: '6px'
  chip: '8px'
  block: '10px'
  face: '0'
  console-sm: '6px'
  console-md: '8px'
  console-lg: '10px'
  console-xl: '14px'
  console-pill: '999px'
spacing:
  chip-gap: '10px'
  item-gap: '30px'
  para-gap: '20px'
  stack-gap: '36px'
  gutter: 'clamp(16px, 5vw, 64px)'
  section: 'clamp(56px, 9vw, 120px)'
  measure: '62ch'
  container: '1180px'
  console-sidebar-w: '260px'
  console-panel-w: '380px'
  console-header-h: '56px'
  console-bottombar-h: '60px'
  console-page-pad: '20px'
  console-control-gap: '10px'
  console-form-gap: '16px'
  console-measure: '78ch'
components:
  button-block:
    backgroundColor: '{colors.orange}'
    textColor: '{colors.ink}'
    typography: '{typography.body}'
    rounded: '{rounded.block}'
    padding: '14px 22px 16px'
  button-block-paper:
    backgroundColor: '{colors.paper}'
    textColor: '{colors.ink}'
    typography: '{typography.body}'
    rounded: '{rounded.block}'
    padding: '14px 22px 16px'
  button-block-compact:
    backgroundColor: '{colors.orange}'
    textColor: '{colors.ink}'
    rounded: '{rounded.block}'
    padding: '10px 16px 12px'
  chip-orange:
    backgroundColor: '{colors.orange}'
    textColor: '{colors.ink}'
    typography: '{typography.stamp}'
    rounded: '{rounded.chip}'
    padding: '6px 12px 8px'
  chip-sage:
    backgroundColor: '{colors.sage}'
    textColor: '{colors.ink}'
    typography: '{typography.stamp}'
    rounded: '{rounded.chip}'
    padding: '6px 12px 8px'
  chip-paper:
    backgroundColor: '{colors.paper}'
    textColor: '{colors.ink}'
    typography: '{typography.stamp}'
    rounded: '{rounded.chip}'
    padding: '6px 12px 8px'
  chip-wood:
    backgroundColor: '{colors.wood}'
    textColor: '{colors.ink}'
    typography: '{typography.stamp}'
    rounded: '{rounded.chip}'
    padding: '6px 12px 8px'
  chip-half:
    backgroundColor: '{colors.wood}'
    textColor: '{colors.ink}'
    typography: '{typography.stamp}'
    rounded: '{rounded.chip}'
    padding: '6px 12px 8px 20px'
  step-number:
    backgroundColor: '{colors.orange}'
    textColor: '{colors.ink}'
    rounded: '{rounded.block}'
    size: '44px'
  stamp:
    textColor: '{colors.orange-base}'
    rounded: '{rounded.focus}'
    padding: '7px 14px'
  nav-link:
    textColor: '{colors.ink}'
    typography: '{typography.label}'
    padding: '6px 2px'
  # Console controls (src/styles/controls.css, shell.css, chat.css, desktop.css, agent.css)
  console-button:
    backgroundColor: '{colors.console-raised}'
    textColor: '{colors.ink}'
    typography: '{typography.console-label}'
    rounded: '{rounded.console-md}'
    padding: '7px 14px 9px'
    height: '36px'
  console-button-primary:
    backgroundColor: '{colors.orange}'
    textColor: '{colors.ink}'
    typography: '{typography.console-label}'
    rounded: '{rounded.console-md}'
    padding: '7px 14px 9px'
    height: '36px'
  console-button-danger:
    backgroundColor: '{colors.console-danger-wash}'
    textColor: '{colors.console-danger}'
    typography: '{typography.console-label}'
    rounded: '{rounded.console-md}'
    padding: '7px 14px 9px'
    height: '36px'
  console-button-ghost:
    textColor: '{colors.ink}'
    typography: '{typography.console-label}'
    rounded: '{rounded.console-md}'
    padding: '7px 14px 9px'
    height: '36px'
  console-button-sm:
    backgroundColor: '{colors.console-raised}'
    textColor: '{colors.ink}'
    typography: '{typography.console-micro}'
    rounded: '{rounded.console-md}'
    padding: '4px 10px 6px'
    height: '30px'
  console-button-icon:
    backgroundColor: '{colors.console-raised}'
    textColor: '{colors.ink}'
    rounded: '{rounded.console-md}'
    padding: '0'
    size: '36px'
  console-input:
    backgroundColor: '{colors.console-raised}'
    textColor: '{colors.ink}'
    typography: '{typography.console-body}'
    rounded: '{rounded.console-md}'
    padding: '8px 12px'
    height: '38px'
  console-chip:
    backgroundColor: '{colors.console-inset}'
    textColor: '{colors.ink-soft}'
    typography: '{typography.console-micro}'
    rounded: '{rounded.console-sm}'
    padding: '2px 9px 3px'
  console-chip-paint:
    backgroundColor: '{colors.orange}'
    textColor: '{colors.ink}'
    typography: '{typography.console-micro}'
    rounded: '{rounded.console-sm}'
    padding: '2px 9px 3px'
  console-chip-sage:
    backgroundColor: '{colors.sage}'
    textColor: '{colors.ink}'
    typography: '{typography.console-micro}'
    rounded: '{rounded.console-sm}'
    padding: '2px 9px 3px'
  console-chip-wood:
    backgroundColor: '{colors.wood}'
    textColor: '{colors.ink}'
    typography: '{typography.console-micro}'
    rounded: '{rounded.console-sm}'
    padding: '2px 9px 3px'
  console-chip-danger:
    backgroundColor: '{colors.console-danger-wash}'
    textColor: '{colors.console-danger}'
    typography: '{typography.console-micro}'
    rounded: '{rounded.console-sm}'
    padding: '2px 9px 3px'
  console-sidebar-row:
    textColor: '{colors.ink}'
    typography: '{typography.console-body}'
    rounded: '{rounded.console-md}'
    padding: '4px 8px'
    height: '34px'
  console-sidebar-row-current:
    backgroundColor: '{colors.orange}'
    textColor: '{colors.ink}'
    typography: '{typography.console-body}'
    rounded: '{rounded.console-md}'
    padding: '4px 8px'
    height: '34px'
  console-badge:
    backgroundColor: '{colors.orange}'
    textColor: '{colors.ink}'
    typography: '{typography.console-micro}'
    rounded: '10px'
    padding: '0 6px'
    height: '20px'
  console-tabs:
    backgroundColor: '{colors.console-inset}'
    rounded: '{rounded.console-md}'
    padding: '3px'
  console-tab:
    textColor: '{colors.ink-soft}'
    typography: '{typography.console-label}'
    rounded: '{rounded.console-sm}'
    padding: '5px 12px'
  console-tab-selected:
    backgroundColor: '{colors.console-raised}'
    textColor: '{colors.ink}'
    typography: '{typography.console-label}'
    rounded: '{rounded.console-sm}'
    padding: '5px 12px'
  console-panel:
    backgroundColor: '{colors.console-ground}'
    padding: '16px'
    width: '{spacing.console-panel-w}'
  console-work-card:
    backgroundColor: '{colors.console-inset}'
    textColor: '{colors.ink}'
    typography: '{typography.console-label}'
    rounded: '{rounded.console-md}'
    padding: '6px 10px'
  console-thread-line:
    backgroundColor: '{colors.console-raised}'
    textColor: '{colors.ink}'
    typography: '{typography.console-label}'
    rounded: '{rounded.console-md}'
    padding: '4px 10px 4px 6px'
  console-composer:
    backgroundColor: '{colors.console-raised}'
    textColor: '{colors.ink}'
    typography: '{typography.console-body}'
    rounded: '{rounded.console-lg}'
    padding: '12px 14px 6px'
  console-pip:
    backgroundColor: '{colors.console-raised}'
    rounded: '{rounded.console-lg}'
    width: '320px'
  console-dialog:
    backgroundColor: '{colors.console-surface}'
    textColor: '{colors.ink}'
    rounded: '{rounded.console-xl}'
    padding: '20px 22px 22px'
    width: 'min(520px, calc(100vw - 32px))'
  console-menu:
    backgroundColor: '{colors.console-raised}'
    rounded: '{rounded.console-lg}'
    padding: '6px'
    width: '200px'
  console-bottombar:
    backgroundColor: '{colors.console-sidebar}'
    textColor: '{colors.console-ink-faint}'
    typography: '{typography.console-micro}'
    height: '{spacing.console-bottombar-h}'
  console-notice:
    backgroundColor: '{colors.console-inset}'
    typography: '{typography.console-label}'
    rounded: '{rounded.console-md}'
    padding: '10px 12px'
---

# Design System: Cubebot

## Overview

**Creative North Star: "Wooden Blocks on the Desk"**

Cubebot is a warm desk with painted wooden blocks on it. The ground is pale paper-warm desk, a birch shelf runs through the scenes, and the only saturated color is the orange paint on the blocks. One painted block has eyes: that block is the agent, and every other block is a part of its body or a neighbor in its cluster. The page explains the product by stacking blocks rather than by framing screenshots or laying out feature cards. Sections sit directly on the desk; depth comes from a block's own darker bottom edge and a soft contact shadow underneath it, never from floating panels.

The type is one face, Rubik, used heavy and tight for headings (stamped letterforms) and regular for reading. Motion is physical and small: a block lands once with a single ease-out settle, the hero cube bobs, blinks, and turns toward the pointer. Nothing glows, nothing slides in from the side, nothing is dark.

The world carries one content rule from the product: honesty about state. Implemented, in progress, and planned are always visually distinct (painted, half-painted, raw wood), so a reader can tell what runs today without reading the copy.

**Key Characteristics:**

- Light warm desk ground; one saturated paint color (orange); sage as the single cool relief
- Blocks, not cards: a painted face with a darker bottom edge and a blurred contact shadow
- Rubik throughout; Black/ExtraBold headings with negative tracking, Regular body
- Material tones double as status: orange = painted/active, sage = sleeping, paper = infrastructure, wood = raw/planned
- Motion is one settle, one blink, one bob; reduced motion freezes every pose

## Colors

A pale warm desk with birch wood, one orange paint, a cool sage relief, and warm near-black ink. Every block tone ships as a triad (top, face, side) plus a base.

### Primary

- **Painted Orange** (`orange`): the only saturated color. Block faces, block buttons, chips for implemented work, link underlines, selection, focus ring, favicon. Lit top face is `orange-top`; side faces and the bottom edge of buttons and chips are `orange-deep`; the block's underside and the stamp outline are `orange-base`.

### Secondary

- **Sage** (`sage`): the single cool note. Sleeping agents (closed eyes), the memory block, secondary cluster residents. Top `sage-top`, side and bottom edge `sage-deep`.

### Tertiary

- **Paper** (`paper`): unpainted white blocks for infrastructure and system pieces (journal, console, PostgreSQL) and the secondary block button. Edge `paper-deep`.
- **Birch Wood** (`wood`): raw, unpainted blocks for planned work, the shelf and tray surfaces, and the half-painted chip body. Shade `wood-deep`; the shelf's front lip and grain lines are `wood-grain`.

### Neutral

- **Desk** (`desk`): page background. Alternating sections drop to **Desk Deep** (`desk-deep`); there is no other section treatment.
- **Ink** (`ink`): all headings, button and chip text, cube eyes, cluster edges. **Ink Soft** (`ink-soft`): body copy under headings, captions, dates for planned milestones.
- **Shadow** (`43 36 32`, used as `rgb(var(--shadow) / a)`): every shadow, divider, and contact blur is this warm ink at low alpha (0.10 dividers, 0.26–0.28 contact, 0.35–0.38 button lift).

### Named Rules

**The One Paint Rule.** Orange is paint, not decoration. It appears on blocks, block buttons, chips, underlines, and the focus ring, and nowhere else: no orange text headings, no orange backgrounds, no gradient text.

**The Tone Is Status Rule.** A block's material tells its state. Orange is painted/implemented/active, half-painted (orange end on raw wood) is in progress, raw wood is planned, paper is infrastructure, sage is sleeping. Never use these tones decoratively against their meaning.

**The Warm Shadow Rule.** No black or cool shadows. Every shadow and divider is `rgb(43 36 32 / a)`.

## Typography

**Display Font:** Rubik (variable, 300–900, self-hosted `/fonts/rubik-latin.woff2`; fallback system-ui, sans-serif)
**Body Font:** Rubik (same file)

**Character:** One rounded, block-stamped face does everything. Weight and tracking carry the hierarchy: headings go heavy and tight like letters pressed into a block; body stays regular and open.

### Hierarchy

- **Display** (900, `clamp(2.8rem, 7.2vw, 5.6rem)`, 0.98, −0.035em): the hero headline only. Balanced wrapping.
- **Headline** (800, `clamp(2rem, 4.2vw, 3.1rem)`, 1.05, −0.02em): section headings, max 18ch wide.
- **Title** (800, 1.2–1.25rem, −0.015em): anatomy terms, step titles, status row heads, wordmark (1.25rem).
- **Lede** (400, `clamp(1.1rem, 1.6vw, 1.3rem)`, 1.5, Ink Soft): hero subline and section intros (intros are 1.1rem), max 62ch.
- **Body** (400, 1.0625rem, 1.55): running copy; descriptions under titles are Ink Soft.
- **Label** (600, 0.9rem): nav links, footer links, cluster captions, milestone copy (0.85–0.9rem, dates 700 in Orange Base).
- **Stamp** (700, 0.92rem, 1.25, −0.005em): chip text and the rotated outline stamp (0.95rem).
- **Face label** (800, 20% of block size, lowercase, −0.02em): the word stamped on a block's front face.

### Named Rules

**The Heavy Heading Rule.** Headings are 800 or 900 with negative tracking and line-height ≤ 1.05. There is no light or thin display setting.

**The No Kicker Rule.** Nothing sits above a heading. Status and date stamps live below the actions, not as an eyebrow.

## Layout

Content sits in a centered container `min(1180px, 100% − 2 × gutter)` with gutter `clamp(16px, 5vw, 64px)`. Sections stack with `clamp(56px, 9vw, 120px)` vertical padding and alternate between Desk and Desk Deep backgrounds; no section borders except a 2px low-alpha divider on status rows and the footer. Reading copy is capped at 62ch.

Two-column scenes use asymmetric grids: hero 11fr/9fr (text left, cube right), anatomy 1fr/160px/1fr with a sticky intro and a block tower in the middle, how-it-works and closing 1fr/1fr or 2fr/3fr. Blocks rest on a wood shelf or tray that spans the scene; text hangs beneath the shelf on the roadmap.

Rhythm: 20px between a heading and its intro, 30px between list items, 36px above an action row, 10px between chips, 28px between status rows.

Breakpoints: at 960px every grid collapses to one column, the nav hides (the GitHub block button stays), the cube scene moves above the headline, the tower lies down as a row, the roadmap becomes a vertical list with 88px block column and the roadmap shelf hides. At 560px blocks shrink (tower 52px, cluster agents 56px) and cluster gaps tighten.

## Elevation & Depth

Depth is material, not elevation. A block reads as solid because its front face carries a darker bottom edge (4px on buttons, 3px on chips and step numbers) and a soft, blurred contact shadow sits on the desk under it. The 3D cube gets a lit top face, darker sides, and a darkest underside. There is no layered surface system and no card shadow; sections are flat regions of the desk.

### Shadow Vocabulary

- **Button lift** (`box-shadow: 0 8px 18px -6px rgb(43 36 32 / 0.35)`): resting block button. Hover `0 12px 22px -6px / 0.38`, pressed `0 4px 10px -4px / 0.35`.
- **Shelf drop** (`0 10px 24px -10px rgb(43 36 32 / 0.35), inset 0 2px 0 rgb(255 255 255 / 0.35)`): the wood shelf and tray, with a highlight on the top edge.
- **Contact shadow** (`radial-gradient(closest-side, rgb(43 36 32 / 0.28), transparent)` ellipse 1.35× wide and 0.28× tall of the block, offset 0.22× below): under any block resting on the desk.
- **Face light** (no shadow): cube faces meet at crisp edges and carry a lighting gradient instead of a highlight line. Front: `linear-gradient(165deg, color-mix(in oklab, face, white 7%), face 45%, color-mix(in oklab, face, black 7%))`; sides darken 12% toward the bottom; the top lightens 14% toward the top-left.

### Named Rules

**The Blurred Contact Rule.** Every external shadow is blurred and warm. Hard, zero-blur offset shadows do not exist here; the one zero-blur inset (the half-painted chip's orange end) is paint, not shadow.

**The Bottom Edge Rule.** A pressable or status-bearing block always shows its darker bottom edge in the same tone family (`*-deep`). Pressing shrinks the edge from 4px to 2px and moves the block down 2px.

## Shapes

Softly rounded blocks. Block radius is 10px (buttons, step numbers); chips are 8px; shelves 8px, trays and the stamp 6px; the focus ring 6px; cube faces are square so the six faces meet without seams. No pills, no sharp corners, no circles except the eyes and contact ellipse. Borders are rare: a 2px low-alpha ink divider between status rows and above the footer, and the 2px Orange Base outline of the rotated (−1.4°) speckled stamp. Cluster edges are 3px ink bars at 50% opacity, dashed for external connections.

## Components

### Buttons

Block buttons: a painted block you press.

- **Shape:** rounded block (10px), darker 4px bottom edge
- **Primary (`button-block`):** Orange face, Orange Deep edge, Ink text at 700 / −0.01em, padding 14px 22px 16px, 10px gap to a 20px inline SVG icon, button-lift shadow. Compact header variant pads 10px 16px 12px.
- **Hover / Focus:** lifts 2px (`translateY(-2px)`) with the larger shadow over 0.18s on the settle ease; focus uses the global 3px orange ring offset 3px.
- **Active:** drops 2px, edge shrinks to 2px, shadow tightens.
- **Secondary (`button-block-paper`):** same block in Paper with a Paper Deep edge.
- **Text link (`.link`):** 600 weight, inherits ink, 18px arrow icon that nudges 3px right on hover. Links everywhere underline in orange, 2px thick, offset 0.18em; hover turns the underline ink.

### Chips

Small blocks seen from the front.

- **Style:** 8px radius, 3px bottom edge, Ink text 700 at 0.92rem, padding 6px 12px 8px, 10px gap when wrapped.
- **Tones:** orange (implemented), sage (sleeping/secondary), paper (infrastructure), wood (planned).
- **Half-painted (`chip-half`):** wood body with a 10px orange end (`box-shadow: inset 10px 0 0 var(--orange)`) and 20px left padding, for work in progress. The matching legend swatch is an 18px block split 50/50 orange/wood.
- **Pick-up:** in a chip list, hover lifts the chip 3px (0.2s settle) and adds a blurred lift shadow (`0 10px 16px -8px` warm ink at 0.4). Chip lists land as a list: each chip drops in with the 90ms stagger.

### Step number

A 44px orange block (10px radius, 3px Orange Deep edge) holding an 800-weight numeral; used for ordered steps.

### Stamp

A rubber-stamp tag: 2px Orange Base outline, Orange Base text at 700, 6px radius, rotated −1.4°, speckled through a radial mask, 18px icon. Sits below the hero actions.

### Navigation

Header row with the 36px logo and 800-weight wordmark left, 600-weight links with a transparent 2px bottom border that turns orange on hover and on the section currently mid-viewport (`is-current`, set by an IntersectionObserver with a −35% / −55% band), and a compact orange GitHub block button right. The header sticks to the top on the Desk color and gains a blurred drop shadow (`0 12px 24px -16px` warm ink at 0.4) over the first 120px of scroll through a scroll-driven animation; sections carry a 72px `scroll-margin-top`. Links hide under 960px; the block button stays. Footer repeats logo, one-line description in Ink Soft, and plain 600-weight links.

### Slab (shelf and tray)

A short wooden shelf blocks rest on, built like the desk: a 20px top surface (`#f3deba` to `#e6c99c`, the desk's two grain stripe frequencies at 93°, white top highlight, 8px top radius) over a 14px front edge (Wood Deep to Wood Grain, 6px bottom radius, `0 12px 24px -12px` warm drop at 0.45). The same 34px slab is the tower base under the anatomy stack, the floor of the paint shop, the cluster's provider tray, and the roadmap shelf behind the milestone blocks (it extends 16px past the grid on each side).

### Desk (hero)

The hero stands its blocks on a real desk seen slightly from above: a 34px top surface (`#f3deba` to `#e6c99c` with two grain stripe frequencies at 93°, white top highlight), a 22px front edge (Wood Deep to Wood Grain, shelf-drop shadow), and two 22px legs (`#b8955d` to `#a37f49`, rounded feet) set 6% in from each end. Blocks sit on the top surface with contact shadows; the big cube's shadow breathes with its bob. A potted plant keeps the cube company: an Orange trapezoid pot with an Orange Top rim and three sage leaves (`sage-top` to `sage-deep`) that sway 5° over 5.2s.

The closing section repeats the desk with the cube (150px, pointer-following, bobbing), the sleeping sage block, the raw block, and a mug in front: a Paper body (`#fffdf9` to `#ece4d5`, 5px top and 14px bottom radius) with a 7px Orange band, a Paper Deep elliptical rim, a 7px handle ring on the right, a contact shadow, and three blurred white steam wisps that rise 28px over 2.8s, 0.9s apart.

### Scenes

Blocks explain the product by being arranged, never by icons or cards.

- **Tower (anatomy):** six 92px labeled blocks stacked pod → skills on a slab. They drop in from 140px above with a small overshoot (`cubic-bezier(0.34, 1.35, 0.64, 1)`, 0.6s, 120ms apart, pod first) and, where scroll-driven animations exist, each block turns 70° to face the reader as it enters the viewport. Hovering a part in the list turns its block to 58° and slides it 18px right and 6px up; the hovered row gets a faint Paper wash (55%). On phones the tower is two rows of three on the slab.
- **Cluster (how it works):** a wooden rack, 620 × 600 scene units, scaled with its container (block sizes in `cqw`). Two posts (`#b8955d` to `#a37f49`) hold three slab shelves: console on top, orchestrator and PostgreSQL in the middle, a sleeping pod and two awake pods at the bottom; the provider block stands on its own small slab outside the rack. Cables are 3px ink-at-45% SVG paths drawn from block to block behind the shelves, dashed for the external provider link, and six-unit Orange dots ride them in sequence on a 4.4s cycle (console → orchestrator → database → pods → providers), parking hidden inside the destination block. Labels are paper tags (Paper face, 2px Paper Deep edge, 6px radius) hanging under each shelf with the cables passing behind. Under 560px the rack stretches to 620 × 820 and tags wrap to 9ch.
- **Paint shop (status):** the orange cube (turned −34°, bobbing) watches a 128px raw block being painted on a slab. The painted block (`paint` prop) carries four horizontal brush strokes up 57% of its front face, laid bottom-up with rounded, slightly ragged right ends and alternating 3–6% lighter mixes so they read as separate passes; the right face is painted to the top of the last finished stroke (43%) in Orange Deep. The paint shop uses the live form (`paint="live"`): all four side faces carry four stroke rects, and a paintbrush SVG (wood handle with a highlight, warm-grey ferrule, Orange Deep bristles with wet Orange tips, tilted −22°, drop-shadowed) floats in a `cube__hand` plane 2px in front of the resting front face, so it does not turn with the block. Each cycle is 22.4s: four 5.6s phases. In a phase the brush lays four left-to-right passes climbing the face (0.9s each, 0.3s back to the left), each stroke's width growing 0 → 96% in step with it; then over 0.8s the block turns a quarter backwards (`quarter` keyframes, ease-in-out) so the fresh face comes forward and the painted one shows as the left side, while the brush dips toward the can at the lower left and returns. Faces come forward in the order front, right, back, left, and a face's strokes reset only while it is hidden at the back, so the loop never jumps. Under reduced motion the block is still: three strokes on the front, the brush resting at the fourth. A paint can stands on the floor beside it (Paper body, Orange label band, Orange Deep open top with an Orange rim, a drip, contact shadow). This is the half-painted state in three dimensions: use it wherever a block is being qualified, including the roadmap's current milestone.
- **Roadmap:** nine 58px blocks on one slab: painted with eyes for shipped milestones, the paint-in-progress block for the current one, raw wood for what is planned. Hovering a milestone lifts its block 8px and turns it to 44°.

### Cube (signature)

A CSS-3D block built from six faces on a body rotated `rotateX(-18deg) rotateY(28deg)` at rest, perspective 9× its size. Tones: orange (default), sage, paper, wood, each with lit top, darker sides, darkest base. The front face may carry eyes or a lowercase stamped label. Eyes are two ink blobs with a Paper glint at the upper left (open = active agent) or two ink smile strokes (closed = sleeping); no eyes = an inert object (volume, journal, console). Open eyes come with two faint white blush ovals (22% white, 16% × 9% of the block) low on the cheeks. Open eyes blink every 5.4s (scaleY to 0.08, second eye 60ms behind) and, on blocks that do not follow the pointer, glance left then right once every 9s (4.5% of the block each way, offset by `--blink-delay` so no two blocks look at once). Sleeping blocks breathe three Ink z's (16–24% of the block, 900 weight) that drift up and right over 3.9s, 1.3s apart. A `contact` shadow may be added when the block rests on the desk. The hero cube bobs 9px over 4.6s, turns toward the pointer (±16° yaw, ±10° pitch, eased by 12% per frame), and its eyes glance the same way (up to 7px × 5px via `--look-x` / `--look-y`). The logo and favicon are the same cube flattened: Orange Top cap, Orange left face, Orange Deep right face, two Ink eyes.

### Motion grammar

One easing: `--settle: cubic-bezier(0.16, 1, 0.3, 1)`. Blocks land from 34px above over 0.55s on the settle ease with opacity in over 0.3s, staggered 90ms per item, once, when their group scrolls into view (12% bottom margin); the 3D tower only drops, never fades, and is the one place with overshoot. Buttons and links transition 0.18–0.2s. `--rx` and `--ry` are registered angles, so a block turns smoothly (0.45s settle) when its pose changes on hover. Idle life is slow and offset: bobs at 4.6s, glances at 9s, z's at 3.9s, steam at 2.8s, message dots at 3.6s, so nothing pulses in sync. Under `prefers-reduced-motion`, reveals are static, eyes do not blink or glance, z's, steam, message dots, and the brush stop, the cubes do not bob, turn on scroll, or follow the pointer, and scrolling is instant.

### Browser surface

Selection is orange with paper text; focus is a 3px orange outline offset 3px with 6px radius; `theme-color` is orange.

## Do's and Don'ts

### Do:

- **Do** build every interactive or status element as a block: tone face, `*-deep` bottom edge, 8–10px radius, ink text.
- **Do** show state through material: orange painted, half-painted, raw wood, paper, sage. Implemented, in progress, and planned must be distinguishable without reading.
- **Do** keep orange the only saturated color and sage the only cool one.
- **Do** rest blocks on wood shelves or trays with a blurred contact shadow when they sit on the desk.
- **Do** use one settle easing and land things once; give reduced-motion users a static pose.
- **Do** set headings in Rubik 800–900 with negative tracking and cap reading copy at 62ch.

### Don't:

- **Don't** use cards or bordered panels as page structure; sections are flat regions of the desk, alternating Desk and Desk Deep.
- **Don't** put a kicker or eyebrow above a heading.
- **Don't** use gradient text, orange headings, or a dark hero.
- **Don't** use zero-blur offset shadows or black/cool shadows; shadows are warm ink, blurred.
- **Don't** add a second typeface, light display weights, or uppercase tracking.
- **Don't** give eyes to a block that is not an agent, or open eyes to one that is sleeping.

<!-- console:start — the console (apps/web) register of the same world. Tokens prefixed `console-` in the frontmatter are normative here; everything above this line is the landing site and stays as written. -->

## Console

The same desk world in its Operate register (`apps/web`, the Slack-grammar console: sidebar, one conversation, contextual right panel, threads for delegated work). Nothing here replaces the site rules; where the console narrows them (no scenery, fixed type scale, 3px edges) it says so.

### Overview

**Creative North Star: "The Desk, Cleared for Work"**

The console is the desk with the shelves, mugs and plants put away. What remains is the paper the conversation is written on, a deeper warm sidebar to the left, and the painted blocks: one cube with eyes per agent, carrying its mood. Orange is still the one paint and now does the console's pointing: the selected row, the primary action, the unread mark, the focus ring. Depth is tonal (sidebar, ground, surface, raised) with the same warm blurred shadow for the few things that float. Rubik runs at a fixed seven-step rem scale with no fluid sizes; headings keep the heavy stamp (800, −0.015em) but stop at 1.5rem.

A dark theme, "desk at night", replaces the paper tones with warm near-black inks and keeps orange, the paints and the cube unchanged; sage dims one step for sleeping.

**Key Characteristics:**

- Four warm planes, flat: sidebar (deepest) → ground → surface (the conversation) → raised (controls, fields, cards); the shadow colour is warm ink in light and black at night
- One accent: orange marks selection, primary actions, unread badges and focus; nothing else is saturated except block paints and status dots
- Block buttons keep their painted edge, shrunk to 3px and drawn as an inset; the sidebar's selected row is flat orange with no edge
- Fixed rem type scale 0.75 → 1.5rem; headings 800 / −0.015em / 1.15, never above 1.5rem
- The cube is the only illustration; its mood (idle, working, dreaming, sleeping, desktop, waiting, retired) comes from activity and is never stored
- Lucide icons at one stroke, 14–22px, in Ink Faint

### Colors

The site palette plus a plane system and a small set of status tints. Light values are the `console-*` tokens; dark values are the `console-*-dark` tokens.

#### Primary

- **Painted Orange** (`orange`): selected sidebar row and palette hit (flat fill, `on-paint` ink text), primary button face with an `orange-deep` edge, unread badge, the `::selection` background, the 3px focus outline, the working status dot and its pulse, the composer and field focus ring (`rgb(242 106 27 / 0.25)`, composer 0.2), the focused composer and field border, the thread line's hover border, the checked avatar-builder option border, the desktop-control outline and the agent's desktop pointer tag. **Orange Deep** is the phone bottom bar's current item, the running-state text, spinner and warning marks and the login-handoff icon (`--warn` is declared as the same value and unused). **Orange Wash** (`console-orange-wash`): selected table row, the paint notice, the login-handoff banner (with a `rgb(242 106 27 / 0.35)` border) and the sidebar `row__flag` (text Orange Base).

#### Secondary

- **Sage** (`sage`, `console-sleep`): the sleeping status dot and the sleeping cube paint. **Sage Deep** doubles as `console-success` (idle dot, "ok" work marks, your own desktop pointer, thread-line faces). In the dark theme `console-sleep-dark` and `console-success-dark` lift one step so they stay visible on ink.

#### Tertiary

- **Danger** (`console-danger`, a warm brick red): `role="alert"` text, failed and waiting state text, the danger button and chip (text on `console-danger-wash`), the danger notice, danger menu items and the "danger" work mark. Dark variants `console-danger-dark` / `console-danger-wash-dark`.
- **Avatar paints**: eight block triads an agent may be painted in: `orange`, `sage`, `paper`, `wood` (from the site, with the `*-top` / `*-base` faces the site hard-coded now tokenised) plus `sky`, `rose`, `lemon`, `plum`. Each has `-top` (lit top face), `-deep` (sides) and `-base` (underside). Paints are the agent's identity, not UI color: the only paint that leaks into chrome is `plum` (dreaming dot) and `plum-deep` (another person's desktop pointer). A retired agent is repainted `wood` with no eyes.

#### Neutral

- **Ground** (`console-ground`, the site's Desk): the body, the agent detail panel and the platform editor card.
- **Sidebar** (`console-sidebar`, the site's Desk Deep): the sidebar and the phone bottom bar.
- **Surface** (`console-surface`, the site's Paper): the conversation column and dialogs.
- **Raised** (`console-raised`): buttons, fields, the composer, menus, the floating preview, the selected tab, the thread line and root, work-card icon wells.
- **Inset** (`console-inset`): chips, the tabs tray, notices, work cards, the desktop stage, `kbd`, the avatar preview and `console-code-bg` for `pre`.
- **Ink** (`ink`), **Ink Soft** (`ink-soft`), **Ink Faint** (`console-ink-faint`): text, secondary text and captions, icons / table heads / placeholders / bottom-bar labels.
- **Line** (`console-line`, warm ink at 0.12) for every 1px divider and border; **Line Strong** (0.22) for field and composer borders and the steering rule. Hover washes are the warm shadow color at 0.03–0.07.

#### Named Rules

**The Sidebar Is the Deepest Plane Rule.** In both themes the planes order sidebar → ground → surface → raised, darkest to lightest, and the conversation sits on the lightest large plane. The dark theme is this order in warm near-black: `#120e0b` → `#201a16` → `#26201b` → `#2b2420`. No plane may be lighter than the one that floats on it.

**The One Accent Rule.** Orange is the only accent: selection, primary action, unread, focus. Status dots and chips use the world's paints (sage, wood, plum, danger) and are never used to point.

**The Desk at Night Rule.** Dark mode (`prefers-color-scheme: dark` unless `data-theme="light"`, or `data-theme="dark"`) swaps only the planes, inks, lines, washes and status tints. Orange, the paints and the cube's lighting are unchanged; the shadow base becomes black (`--shadow: 0 0 0`) and button edges become `rgb(0 0 0 / 0.35)`.

### Typography

**Display / Body Font:** Rubik (same self-hosted variable file as the site)
**Mono Font:** `ui-monospace, 'SF Mono', Menlo, Consolas, monospace` at 0.875em, only for commands, `pre` and `kbd`

**Character:** The same stamped face at desk-tool sizes. Weight carries hierarchy: 800 for headings, names and wordmark; 700 for chips, badges, section heads and work-card titles; 600 for buttons, labels, tabs and sidebar section heads; 400 for reading.

#### Hierarchy

Six fixed steps in use, `--t-xs` 0.75rem, `--t-sm` 0.8125rem, `--t-md` 0.9375rem, `--t-lg` 1.0625rem, `--t-xl` 1.25rem, `--t-2xl` 1.5rem (a seventh, `--t-3xl` 2rem, is declared and unused). No `clamp()`, no viewport units.

- **H1** (`console-h1`, 800, 1.5rem, 1.15, −0.015em): page and agent heads use 1.25rem (`t-xl`) in practice; 1.5rem is the base h1.
- **H2** (`console-h2`, 800, 1.25rem): section heads; 1.0625rem inside the panel and platform editor.
- **H3** (`console-h3`, 800, 1.0625rem): form groups; 0.9375rem in Ink Soft inside the panel.
- **Body** (`console-body`, 400, 0.9375rem, 1.5): messages (max 78ch), rows, fields, checks.
- **Label** (`console-label`, 600, 0.8125rem): buttons, field labels (Ink Soft), tabs, tables, work summaries, menu items (500), thread line, notices, panel leads.
- **Micro** (`console-micro`, 700, 0.75rem): chips, badges, bottom-bar labels (600), hints (400, Ink Faint), small buttons (600), the desktop pointer tag.
- **Author** (800, inherits body size, −0.01em): message author names; sidebar wordmark is 800 at 1.0625rem / −0.015em.

#### Named Rules

**The Fixed Scale Rule.** Console type never scales with the viewport. Pick one of the seven steps.

**The Bold Name Rule.** An agent is found by its face and its bold name: author names and the wordmark are 800; an unread row goes to 700. Nothing else in a row is bold.

### Layout

A full-height two-column grid: 260px sidebar (`console-sidebar-w`) and a flexible main column; the main column is a flex column so the transcript scrolls and the composer stays at the bottom. Header rows (sidebar head, page head, agent head) are 56px with a 1px Line below. The agent screen splits conversation | 380px panel (`console-panel-w`), the panel narrowing to 340px under 1100px. Page bodies pad 20px (16px on phones), narrow pages cap at 72ch, forms at 62ch, messages and work cards at 78ch. The platform screens are `minmax(0,1fr) 360px` with a sticky editor card, collapsing to one column under 1000px.

Rhythm: 10px between actions and chips, 12px between form columns, 16px between form fields and panel sections, 6px inside a message body, 4px between exchanges, 34px sidebar rows, 2px between segmented tabs.

Phones (≤ 820px): the sidebar becomes its own page, a fixed 60px bottom bar (`console-bottombar-h`, plus the safe-area inset) with four items takes over navigation, the panel goes full width and hides the conversation behind it, the agent head's word hides, transcript and composer pad 12px. The avatar builder stacks under 420px.

### Elevation & Depth

Tonal first, shadow second. Four flat planes do most of the work (see the Sidebar Is the Deepest Plane Rule); borders are 1px Line. Three warm shadows remain, all blurred, and the block edge is now an inset. In the dark theme the shadow base is pure black at the same alphas.

#### Shadow Vocabulary

- **Block edge** (`box-shadow: inset 0 -3px 0 var(--edge), 0 1px 0 rgb(43 36 32 / 0.08)`): resting button; the edge is `paper-deep` (default), `orange-deep` (primary), `danger` (danger) or transparent (ghost). Chips carry `inset 0 -2px 0 *-deep`; the badge too. Pressed: `inset 0 -1px 0`.
- **Button hover** (`0 6px 14px -6px rgb(43 36 32 / 0.35)`): with a 1px lift.
- **Lift** (`--lift`, `--lift-hover`): the site's button shadows are declared in tokens.css but no console rule uses them; the console button uses the edge and hover shadows above.
- **Float** (`--float: 0 18px 44px -14px rgb(43 36 32 / 0.45)`): anything that leaves the desk: dialogs, menus, the floating preview and its pills, the desktop screen.
- **Tab select** (`0 1px 2px rgb(43 36 32 / 0.15)`): the selected segmented tab's raised face.
- **Focus ring** (`0 0 0 3px rgb(242 106 27 / 0.25)`; composer 0.2): fields and the composer on focus, with the border turning orange. Everything else uses the global `3px solid orange` outline, offset 2px, radius 6px.
- **Dialog backdrop** (`rgb(43 36 32 / 0.4)` with `blur(2px)`).

#### Named Rules

**The Inset Edge Rule.** The block's bottom edge is drawn as an inset shadow (3px rest, 1px pressed) so a control's box never grows. Buttons lift 1px on hover and sink 1px when pressed, over 160ms on the settle ease.

**The Flat Selection Rule.** The selected sidebar row and the selected palette hit are flat orange fills with no edge and no shadow: selection is paint on the desk, not a block to press.

### Shapes

Radii step 6 / 8 / 10 / 14px (`console-sm` … `console-xl`): 6px for chips, tabs, section heads, menu items, the account initial and the focus ring; 8px for buttons, fields, rows, messages, work cards, thread lines, tables' `pre`, the desktop screen and notices; 10px for the composer, menus, the floating preview, the login banner, the platform editor and avatar preview; 14px for dialogs. Two rounder exceptions: the unread badge (10px on a 20px height) and the preview pills (`999px`), the console's only pill; status dots and the pointer tag's dot are 8px circles. Borders are 1px Line everywhere except fields (Line Strong) and the login banner (orange at 0.35). The cube keeps square faces.

### Components

#### Buttons

Block buttons, shrunk to tool size.

- **Shape:** 8px radius, 36px tall, padding 7px 14px 9px, 600 at 0.8125rem, 8px gap to an 18px lucide icon
- **Default (`console-button`):** Raised face, Paper Deep inset edge
- **Primary (`console-button-primary`):** Orange face, Orange Deep edge, `on-paint` ink text; one per surface (Send, Save, Create)
- **Danger (`console-button-danger`):** Danger Wash face, Danger edge and text (Cancel while a turn runs, Retire)
- **Ghost (`console-button-ghost`):** no face, no edge; hover is a 6% warm wash with no lift
- **Small (`console-button-sm`):** 30px, 4px 10px 6px, 0.75rem, 16px icon. **Icon (`console-button-icon`):** 36px / 30px square, no padding.
- **States:** hover lifts 1px with the button-hover shadow; active sinks 1px and the edge drops to 1px; disabled is 55% opacity with no lift.
- **Link button (`.link-btn`):** plain text with the site's 2px orange underline (offset 0.18em); "Steer this work", "Thread #n".

#### Chips

- **Style (`console-chip`):** 6px radius, 2px 9px 3px, 700 at 0.75rem, Inset face, Ink Soft text, 12px icon
- **Tones:** `console-chip-paint` (orange face, 2px Orange Deep edge), `console-chip-sage`, `console-chip-wood` (same edge in their deep tone, ink text fixed to `#2b2420` so it holds on dark), `console-chip-danger` (wash face, danger text, no edge)
- **Status dot:** 8px circle: Ink Faint at rest, orange pulsing (1.6s, ring to 5px) while working, plum dreaming, Success idle, Sleep sleeping, Wood Deep retired

#### Inputs / Fields

- **Style (`console-input`):** Raised face, 1px Line Strong border, 8px radius, 38px min height, 8px 12px, body size at 400; textareas 96px min and vertically resizable; labels 600 Ink Soft with an optional 400 Ink Faint hint on the right; checkboxes 18px with orange `accent-color`
- **Focus:** border orange plus the focus ring
- **Notices:** 8px radius, 10px 12px, Inset; danger (wash + danger text) and paint (Orange Wash) variants

#### Tables and lists

- **Table:** 0.8125rem; 600 Ink Faint heads, 8px 12px; cells 9px 12px, 1px Line under every row; row hover a 3% wash; selected row Orange Wash; numeric columns right-aligned tabular
- **List:** 10px rows divided by Line, 600 titles; **key/value** grids at 0.8125rem with 600 Ink Faint terms

#### Tabs

A segmented control (`console-tabs`): Inset tray, 8px radius, 3px padding, 2px gaps; each tab 5px 12px, 6px radius, 600 Ink Soft; the current tab is a Raised face with ink text and the tab-select shadow. Inside the panel head the tray goes transparent and tabs tighten to 5px 8px.

#### Panel

The contextual right panel (`console-panel`): 380px, Ground plane, 1px Line on its left, a head with the tabs and a close control, a 16px body in 16px-gapped sections with 1.0625rem h2 and Ink Soft 0.9375rem h3. On phones it is the whole screen. Forms inside it drop the 62ch cap.

#### Work cards and the thread line

- **Work card (`console-work-card`):** a `details` block in Inset with a 1px Line border and 8px radius, max 78ch. The summary row (6px 10px, 0.8125rem) holds a 22px Raised icon well (6px radius, 14px icon), a 700 capitalised title, the command in mono Ink Soft that ellipsises, and a 16px mark: Success ok, Orange Deep warn or spinning (1.2s), Danger fail. The body (0 10px 10px) stacks `pre` output on Code Bg with a Line border.
- **Thread line (`console-thread-line`):** "3 tasks · 2 done" as a Raised 8px pill-shaped button with a Line border, 600 at 0.8125rem, with 20px Sage Deep initials overlapping by 6px and ringed 2px in Raised; hover turns the border orange.
- **Thread root:** Raised, Line border, 8px, 10px 12px; children 10px rows divided by Line. **Steering** replies hang off a 1px Line Strong left rule, 10px in.

#### Messages

Grouped as exchanges (4px gaps, 6px vertical padding): a 36px-column grid with the cube (36px) or a 32px Wood Deep initial square for people, 10px to a body that stacks head, text (pre-wrap, 78ch) and an actions row; hover washes the message 3.5%. Head: 800 author, Ink Soft time, state text in Orange Deep while running and Danger when failed or waiting. Relayed room messages lead with a `#` well and a Room chip.

#### Composer

`console-composer`: a Raised 10px box with a Line Strong border; a borderless textarea (12px 14px 6px, 44–240px tall) over a bar (4px 8px 8px 10px) holding the queue note (ellipsised, wraps on phones) and the primary Send; while a turn runs the danger Cancel appears in the message actions. Focus-within turns the border orange with the 0.2 ring. Read-only composers lose the box and gain a top Line.

#### Navigation

- **Sidebar:** `console-sidebar` plane with a Line on its right; 56px head with the 800 wordmark, search and add icon buttons; a scrolling body padded 8px with collapsible sections (700 Ink Soft 0.8125rem heads with a 14px chevron that turns −90° when collapsed); rows 34px / 8px radius at body size with an 18px Ink Faint icon or the agent's cube, a 6% wash on hover, the flat orange current fill (icons, badges and flags go `on-paint` / Paper), 700 label when unread, an orange unread badge (20px, 800 at 0.75rem, 2px Orange Deep edge) or a Line-tinted count, and an Orange Wash flag for room notes; a Line-topped account row with a 26px Ink initial square and a details-based menu.
- **Menus:** Raised 10px lists with a Line border and the Float shadow, 6px padding, 8px 10px items at 500 / 0.8125rem with 16px Ink Faint icons and a 7% hover wash; danger items in Danger.
- **Command palette:** a 560px dialog 12vh from the top, a 1.0625rem borderless input over a 6px-padded hit list; the selected hit is flat orange.
- **Phone bottom bar (`console-bottombar`):** fixed, Sidebar plane, Line on top, four equal items (22px icon over a 600 / 0.75rem label) in Ink Faint; the current item is Orange Deep, not a fill; badges ride 6px right of centre.

#### Dialogs

`console-dialog`: Surface plane, 14px radius, Float shadow, `min(520px, 100vw − 32px)`, a 20px 22px 22px body in 16px-gapped sections with a space-between head; backdrop is warm ink at 0.4 blurred 2px.

#### Cube (console moods)

The site's cube (same six-face build, lighting gradients, rest pose `rotateX(-18deg) rotateY(28deg)`, perspective 9×), defaulting to 36px, painted in one of the eight paints with one of four eye shapes (round, wide, sleepy, dot) and an optional accessory (cap, glasses, bow, antenna) drawn in ink and paper so it reads on any paint. Its mood is derived from the agent's activity (`moodOf`) and never stored:

- **Idle:** open eyes, blush, blink every 5.4s, glance left then right every 9s (offset per block by `--delay`)
- **Working:** body nods 3° over 2.4s, eyes look down 6% of the block and blink faster (3.2s), no glance
- **Dreaming:** body drifts (−4° pitch, +6° yaw, 6% rise over 6s), eyes half-lidded with a paint-coloured lid, no blink
- **Sleeping:** closed smile-stroke eyes, no blush, three Ink Soft z's (30% of the block, 900) drifting up-right over 3.9s, 1.3s apart
- **Desktop:** a cool screen glow (`rgb(220 240 255 / 0.45)` radial from 50% 70%) breathes on the front face over 2.2s; eyes look down-left
- **Waiting (login handoff):** eyes 12% larger, looking up, no blink or glance
- **Retired:** repainted Wood, no eyes, no accessory

A `follow` block (the agent head) turns toward the pointer (±16° yaw, ±10° pitch, eased 12% per frame) and its eyes glance up to 7px × 5px. Under `prefers-reduced-motion` the body, eyes, z's and glow stop (`animation: none`), the first z stays visible, the pointer follow is never attached, and base.css collapses every transition and animation to 0.001ms.

#### Floating preview

`console-pip`: a fixed 320px Raised window (10px radius, Line border, Float shadow) with a grab-handle head (6px 6px 6px 10px, 700 name at 0.8125rem, controls) over a 16:10 black screen that zooms in on click. When collapsed it becomes `pip-pill`s stacked top-left: `999px` Raised pills with the same border and shadow. The full desktop page uses an Inset stage with a 16:10 black `desktop-screen` (8px radius, Float shadow); while you are controlling it the canvas carries an inset 3px orange outline and a crosshair. Pointer tags are 0.75rem / 700 labels with an 8px dot: orange for the agent, Sage Deep for you, Plum Deep for another person. The login handoff banner is an Orange Wash 10px box with an Orange Deep 20px icon.

#### Avatar builder

A 120px Inset preview (10px) beside rows of 32px Raised options (8px radius, 2px transparent border, `inset 0 -2px 0` 12% warm edge); the checked option gets an orange border; paint swatches are 18px blocks with a 3px darker inset edge.

### Do's and Don'ts

#### Do:

- **Do** place the conversation on Surface, management on Ground, chrome on Sidebar, and controls on Raised; depth is the plane, not a shadow.
- **Do** keep orange to selection, the one primary action per surface, unread marks and focus; use the sidebar's flat orange fill for "you are here" and the inset-edge block for "press me".
- **Do** set every size from the seven-step rem scale and the 6 / 8 / 10 / 14px radii.
- **Do** show an agent as its cube and bold name wherever it appears (sidebar, head, messages, thread faces) and let the cube's mood come from activity.
- **Do** use Float for anything that leaves the desk (menus, dialogs, the preview) and warm blurred shadows only; in the dark theme the base is black at the same alphas.
- **Do** honour reduced motion: static cube, first z shown, no pointer follow, no pulses.

#### Don't:

- **Don't** bring the site's scenery into the console: no shelves, mugs, plants, slabs or painted-block scenes; the cube is the only illustration.
- **Don't** put a kicker or eyebrow above a heading; the section heads in the sidebar and panel are headings, not eyebrows.
- **Don't** add a second accent: status dots and chips are the world's paints and never point; a non-orange filled control is a mistake.
- **Don't** use fluid type, a heading above 1.5rem, uppercase tracking or a second typeface; mono is for commands and output only.
- **Don't** give the selected sidebar row an edge or a shadow, or give a block button a zero-blur drop.
- **Don't** make any plane lighter than the plane that floats on it; the sidebar is always the deepest.

<!-- console:end -->
