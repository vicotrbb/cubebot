import type { RoomMessage } from '../../api/client'

/** A row of a task's room: a message and the agents' replies shown beneath it. */
type Round = {
  message: RoomMessage
  deliveries: RoomMessage['deliveries']
}

/** In a task an agent posts while it works, so its reply, the result, belongs after those posts
 *  rather than under the message that started it. A person or the orchestrator speaking starts the next round. */
export function rounds(messages: RoomMessage[]) {
  const rows: Round[] = []
  let held: RoomMessage['deliveries'] = []
  const settle = () => {
    const last = rows.at(-1)
    if (last) last.deliveries = held
    held = []
  }
  for (const message of messages) {
    if (!message.author_agent_id) settle()
    rows.push({ message, deliveries: [] })
    held = [...held, ...message.deliveries]
  }
  settle()
  return rows
}
