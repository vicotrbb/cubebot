import { useNavigate } from '@tanstack/react-router'
import { Hash } from 'lucide-react'
import { useEffect, useMemo, useRef, useState } from 'react'
import { useAgents, useRooms, useSession } from '../../app/queries'
import { Cube, moodOf } from '../../components/Cube'
import { PLATFORM } from './Sidebar'

type Hit = {
  key: string
  label: string
  hint: string
  go: () => void
  icon?: React.ReactNode
}

/** ⌘K: jump to an agent, room or page by name. */
export function CommandPalette({
  open,
  onClose,
}: {
  open: boolean
  onClose: () => void
}) {
  const navigate = useNavigate()
  const agents = useAgents()
  const rooms = useRooms()
  const session = useSession()
  const [query, setQuery] = useState('')
  const [cursor, setCursor] = useState(0)
  const input = useRef<HTMLInputElement>(null)
  const dialog = useRef<HTMLDialogElement>(null)
  useEffect(() => {
    const element = dialog.current
    if (!element) return
    if (open && !element.open) {
      element.showModal()
      setQuery('')
      setCursor(0)
      requestAnimationFrame(() => input.current?.focus())
    }
    if (!open && element.open) element.close()
  }, [open])
  const hits = useMemo(() => {
    const go = (to: string) => () => {
      onClose()
      void navigate({ to })
    }
    const all: Hit[] = [
      ...(agents.data ?? []).map((agent) => ({
        key: `a:${agent.id}`,
        label: agent.name,
        hint: 'Agent',
        icon: <Cube avatar={agent.avatar} mood={moodOf(agent)} size={20} />,
        go: go(`/agents/${agent.id}`),
      })),
      ...(rooms.data ?? []).map((room) => ({
        key: `r:${room.id}`,
        label: room.name,
        hint: 'Room',
        icon: <Hash size={16} />,
        go: go(`/rooms/${room.id}`),
      })),
      {
        key: 'p:activity',
        label: 'Activity',
        hint: 'Page',
        go: go('/activity'),
      },
      { key: 'p:files', label: 'Files', hint: 'Page', go: go('/files') },
      { key: 'p:skills', label: 'Skills', hint: 'Page', go: go('/skills') },
      { key: 'p:kb', label: 'Knowledge base', hint: 'Page', go: go('/kb') },
      { key: 'p:you', label: 'You', hint: 'Page', go: go('/you') },
      ...(session.data?.administrator
        ? PLATFORM.map(([to, label]) => ({
            key: `p:${to}`,
            label,
            hint: 'Platform',
            go: go(to),
          }))
        : []),
    ]
    const needle = query.trim().toLowerCase()
    return (
      needle
        ? all.filter((hit) => hit.label.toLowerCase().includes(needle))
        : all
    ).slice(0, 12)
  }, [agents.data, rooms.data, session.data, query, navigate, onClose])
  const active = hits[Math.min(cursor, hits.length - 1)]
  return (
    <dialog
      ref={dialog}
      className="dialog palette"
      aria-label="Search"
      onClose={onClose}
      onClick={(event) => {
        if (event.target === event.currentTarget) onClose()
      }}
    >
      <input
        ref={input}
        className="palette__input"
        placeholder="Jump to an agent, room or page…"
        value={query}
        aria-label="Search"
        onChange={(event) => {
          setQuery(event.target.value)
          setCursor(0)
        }}
        onKeyDown={(event) => {
          if (event.key === 'ArrowDown') {
            event.preventDefault()
            setCursor((c) => Math.min(c + 1, hits.length - 1))
          } else if (event.key === 'ArrowUp') {
            event.preventDefault()
            setCursor((c) => Math.max(c - 1, 0))
          } else if (event.key === 'Enter' && active) {
            event.preventDefault()
            active.go()
          }
        }}
      />
      <ul className="palette__list" role="listbox">
        {hits.map((hit) => (
          <li key={hit.key}>
            <button
              type="button"
              role="option"
              aria-selected={hit === active}
              className="palette__hit"
              onMouseEnter={() => setCursor(hits.indexOf(hit))}
              onClick={hit.go}
            >
              <span className="palette__icon">{hit.icon}</span>
              <span className="row__label">{hit.label}</span>
              <span className="faint small">{hit.hint}</span>
            </button>
          </li>
        ))}
        {hits.length === 0 && (
          <li className="faint palette__none">Nothing matches.</li>
        )}
      </ul>
    </dialog>
  )
}
