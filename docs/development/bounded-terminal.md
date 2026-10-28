# Terminal execution

Terminal execution is disabled by default. `terminal_enabled: true` in the explicit controller configuration sets `CUBEBOT_TERMINAL_ENABLED` in the sandbox and grants new turns terminal eligibility. Existing text-only authorities remain ineligible for tools.

The native provider adapter carries stateless function calls. The agent owns the terminal schema, argument validation, model loop and command execution. Under ADRs 0083 and 0084 commands run as children of the agent inside its sandbox; the sandbox boundary, not a separate container or UID, is the isolation claim, and only the `kvm` backend can carry it. There is no command-count, turn-duration, command-duration or output-capture budget. Work continues while it is authorized. Commands run one at a time.

Each command runs through `/bin/bash --noprofile --norc` in its own process group with `HOME` set to `CUBEBOT_WORKSPACE_DIRECTORY` and no inherited agent environment. Its complete combined stdout/stderr goes to `.cubebot-terminal-output/<invocation-id>.log` in the workspace. The result contains a tail preview and the full file path.

Durable attempts and no blind replay: before anything runs, the orchestrator records the invocation (`authorize_terminal`) and the agent journals its intent in a private attempt store under `CUBEBOT_JOURNAL_DIRECTORY/attempts`. The outcome is journaled after the process group is stopped. Immutable invocation IDs bind the command and turn authority; a repeated ID with different content is rejected, and a repeated ID returns its recorded outcome without running again. After a crash, recovery reads the local attempt store for every invocation the orchestrator still holds open. Intent without an outcome stays uncertain and is never executed again. Automatic turn retry is blocked while terminal effects remain uncertain.

Active turns renew short authority windows with the orchestrator. Loss of authorization or expiry stops the command's process group and records `authority_expired`. Cancellation cannot undo filesystem or external effects. A descendant that leaves the process group survives until the sandbox stops.

Run `just check`, `just integration` and `just smoke`. The cluster terminal qualification and container-isolation probes were removed with the two-container layout; terminal behavior inside a `kvm` sandbox needs new qualification once that backend exists.
