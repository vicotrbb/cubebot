import {
  useLayoutEffect,
  useRef,
  useState,
  type KeyboardEvent,
  type RefObject,
} from 'react'
import { Cube, type Avatar } from '../../components/Cube'
import { PersonAvatar } from '../../components/PersonAvatar'
import { complete, mentionAt, suggest, type MentionQuery } from './mentions'

export type Mentionable = {
  id: string
  name: string
  avatar?: Avatar | null
  person?: boolean
}

export function useMentions(
  people: Mentionable[] | undefined,
  area: RefObject<HTMLTextAreaElement | null>,
  text: string,
  edit: (value: string) => void,
  id: string,
) {
  const [mention, setMention] = useState<MentionQuery | null>(null)
  const [active, setActive] = useState(0)
  const options = mention && people ? suggest(people, mention.query) : []
  const listId = `mentions-${id}`
  const caret = useRef<number | null>(null)
  useLayoutEffect(() => {
    const element = area.current
    if (caret.current === null || !element) return
    element.focus()
    element.setSelectionRange(caret.current, caret.current)
    caret.current = null
  }, [area, text])
  function track(element: HTMLTextAreaElement) {
    if (!people) return
    const found = mentionAt(element.value, element.selectionStart)
    if (found?.start !== mention?.start || found?.query !== mention?.query)
      setActive(0)
    setMention(found)
  }
  function pick(person: Mentionable) {
    const element = area.current
    if (!element || !mention) return
    const done = complete(text, element.selectionStart, mention, person.name)
    caret.current = done.caret
    edit(done.text)
    setMention(null)
  }
  function steer(event: KeyboardEvent<HTMLTextAreaElement>) {
    const chosen = options[active] ?? options[0]
    if (!chosen || event.nativeEvent.isComposing) return false
    const step = { ArrowDown: 1, ArrowUp: -1 }[event.key]
    if (step) setActive((active + step + options.length) % options.length)
    else if (event.key === 'Enter' || event.key === 'Tab') pick(chosen)
    else if (event.key === 'Escape') setMention(null)
    else return false
    event.preventDefault()
    return true
  }
  const comboBox = people && {
    role: 'combobox',
    'aria-autocomplete': 'list' as const,
    'aria-expanded': options.length > 0,
    'aria-controls': listId,
    'aria-activedescendant':
      options.length > 0 ? `${listId}-${active}` : undefined,
  }
  const menu = options.length > 0 && (
    <ul
      id={listId}
      role="listbox"
      className="mention-menu"
      aria-label="Members to mention"
    >
      {options.map((person, index) => (
        <li
          key={person.id}
          id={`${listId}-${index}`}
          role="option"
          aria-selected={index === active}
          className="mention-menu__item"
          // Keeps focus in the textarea, so the caret survives the pick.
          onMouseDown={(event) => event.preventDefault()}
          onMouseEnter={() => setActive(index)}
          onClick={() => pick(person)}
        >
          {person.person ? (
            <PersonAvatar
              account={person.id}
              name={person.name}
              className="mention-menu__person"
            />
          ) : (
            <Cube avatar={person.avatar ?? null} size={22} />
          )}
          <span className="mention-menu__name">{person.name}</span>
        </li>
      ))}
    </ul>
  )
  return {
    track,
    steer,
    close: () => setMention(null),
    comboBox,
    menu,
  }
}
