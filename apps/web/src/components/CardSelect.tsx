import { Check, ChevronDown } from 'lucide-react'
import {
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  useState,
  type CSSProperties,
  type KeyboardEvent,
  type ReactNode,
  type RefObject,
} from 'react'
import './card-select.css'

export type CardOption<T extends string> = {
  value: T
  title: string
  detail: string
  mark: ReactNode
}

const GAP = 6
const EDGE = 8
const TALLEST = 360
const ROOMY = 220

type Place = { style: CSSProperties; side: 'below' | 'above' }

function placeBeside(trigger: HTMLElement, list: HTMLElement): Place {
  const box = trigger.getBoundingClientRect()
  const wanted = Math.min(TALLEST, list.scrollHeight)
  const below = window.innerHeight - box.bottom - GAP - EDGE
  const above = box.top - GAP - EDGE
  const side =
    below >= Math.min(wanted, ROOMY) || below >= above ? 'below' : 'above'
  const room = side === 'below' ? below : above
  const style: CSSProperties = {
    left: box.left,
    width: box.width,
    maxHeight: Math.max(120, Math.min(TALLEST, room)),
  }
  if (side === 'below') style.top = box.bottom + GAP
  else style.bottom = window.innerHeight - box.top + GAP
  return { style, side }
}

function useAnchored(
  open: boolean,
  trigger: RefObject<HTMLElement | null>,
  list: RefObject<HTMLElement | null>,
) {
  const [place, setPlace] = useState<Place | null>(null)
  useLayoutEffect(() => {
    const element = list.current
    if (!open || !element || !trigger.current) return
    element.showPopover()
    const follow = (event?: Event) => {
      if (event?.target === element || !trigger.current) return
      setPlace(placeBeside(trigger.current, element))
    }
    follow()
    element.focus()
    window.addEventListener('resize', follow)
    window.addEventListener('scroll', follow, true)
    return () => {
      window.removeEventListener('resize', follow)
      window.removeEventListener('scroll', follow, true)
      setPlace(null)
    }
  }, [open, trigger, list])
  return place
}

function useTypeahead<T extends string>(options: CardOption<T>[]) {
  const typed = useRef({ text: '', at: 0 })
  return (key: string, from: number) => {
    const now = Date.now()
    const text = (now - typed.current.at > 700 ? '' : typed.current.text) + key
    typed.current = { text: text.toLowerCase(), at: now }
    const order = options.map((_, index) => (from + 1 + index) % options.length)
    const sought = typed.current.text
    const exact =
      sought.length > 1 && options[from]?.title.toLowerCase().startsWith(sought)
    if (exact) return from
    return order.find((index) =>
      options[index]?.title.toLowerCase().startsWith(sought),
    )
  }
}

/** A select whose closed state and choices are cards: a button and a listbox (WAI-ARIA APG). */
export function CardSelect<T extends string>({
  label,
  options,
  value,
  onChange,
}: {
  label: string
  options: CardOption<T>[]
  value: T
  onChange: (value: T) => void
}) {
  const [open, setOpen] = useState(false)
  const [active, setActive] = useState(0)
  const root = useRef<HTMLDivElement>(null)
  const list = useRef<HTMLUListElement>(null)
  const button = useRef<HTMLButtonElement>(null)
  const id = useId()
  const current = options.find((option) => option.value === value) ?? options[0]
  const place = useAnchored(open, button, list)
  const seek = useTypeahead(options)

  useEffect(() => {
    if (!open) return
    const away = (event: PointerEvent) => {
      if (!root.current?.contains(event.target as Node)) setOpen(false)
    }
    document.addEventListener('pointerdown', away)
    return () => document.removeEventListener('pointerdown', away)
  }, [open])

  useEffect(() => {
    if (!open) return
    document
      .getElementById(`${id}-${active}`)
      ?.scrollIntoView({ block: 'nearest' })
  }, [open, active, id])

  const show = () => {
    setActive(Math.max(0, options.indexOf(current as CardOption<T>)))
    setOpen(true)
  }
  const close = () => {
    setOpen(false)
    button.current?.focus()
  }
  const pick = (index: number) => {
    const option = options[index]
    if (option) onChange(option.value)
    close()
  }
  const keys: Record<string, () => void> = {
    ArrowDown: () => setActive((at) => Math.min(options.length - 1, at + 1)),
    ArrowUp: () => setActive((at) => Math.max(0, at - 1)),
    Home: () => setActive(0),
    End: () => setActive(options.length - 1),
    Enter: () => pick(active),
    ' ': () => pick(active),
    Escape: close,
    Tab: () => setOpen(false),
  }
  const press = (event: KeyboardEvent) => {
    const act = keys[event.key]
    if (act) {
      if (event.key !== 'Tab') event.preventDefault()
      act()
      return
    }
    if (event.key.length !== 1 || event.metaKey || event.ctrlKey) return
    const found = seek(event.key, active)
    if (found !== undefined) setActive(found)
  }

  return (
    <div className="card-select" ref={root}>
      <button
        ref={button}
        type="button"
        className="card-select__button"
        aria-haspopup="listbox"
        aria-expanded={open}
        aria-controls={`${id}-list`}
        aria-label={`${label}: ${current?.title ?? ''}`}
        onClick={() => (open ? setOpen(false) : show())}
        onKeyDown={(event) => {
          if (event.key !== 'ArrowDown' && event.key !== 'ArrowUp') return
          event.preventDefault()
          show()
        }}
      >
        {current && <Card option={current} />}
        <ChevronDown className="card-select__chevron" aria-hidden="true" />
      </button>
      {open && (
        <ul
          ref={list}
          id={`${id}-list`}
          role="listbox"
          popover="manual"
          tabIndex={-1}
          aria-label={label}
          aria-activedescendant={`${id}-${active}`}
          className="card-select__list"
          data-side={place?.side}
          style={place?.style ?? { opacity: 0 }}
          onKeyDown={press}
        >
          {options.map((option, index) => (
            <li
              key={option.value}
              id={`${id}-${index}`}
              role="option"
              aria-selected={option.value === value}
              className={
                index === active
                  ? 'card-select__option card-select__option--active'
                  : 'card-select__option'
              }
              onPointerMove={() => setActive(index)}
              onClick={() => pick(index)}
            >
              <Card option={option} />
              {option.value === value && (
                <Check className="card-select__check" aria-hidden="true" />
              )}
            </li>
          ))}
        </ul>
      )}
    </div>
  )
}

function Card<T extends string>({ option }: { option: CardOption<T> }) {
  return (
    <span className="card-select__card">
      <span className="card-select__mark" aria-hidden="true">
        {option.mark}
      </span>
      <span className="card-select__text">
        <span className="card-select__title">{option.title}</span>
        <span className="card-select__detail">{option.detail}</span>
      </span>
    </span>
  )
}
