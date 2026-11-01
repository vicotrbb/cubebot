# ADR 0086: A vault the desktop arbiter fills

Status: Accepted

Date: 2026-11-01

ADR 0038 leaves secret storage for website logins open, and ADR 0039's login handoff needs a person at the desktop every time. An agent that can read a password can leak it into its context, tool output or files; one that can only ask for it to be typed cannot.

Decision: The orchestrator keeps a vault of logins and secrets, and the desktop arbiter (ADR 0085) types them for the agent. The user selected this on 2026-11-01.

- **Items.** One `vault_items` table: scope installation or agent, kind `login` (site, username, password) or `secret`, a name, the secret as an ADR 0066 Credentials record bound to the item's id, who created it, and when it was last filled. Usernames, sites and names are metadata; only the secret is encrypted.
- **Access.** An installation item has an ADR 0081 access policy, all agents by default or explicit grants, and only administrators manage it. An agent's own item belongs to that agent alone and is managed with `browser.control` on it; `browser.view` lists the items an agent may fill. No API response carries a secret.
- **Filling.** The agent's `vault_list` returns metadata only. `vault_fill` (item, field) asks the orchestrator, which decrypts the field and sends a `fill` request on the desktop host channel; the arbiter types it at the focus, refusing it like the agent's own input while a person holds control, during a login handoff, or before the agent looks again. A person holding control fills from the Desktop tab the same way, as themselves. The text never returns to the agent, and the arbiter logs only the item id and the text's length.
- **Saving.** The agent's `vault_put` saves a secret it made, such as for a new account, as its own item, replacing one with the same name. Its `secret` argument is replaced with `[redacted]` before the call reaches the agent's journal or the turn ledger; the model's own request still held it. During a login handoff the person can save the login and have it typed in one step.
- **Typable only.** The arbiter types printable ASCII and spaces on a US layout, so a username or secret with any other character is refused when saved, not when filled.

This amends ADR 0043 for website logins: an agent may hold logins of its own besides installation-managed ones.

Alternatives considered: Giving the agent the secret as tool output, which puts it in the ledger and model context. A password manager inside the guest, which agent-run code can read. Typing through the browser tools' DevTools pipe, which bypasses the arbiter's mode and its record of who typed.

Open details: key rotation for vault records; per-agent exclusions from all-agents items; filling more than one field per call; items for non-US layouts.

Validation scenarios: A fill types the secret and neither the turn ledger, the model's requests nor the desktop records hold it; a `vault_put` secret is redacted in the ledger; an item with an untypable character is refused; the agent's fill is refused while a person holds control; a person's fill is refused unless they hold control; an agent without access to an installation item can neither list nor fill it.
