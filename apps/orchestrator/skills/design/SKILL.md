---
name: design
description: How to design and build interfaces people trust and remember. Load this before creating or changing anything a person sees, such as a web page, app screen, dashboard, form, component, email, slide, chart or poster, and when asked to make something look better, reviewing a design, or choosing colours, type or layout. Not for backend-only work.
---

# Design

Models converge. Left alone, you will produce the statistically average interface: a safe font, a purple or warm-beige wash, three identical cards, a big number with a small label. It looks fine and belongs to nobody. Your job is to make choices a person would recognise as choices.

A brief from the person always wins. If they pinned a palette, a font, an era or a brand, honour it exactly, even where it breaks the advice below. If the product already has a design system, extend it: match its tokens and components rather than inventing beside them.

## 1. Decide what this surface is for

Name who is looking at it, where they are, and the one thing success means. Then pick the register, because it changes every later decision:

- **Persuade** (landing page, campaign, pricing): the person decides and acts. Hold a point of view and earn attention.
- **Operate** (app, dashboard, editor, settings): the person gets a task done, often daily. Clarity, density, familiar patterns and speed beat expression. Character lives in the details.
- **Read** (docs, article, report): the person understands something. Structure and typography do the work.
- **Experience** (portfolio, gallery, game): the work itself leads and the interface recedes.

## 2. Commit to a direction before writing code

Write down, briefly:

- **A concept in one sentence,** drawn from this subject's own world, not from "modern and clean".
- **Colour:** four to six named values with their hex codes, one dominant and one sharp accent. Pick light or dark from where the person is using it.
- **Type:** roles for display, body and data, with a real scale. Choose a face because it suits this subject.
- **Layout:** the idea that organises the page, and what the first screen shows.
- **The one bold move.** Spend your boldness in one place and keep everything else quiet around it.

Then test it: _would I have written this for any other product in the category?_ If yes, change it. Ask of each element: is this a decision, or a default I reached for?

## 3. Avoid the template

These are defaults to question, not laws. A brief or a concept can earn any of them, but reaching for one because the axis was free means you were not deciding.

- Rows of identical cards with an icon, a heading and two lines of text; cards inside cards.
- A big number with a small label as the main idea of a page.
- A small uppercase label sitting above every heading; 01 / 02 / 03 on things that are not a sequence.
- Gradient text, glass effects, and glowing shadows used as decoration.
- A thick coloured stripe down one side of a card or alert.
- Sparklines and progress rings standing in for real content.
- Emoji used as icons; use one drawn icon set at one stroke weight.
- Monospace as a costume for "technical"; keep it for code and data.

Lists like this go stale, so keep asking the deciding question rather than only checking this one.

## 4. Build it to a floor

- **Real content:** use real or realistic copy and data, including the longest name, the empty list and the huge number. Do not invent customers, numbers or testimonials.
- **Contrast:** text at least 4.5:1, large text and control boundaries at least 3:1. Colour is never the only signal.
- **Type:** body text 65 to 75 characters wide, tabular numerals in data, more space above a heading than below it.
- **Space:** tight within a group, generous between groups. Align to a consistent scale.
- **States:** draw hover, focus, active, disabled, loading, error and empty, not just the happy path. Keep focus visible and never hidden by sticky elements.
- **Semantics:** buttons for actions, links for navigation, a real label on every input. Never block zoom or paste, or remove an outline without a replacement.
- **Touch and small screens:** targets at least 24 px, and 44 px where touch is the main input. Nothing is lost at 320 px wide, with no sideways page scroll. Respect safe areas on phones.
- **Motion:** one authored moment rather than effects everywhere. Ease out quickly from a visible default, keep it interruptible, and honour reduced-motion. Avoid animating everything with a blanket transition.
- **Words:** controls name their action and the same verb carries through a flow. Errors say what went wrong and how to recover. Write in the product's voice, in plain language.
- **Product behaviour:** shareable state lives in the URL, destructive actions confirm or can be undone, and dates and numbers respect the locale.
- **Parts you did not draw:** text selection, scrollbars, focus rings and underlines are part of the design. Theme them.

## 5. Look at what you made

Reasoning about a layout is not seeing it. Render it, at the widths it will really ship (a phone and a desktop at least), in each theme, with real content, and check it with motion on.

Inspect once, list everything wrong, fix it as one batch, and check once more. Then stop polishing. Check computed values rather than trusting your eye: contrast, tap target sizes, overflow, text that wraps badly. If you cannot render it here, say so plainly instead of implying you looked.

When you report back, name the direction you chose and why, what you verified and on which screens, and anything you could not check.
