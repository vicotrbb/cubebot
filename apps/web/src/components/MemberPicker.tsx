import { Search, UserPlus, X } from 'lucide-react'
import {
  useEffect,
  useId,
  useMemo,
  useRef,
  useState,
  type KeyboardEvent,
  type ReactNode,
} from 'react'
import { Cube, moodOf } from './Cube'
import { matching, memberKey, type MemberOption } from './members'
import { PersonAvatar } from './PersonAvatar'
import './member-picker.css'

export function MemberAvatar({ member }: { member: MemberOption }) {
  if (member.kind === 'agent')
    return (
      <Cube
        avatar={member.agent?.avatar ?? null}
        mood={member.agent ? moodOf(member.agent) : 'idle'}
        size={22}
      />
    )
  return (
    <PersonAvatar
      account={member.id}
      name={member.name}
      className="member__photo"
    />
  )
}

export function MemberRow({
  member,
  detail,
  onRemove,
  busy = false,
  children,
}: {
  member: MemberOption
  detail?: ReactNode
  onRemove?: (() => void) | undefined
  busy?: boolean
  children?: ReactNode
}) {
  return (
    <li className="member">
      <MemberAvatar member={member} />
      <span className="member__who">
        <span className="member__name">{member.name}</span>
        <span className="member__kind">
          {detail ?? (member.kind === 'agent' ? 'Agent' : 'Person')}
        </span>
      </span>
      {children}
      {onRemove && (
        <button
          type="button"
          className="btn btn--ghost btn--icon btn--sm member__remove"
          aria-label={`Remove ${member.name}`}
          title="Remove"
          disabled={busy}
          onClick={onRemove}
        >
          <X />
        </button>
      )}
    </li>
  )
}

export function MemberList({
  members,
  removable,
  onRemove,
  busy,
  empty,
}: {
  members: MemberOption[]
  removable: (member: MemberOption) => boolean
  onRemove: (member: MemberOption) => void
  busy: boolean
  empty: string
}) {
  if (members.length === 0) return <p className="members__empty">{empty}</p>
  return (
    <ul className="members">
      {members.map((member) => (
        <MemberRow
          key={memberKey(member)}
          member={member}
          busy={busy}
          onRemove={removable(member) ? () => onRemove(member) : undefined}
        />
      ))}
    </ul>
  )
}

export function AddMember({
  options,
  onAdd,
  busy = false,
  placeholder = 'Search agents and people',
}: {
  options: MemberOption[]
  onAdd: (member: MemberOption) => void
  busy?: boolean
  placeholder?: string
}) {
  const [open, setOpen] = useState(false)
  const [query, setQuery] = useState('')
  const [active, setActive] = useState(0)
  const root = useRef<HTMLDivElement>(null)
  const input = useRef<HTMLInputElement>(null)
  const opener = useRef<HTMLButtonElement>(null)
  const id = useId()
  const found = useMemo(() => matching(options, query), [options, query])
  const at = Math.min(active, Math.max(0, found.length - 1))

  useEffect(() => {
    if (!open) return
    input.current?.focus()
    const away = (event: PointerEvent) => {
      if (!root.current?.contains(event.target as Node)) setOpen(false)
    }
    document.addEventListener('pointerdown', away)
    return () => document.removeEventListener('pointerdown', away)
  }, [open])

  const close = () => {
    setOpen(false)
    setQuery('')
    setActive(0)
    requestAnimationFrame(() => opener.current?.focus())
  }
  const pick = (member: MemberOption | undefined) => {
    if (!member) return
    onAdd(member)
    setQuery('')
    setActive(0)
    input.current?.focus()
  }
  const keys: Record<string, () => void> = {
    ArrowDown: () => setActive(Math.min(found.length - 1, at + 1)),
    ArrowUp: () => setActive(Math.max(0, at - 1)),
    Home: () => setActive(0),
    End: () => setActive(found.length - 1),
    Enter: () => pick(found[at]),
    Escape: close,
  }
  const steer = (event: KeyboardEvent<HTMLInputElement>) => {
    const action = keys[event.key]
    if (!action) return
    event.preventDefault()
    action()
  }

  if (!open)
    return (
      <button
        ref={opener}
        type="button"
        className="btn btn--sm member-add__open"
        disabled={busy || options.length === 0}
        title={options.length === 0 ? 'Everyone is already here' : undefined}
        onClick={() => setOpen(true)}
      >
        <UserPlus /> Add member
      </button>
    )
  return (
    <div className="member-add" ref={root}>
      <div className="member-add__search">
        <Search aria-hidden="true" />
        <input
          ref={input}
          role="combobox"
          aria-expanded="true"
          aria-controls={`${id}-list`}
          aria-autocomplete="list"
          aria-activedescendant={found[at] ? `${id}-${at}` : undefined}
          aria-label="Add a member"
          placeholder={placeholder}
          value={query}
          onChange={(event) => {
            setQuery(event.target.value)
            setActive(0)
          }}
          onKeyDown={steer}
        />
        <button
          type="button"
          className="btn btn--ghost btn--icon btn--sm"
          aria-label="Close"
          onClick={close}
        >
          <X />
        </button>
      </div>
      <ul
        id={`${id}-list`}
        role="listbox"
        aria-label="Agents and people"
        className="member-add__list"
      >
        {found.map((member, index) => (
          <li
            key={memberKey(member)}
            id={`${id}-${index}`}
            role="option"
            aria-selected={index === at}
            className="member-add__option"
            onPointerDown={(event) => event.preventDefault()}
            onPointerMove={() => setActive(index)}
            onClick={() => pick(member)}
          >
            <MemberAvatar member={member} />
            <span className="member__name">{member.name}</span>
            <span className="member__kind">
              {member.kind === 'agent' ? 'Agent' : 'Person'}
            </span>
          </li>
        ))}
        {found.length === 0 && (
          <li className="member-add__none" role="presentation">
            {options.length === 0
              ? 'Everyone is already here.'
              : `No agent or person matches “${query}”.`}
          </li>
        )}
      </ul>
    </div>
  )
}
