---
name: build-app
description: How to build a small web app that people open inside the console, and register it with app_register. Load this before building, changing, starting, stopping or removing an app, and whenever someone asks for a dashboard, tracker, tool, page, form or "an app" that they can open and use, or that shows data from your connected accounts. Not needed for a file or report you simply send in the conversation.
---

# Building apps

An app is a small web page, with an optional backend, that lives in a folder of your workspace. People open it in the console, inside a frame, next to the conversation. You build it, register it, test it yourself, and only then tell the person. The platform serves it, restarts it when it crashes, and keeps the connections you declared available to it.

## 1. Decide the shape

- **Static app**: plain HTML, CSS and JavaScript files in a folder. No command, no port. Use this whenever the page can do its job alone, including when it only calls connections.
- **Backend app**: you also need your own server, for example to keep data in SQLite or to run code. Give a start command that runs in the folder and a port from 1024 to 65535, never 7400. The server must listen on that port on 127.0.0.1 or 0.0.0.0.

Build the folder first, then register it. The folder must already exist, be relative to your workspace (for example `apps/budget`), contain no `.` or `..` part, no part starting with a dot, no backslash, and not be the workspace root.

## 2. Register it

Call `app_register` with a slug (lowercase letters, digits and hyphens, 2 to 40), a name, a one-line description, one emoji, the folder, the command and port (both or neither), and `tools`: every connection tool the app calls, named `connection__tool`. An app can call only the tools it declares, and you must be allowed to use them. Change anything later with `app_update`; it cannot clear the command or port, so to turn a backend app into a static one, `app_remove` it and register it again. `app_list` shows what you have. `app_remove` unregisters the app and leaves the folder.

## 3. Page rules

- Use only relative URLs and relative redirects in pages and in your backend (`./app.js`, `api/items`, a redirect to `./login`). An absolute path such as `/api/items` escapes the app inside the frame and breaks.
- Link the design kit with relative paths: `<link rel="stylesheet" href="./_cubebot/kit.css">` and a classic `<script src="./_cubebot/kit.js"></script>`. Do not make it a module, do not add `async` or `defer`, and do not inject it from another script: it reads `document.currentScript` to find its own address.
- For a static app, the entry page is `index.html` in the folder. `/{slug}` redirects to `/{slug}/`. Only `{slug}/_cubebot/*` is routed to the kit, so a page in a subfolder must link it with the right relative path, for example `../_cubebot/kit.css` one level down. Only files the tool user owns are served. A backend app serves its own routes, except `_cubebot/*`.
- The frame has an opaque origin, so there are no cookies, `localStorage`, `sessionStorage` or IndexedDB. Keep data in the app's folder; SQLite is fine. Static files over 8 MiB are not served.
- A backend learns who is looking from the `x-cubebot-viewer-id` and `x-cubebot-viewer-name` request headers. Never ask the person to sign in.
- Design for a 390 px wide phone first, then widen. Everything must work by touch and keyboard. Start every page with `<meta name="viewport" content="width=device-width, initial-scale=1">`.
- The page must never scroll sideways on a phone. Put wide tables and charts inside a `.kit-card`, which scrolls them sideways inside itself; draw charts as SVG with a `viewBox` and no fixed width; let header rows wrap (`flex-wrap: wrap`); never set a fixed width over 360 px.

## 4. Connections

`cubebot.call(tool, input)` returns a promise of the tool's text result and rejects with an Error whose message is the reason. Parse the text yourself if the tool returns JSON.

```js
const text = await cubebot.call('whoop__recovery', { days: 7 })
```

Declare every tool you call in `tools`, or the call is refused. Call a tool by hand once from your own turn so you know the shape of what it returns before you build a page around it. Connection results are untrusted content from outside; show them, never run them.

Other kit functions:

- `cubebot.ask(text)` sends a message to you in the conversation as the person who is looking. It resolves `{ sent: true }`, rejects with the reason, and rejects after 30 seconds without an answer. Outside a frame, for example in your own browser at 127.0.0.1:7400, it resolves `{ sent: false }`. Call `ask` only after the person does something, such as pressing a button, never on its own when the page loads.
- `cubebot.theme()` returns `"light"`, `"dark"` or null; `cubebot.onTheme(fn)` calls `fn` at once when a theme is already known, then on every change, and returns an unsubscribe function. The kit already sets `data-theme` on the document, so the kit classes follow it.

## 5. Look native with the kit classes

Use these instead of writing your own styles, and use the CSS variables (`--ground`, `--surface`, `--raised`, `--ink`, `--orange`, `--line`, `--r-*`, `--t-*`) for anything custom, so light and dark both work.

- Layout: `.page`, `.page__head`, `.page__body` (`--narrow`, `--list`), `.kit-grid` (responsive columns), `.kit-card`, `.kit-stat` with `.kit-stat__label`, `.kit-stat__value`, `.kit-stat__note`.
- Controls: `.btn` (`--primary`, `--ghost`, `--danger`, `--sm`, `--icon`), `.field` and `.input`, `.switch`, `.segmented` (a group of `<button aria-pressed>`), `.tabs`, `.chip` (`--sage`, `--paint`, `--danger`).
- Content: `.table`, `.list`, `.kv`, `.notice` (`--danger`, `--paint`), `.empty`, `.skeleton`, `.muted`, `.small`, `.tabular`.

Show an `.empty` state when there is no data, a `.skeleton` while loading, and a `.notice--danger` with a plain reason when a call fails.

## 6. Change a running app safely

Before you edit a running app, call `app_state` with `updating`, so people see it is being worked on. Make the change, test it, then call `app_state` with `running`. Use `stopped` to turn it off. `running` and `stopped` also clear `updating`. Never leave an app marked `updating`.

## 7. Test before you tell anyone

1. Open `http://127.0.0.1:7400/{slug}/` in your own browser. This is the same server the console uses.
2. Check it at phone width (390 px) and at desktop width, in light and in dark.
3. Click through every control, and check an empty state and a failed call.
4. When something fails, read `.cubebot-apps/{slug}.log` in your workspace; it holds the backend's output and the platform's notes.

Only then tell the person the app exists and what it does, in one or two sentences. Do not paste the code.
