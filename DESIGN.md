---
name: Cubebot
description: Painted wooden blocks on a warm desk. A 3D orange cube with eyes is the agent; the product is explained by stacking more blocks.
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
rounded:
  focus: '6px'
  chip: '8px'
  block: '10px'
  face: '0'
spacing:
  chip-gap: '10px'
  item-gap: '30px'
  para-gap: '20px'
  stack-gap: '36px'
  gutter: 'clamp(16px, 5vw, 64px)'
  section: 'clamp(56px, 9vw, 120px)'
  measure: '62ch'
  container: '1180px'
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
