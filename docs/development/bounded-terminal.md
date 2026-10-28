# Terminal execution

Terminal execution is disabled by default. `terminal_enabled: true` in the explicit controller configuration sets `CUBEBOT_TERMINAL_ENABLED` in the sandbox, and the agent then offers the terminal tool to the model.

The native provider adapter carries stateless function calls. The agent owns the terminal schema, argument validation, model loop and command execution. Under ADRs 0083 and 0084 commands run as children of the agent inside its sandbox; the sandbox boundary, not a separate container or UID, is the isolation claim, and only the `kvm` backend can carry it. There is no command-count, turn-duration, command-duration or output-capture budget. Work continues until the turn ends, is cancelled or the sandbox stops. Commands run one at a time.

Each command runs through `/bin/bash --noprofile --norc` in its own process group with `HOME` set to `CUBEBOT_WORKSPACE_DIRECTORY` and no inherited agent environment. Its complete combined stdout/stderr goes to `.cubebot-terminal-output/<call-id>.log` in the workspace. The result contains a tail preview and the full file path.

No blind replay: each call is a `tool_call` turn event and its outcome a `tool_result` event on the [runtime channel](reference.md#runtime-channel). The agent journals the call under `CUBEBOT_JOURNAL_DIRECTORY/tools` before the command starts, and the result after its process group stops. After a crash it reports a journaled result, or `uncertain` for a call that never finished, and never runs the command again. A call without a definite result after its run ended is shown as uncertain, and retrying that turn is blocked.

Cancelling the turn stops the command's process group and records `stopped` with reason `cancelled`. Cancellation cannot undo filesystem or external effects. A descendant that leaves the process group survives until the sandbox stops.

Run `just check`, `just integration` and `just smoke`. The cluster terminal qualification and container-isolation probes were removed with the two-container layout; terminal behavior inside a `kvm` sandbox needs new qualification once that backend exists.
