# Tool execution

Tools are disabled by default. `terminal_enabled: true` in the explicit controller configuration sets `CUBEBOT_TERMINAL_ENABLED` in the sandbox, and the agent then offers its tools to the model: `bash`, `read_file`, `write_file`, `edit_file`, `glob`, `grep`, and the tools of any configured MCP server.

The native provider adapters carry stateless function calls, several per response. The agent owns the `Tool` trait and registry in `apps/agent/src/tools`, argument validation, the model loop and execution. Under ADRs 0083 and 0084 tools run as children of the agent inside its sandbox. There is no call-count, turn-duration, call-duration or output-capture budget, and no lease. Work continues until the turn ends, is cancelled or the sandbox stops.

## Tool user

Every tool runs as the unprivileged user named by `CUBEBOT_TOOL_USER` (`uid:gid`, never root), in its own process group, with `HOME` and the working directory set to `CUBEBOT_WORKSPACE_DIRECTORY` and no inherited agent environment. File tools run as that user too, so the kernel bounds what a call can touch: `read_file` is `cat`, `write_file` a fixed bash script fed the content on standard input, `glob` bash pathname expansion (`**` needs bash 4 or later and is refused with exit code 2 otherwise), `grep` `grep -rnIE`, and `edit_file` re-executes `cubebot-agent edit-file` as the tool user. Model arguments are passed as program arguments or standard input and are never evaluated as code. With tools enabled the agent refuses to start without a valid tool user.

Only a privileged agent can switch users. The guest agent runs as root, and the controller sets the tool user to uid 10001.

## Output

Each call's complete combined output is written by the tool user to `.cubebot-tool-output/<call-id>.log` in the workspace. The result shows the whole output when it is at most 16 KiB, otherwise its first and last 8 KiB with the omitted byte count, followed by the file path. The agent reads previews only from a regular file the tool user owns, opened without following links. `grep` skips that directory.

## Parallel calls and events

Each call is a `tool_call` turn event and its outcome a `tool_result` event on the [runtime channel](reference.md#runtime-channel). All calls in a response are journaled under `CUBEBOT_JOURNAL_DIRECTORY/tools` before any starts; they run in parallel and each result is journaled and reported as it finishes. A call naming an unknown tool or with invalid arguments still gets its `tool_call` event (unparsable arguments kept as text) and a result with exit code null saying why it was not run, so the model can correct it while the other calls run. After a crash the agent reports a journaled result, or `uncertain` for each call that never finished, and never runs a call again. A call without a definite result after its run ended is shown as uncertain, and retrying that turn is blocked.

Cancelling the turn kills every unfinished call's process group and records `stopped` with reason `cancelled`. Cancellation cannot undo filesystem or external effects. A descendant that leaves the process group survives until the sandbox stops.

## MCP servers

`CUBEBOT_MCP_SERVERS` is a JSON array of servers, each `{"transport":"stdio","name":..,"command":..,"args":[..],"env":{..}}` or `{"transport":"http","name":..,"url":..,"headers":{..}}`. Names are unique letters, digits or hyphens; server tools are offered as `<server>__<tool>`, so they cannot shadow a built-in or another server. Stdio servers run as the tool user and speak newline-delimited JSON-RPC; HTTP servers use Streamable HTTP with session and protocol-version headers, responses in JSON or SSE, and no redirects. The JSON-RPC client is in-house. Servers initialize at agent startup with a 30-second limit; one that fails is logged and left out. A stdio server that exits is started and initialized again by the next call; calls it was running are `uncertain`. An HTTP 404 for the current session starts a new session and resends that request once, since the server did not process it. Server pings are answered on stdio and while reading an HTTP event stream, which is read as it arrives. A cancelled call sends `notifications/cancelled`; the result is still `stopped`, since the server may already have acted. Results are retained like other output; `isError` results have code 1. A request the server never received reports exit code null, a definitive JSON-RPC or HTTP 4xx refusal is a code 1 result, and a lost response is `uncertain`.

Not yet supported: grants per tool (ADRs 0040, 0044), orchestrator-managed server configuration (ADR 0042), and catalog changes after startup or a restart. Server-to-client requests other than ping, such as sampling, roots and elicitation, are refused because ADR 0040 does not authorize them.

Run `just check`, `just integration` and `just smoke`. Tool behavior inside a `kvm` sandbox, including user separation, needs qualification once that backend exists.

## Computer use

With `computer_use_enabled` the agent also offers a `computer` tool (ADR 0085). It is declared as the provider's own tool, Anthropic's `computer_toolset_20260801` (with `zoom`, `hold_key`, `left_mouse_down`, `left_mouse_up` and `cursor_position` turned off) or OpenAI's `computer`, so the model sees a 1280x800 screen it controls with screenshots, clicks, drags, scrolls, typing and keys. Both forms become the same requests to the sandbox's screen arbiter over the guest's vsock desktop port, each tagged with the tool call's id. An Anthropic member is one call; an OpenAI call runs its batch of actions and answers with a screenshot. A screenshot result is the `screenshot` tool outcome, whose base64 PNG goes to the model as an image and stays in the ledger with the call. While a person holds control, input is refused with "a person has control" and screenshots still work; an OpenAI call then gets only its screenshot, since its output has no text.

Guest init starts the desktop as the tool user: Xorg with the modesetting driver on the virtio-gpu display and the evdev driver on the virtio keyboard and tablet (`apps/agent/xorg.conf`), then openbox, xterm and Chromium. Chromium runs with `--no-sandbox`: the guest kernel has no user namespaces, and the VM is the boundary. Any guest process can open the desktop port and send input as the agent, with any call id; that is within the agent's authority (ADR 0083), not a person's.
