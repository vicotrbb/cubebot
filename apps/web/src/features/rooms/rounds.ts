import type { RoomMessage } from '../../api/client'

type Round = {
  message: RoomMessage
  deliveries: RoomMessage['deliveries']
}

/** In a task an agent posts while it works, so its reply, the result, belongs after those posts rather than under the message that started it. */
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
