import { useQueryClient } from '@tanstack/react-query'
import {
  createFileRoute,
  Link,
  Outlet,
  useChildMatches,
} from '@tanstack/react-router'
import { ChevronLeft, Hash, Users, X } from 'lucide-react'
import { type RoomSummary } from '../api/types'
import { keys, useAgents, useRooms, useSession } from '../app/queries'
import { Empty, Skeleton } from '../components/primitives'
import { Members } from '../features/rooms/Members'
import { Room } from '../features/rooms/Room'
import { FavoriteButton } from '../features/shell/Sidebar'
import { useIsPhone } from '../features/shell/viewport'
import '../features/agent/agent.css'

export const Route = createFileRoute('/_app/rooms/$roomId')({
  component: RoomScreen,
})

function RoomScreen() {
  const { roomId } = Route.useParams()
  const session = useSession()
  const rooms = useRooms()
  const agents = useAgents()
  const client = useQueryClient()
  const phone = useIsPhone()
  const panelOpen = useChildMatches().length > 0
  const room = rooms.data?.find((item) => item.id === roomId)
  if (rooms.isPending || !session.data)
    return (
      <div className="page__body">
        <Skeleton lines={2} width="40%" />
      </div>
    )
  if (!room)
    return (
      <Empty title="This room is unavailable.">
        You may no longer have access to it.
      </Empty>
    )
  return (
    <div className="agent-screen" data-panel={panelOpen}>
      <header className="agent-head">
        {phone && (
          <Link
            to="/"
            className="btn btn--ghost btn--icon"
            aria-label="Back to home"
          >
            <ChevronLeft />
          </Link>
        )}
        <Hash className="row__icon" />
        <div className="agent-head__title">
          <h1>{room.name}</h1>
          <p className="faint small">
            {room.members.map((member) => member.name).join(', ') ||
              'No members'}
          </p>
        </div>
        <div className="agent-head__actions">
          <FavoriteButton kind="room" id={room.id} />
          {!panelOpen && (
            <Link
              to="/rooms/$roomId/members"
              params={{ roomId: room.id }}
              className="btn btn--sm"
            >
              <Users />
              <span className="agent-head__word">
                {room.manage ? 'Edit members' : 'Members'}
              </span>
            </Link>
          )}
        </div>
      </header>
      <div className="agent-split">
        <Room key={room.id} room={room} session={session.data} />
        {panelOpen && (
          <aside className="panel" aria-label="Room details">
            <div className="panel__head">
              <nav className="tabs panel__tabs" aria-label="Room details">
                <Link
                  to="/rooms/$roomId/members"
                  params={{ roomId: room.id }}
                  activeProps={{ 'aria-current': 'page' }}
                >
                  Members
                </Link>
              </nav>
              <Link
                to="/rooms/$roomId"
                params={{ roomId: room.id }}
                className="btn btn--ghost btn--icon btn--sm"
                aria-label="Close members"
              >
                <X />
              </Link>
            </div>
            <div className="panel__body">
              <Outlet />
              <RoomMembersPanel
                room={room}
                agents={agents.data ?? []}
                onSaved={(changed) =>
                  client.setQueryData<RoomSummary[]>(keys.rooms, (current) =>
                    current?.map((known) =>
                      known.id === changed.id
                        ? { ...known, ...changed }
                        : known,
                    ),
                  )
                }
              />
            </div>
          </aside>
        )}
      </div>
    </div>
  )
}

function RoomMembersPanel({
  room,
  agents,
  onSaved,
}: {
  room: RoomSummary
  agents: Parameters<typeof Members>[0]['agents']
  onSaved: (room: RoomSummary) => void
}) {
  return (
    <section className="panel__section">
      <h2>Members</h2>
      {room.manage ? (
        <Members
          key={room.members.map((m) => m.id).join()}
          room={room}
          agents={agents}
          onSaved={onSaved}
        />
      ) : (
        <ul className="list">
          {room.members.map((member) => (
            <li key={member.id}>
              <Link to="/agents/$agentId" params={{ agentId: member.id }}>
                {member.name}
              </Link>
            </li>
          ))}
        </ul>
      )}
    </section>
  )
}
