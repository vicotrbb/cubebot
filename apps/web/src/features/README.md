# Features

Each feature owns its components, state, styles and API calls: `shell` (sidebar, bottom bar, palette), `chat` (conversation, composer, work cards), `agent` (details panel tabs, avatar builder), `desktop` (socket, full view, floating preview, login banner), `memory`, `skills`, `routines`, `vault`, `files`, `knowledge` (the wiki: tree, block editor, autosave), `rooms`, `platform`. Routes under `src/routes` compose features and own nothing else; a tab is a child route so it has a URL. `docs/design/console.md` is the design.
