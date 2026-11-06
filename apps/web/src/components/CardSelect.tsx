import { Check, ChevronDown } from 'lucide-react'
import { useEffect, useId, useRef, useState, type ReactNode } from 'react'
import './card-select.css'

export type CardOption<T extends string> = {
  value: T
  title: string
  detail: string
  mark: ReactNode
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

  useEffect(() => {
    if (!open) return
    list.current?.focus()
    const away = (event: PointerEvent) => {
      if (!root.current?.contains(event.target as Node)) setOpen(false)
    }
    document.addEventListener('pointerdown', away)
    return () => document.removeEventListener('pointerdown', away)
  }, [open])

  const show = () => {
    setActive(Math.max(0, options.indexOf(current as CardOption<T>)))
    setOpen(true)
  }
  const pick = (index: number) => {
    const option = options[index]
    if (option) onChange(option.value)
    setOpen(false)
    button.current?.focus()
  }
  const keys: Record<string, () => void> = {
    ArrowDown: () => setActive((at) => Math.min(options.length - 1, at + 1)),
    ArrowUp: () => setActive((at) => Math.max(0, at - 1)),
    Home: () => setActive(0),
    End: () => setActive(options.length - 1),
    Enter: () => pick(active),
    ' ': () => pick(active),
    Escape: () => {
      setOpen(false)
      button.current?.focus()
    },
    Tab: () => setOpen(false),
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
          tabIndex={-1}
          aria-label={label}
          aria-activedescendant={`${id}-${active}`}
          className="card-select__list"
          onKeyDown={(event) => {
            const act = keys[event.key]
            if (!act) return
            if (event.key !== 'Tab') event.preventDefault()
            act()
          }}
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
