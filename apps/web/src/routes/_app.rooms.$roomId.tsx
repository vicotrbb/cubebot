import { useQueryClient } from '@tanstack/react-query'
import {
  createFileRoute,
  Link,
  Outlet,
  useChildMatches,
} from '@tanstack/react-router'
import { ChevronLeft, Hash, Users, UsersRound, X } from 'lucide-react'
import { type AgentSummary, type RoomSummary } from '../api/types'
import {
  keys,
  useAgents,
  useRooms,
  useSession,
  useStanding,
} from '../app/queries'
import { Empty, Skeleton } from '../components/primitives'
import { Sharing, useResourceAccess } from '../features/agent/Access'
import { DeleteRoom } from '../features/rooms/DeleteRoom'
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
        <RoomTitle room={room} />
        <div className="agent-head__actions">
          <FavoriteButton kind="room" id={room.id} />
          {!room.team && !room.archived && (
            <DeleteWhenOwned
              room={room}
              me={session.data.id}
              administrator={session.data.administrator}
            />
          )}
          {!panelOpen && (
            <Link
              to="/rooms/$roomId/members"
              params={{ roomId: room.id }}
              className="btn btn--sm"
              aria-label={
                editable(room, session.data.administrator)
                  ? 'Edit members'
                  : 'Members'
              }
            >
              <Users />
              <span className="agent-head__word">
                {editable(room, session.data.administrator)
                  ? 'Edit members'
                  : 'Members'}
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
                administrator={session.data.administrator}
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

/** Members delete rooms they own; administrators any. */
function DeleteWhenOwned({
  room,
  me,
  administrator,
}: {
  room: RoomSummary
  me: string
  administrator: boolean
}) {
  const access = useResourceAccess(room.id, room.manage && !administrator)
  if (!administrator && access.data?.owner !== me) return null
  return <DeleteRoom room={room} />
}

function RoomMembersPanel({
  room,
  agents,
  administrator,
  onSaved,
}: {
  room: RoomSummary
  agents: AgentSummary[]
  administrator: boolean
  onSaved: (room: RoomSummary) => void
}) {
  const solo = useStanding().data?.solo ?? true
  return (
    <>
      <section className="panel__section">
        <h2>Members</h2>
        <Members
          room={room}
          agents={agents}
          administrator={administrator}
          onSaved={onSaved}
        />
      </section>
      {!solo && !room.archived && (administrator || room.manage) && (
        <Sharing
          target={{ kind: 'room', id: room.id }}
          name={`#${room.name}`}
          held={ROOM_HELD}
          editable
          transferable={!room.team}
        />
      )}
    </>
  )
}

/** The manage level and ownership both carry reading and posting; the server refuses anything more. */
const ROOM_HELD = ['conversation_read', 'conversation_send', 'room_manage']

function editable(room: RoomSummary, administrator: boolean) {
  return !room.archived && (administrator || room.manage)
}

function RoomTitle({ room }: { room: RoomSummary }) {
  return (
    <>
      {room.team || room.archived ? (
        <UsersRound className="row__icon" />
      ) : (
        <Hash className="row__icon" />
      )}
      <div className="agent-head__title">
        <h1>{room.name}</h1>
        <p className="faint small">
          {room.team && (
            <>
              <Link to="/teams/$teamId" params={{ teamId: room.team }}>
                Team room
              </Link>
              {' · '}
            </>
          )}
          <span className="agent-head__members">
            {room.archived
              ? 'Deleted; administrators can still read it'
              : room.members.map((member) => member.name).join(', ') ||
                'No agents yet'}
          </span>
        </p>
      </div>
    </>
  )
}
