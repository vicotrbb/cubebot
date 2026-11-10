import { useQueryClient } from '@tanstack/react-query'
import { useNavigate } from '@tanstack/react-router'
import { Trash2 } from 'lucide-react'
import { useState } from 'react'
import { api, failure, ok } from '../../api/client'
import type { RoomSummary } from '../../api/types'
import { keys } from '../../app/queries'
import { Alert, confirmAction } from '../../components/primitives'

export function DeleteRoom({ room }: { room: RoomSummary }) {
  const client = useQueryClient()
  const navigate = useNavigate()
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  async function remove() {
    const yes = await confirmAction({
      title: `Delete #${room.name}?`,
      body: 'It leaves everyone’s sidebar and nobody can write in it again. Agents keep their own conversations, and administrators can still read the room’s history.',
      action: 'Delete room',
    })
    if (!yes) return
    setBusy(true)
    setError('')
    try {
      await ok(
        api.DELETE('/api/rooms/{room}', {
          params: { path: { room: room.id } },
        }),
      )
      await client.invalidateQueries({ queryKey: keys.rooms })
      await navigate({ to: '/' })
    } catch (caught) {
      setError(failure(caught))
      setBusy(false)
    }
  }
  return (
    <>
      <button
        type="button"
        className="btn btn--ghost btn--sm"
        aria-label="Delete room"
        disabled={busy}
        onClick={() => void remove()}
      >
        <Trash2 />
        <span className="agent-head__word">
          {busy ? 'Deleting…' : 'Delete'}
        </span>
      </button>
      {error && <Alert>{error}</Alert>}
    </>
  )
}
