import { X } from 'lucide-react'
import { useEffect, useRef, type ReactNode } from 'react'

export function Skeleton({
  lines = 3,
  width = '100%',
}: {
  lines?: number
  width?: string
}) {
  return (
    <div className="skeleton-group" aria-hidden="true" style={{ width }}>
      {Array.from({ length: lines }, (_, index) => (
        <span
          key={index}
          className="skeleton"
          style={{
            width: `${100 - (index % 3) * 18}%`,
            marginBottom: 10,
          }}
        />
      ))}
    </div>
  )
}

export function Empty({
  title,
  children,
  action,
}: {
  title: string
  children?: ReactNode
  action?: ReactNode
}) {
  return (
    <div className="empty">
      <h3>{title}</h3>
      {children && <p>{children}</p>}
      {action}
    </div>
  )
}

export function Dialog({
  open,
  title,
  onClose,
  children,
  label,
}: {
  open: boolean
  title: string
  onClose: () => void
  children: ReactNode
  label?: string
}) {
  const ref = useRef<HTMLDialogElement>(null)
  useEffect(() => {
    const dialog = ref.current
    if (!dialog) return
    if (open && !dialog.open) dialog.showModal()
    if (!open && dialog.open) dialog.close()
  }, [open])
  return (
    <dialog
      ref={ref}
      className="dialog"
      aria-label={label ?? title}
      onClose={onClose}
      onClick={(event) => {
        if (event.target === event.currentTarget) onClose()
      }}
    >
      <div className="dialog__body">
        <div className="dialog__head">
          <h2>{title}</h2>
          <button
            type="button"
            className="btn btn--ghost btn--icon"
            aria-label="Close"
            onClick={onClose}
          >
            <X />
          </button>
        </div>
        {open && children}
      </div>
    </dialog>
  )
}

/** A `details` menu that closes when an item is chosen, on a click outside it and on Escape,
 *  as well as from its trigger. */
export function Menu({
  className,
  trigger,
  triggerClassName,
  label,
  children,
}: {
  className?: string
  trigger: ReactNode
  triggerClassName: string
  label?: string
  children: ReactNode
}) {
  const ref = useRef<HTMLDetailsElement>(null)
  const detach = useRef<(() => void) | null>(null)
  // Listens from the opening click, not from `toggle`: browsers coalesce toggle events, so a
  // quick close and reopen can skip one. Each handler reads the element's own state.
  const watch = () => {
    const menu = ref.current
    if (!menu || detach.current) return
    const outside = (event: PointerEvent) => {
      if (menu.open && !menu.contains(event.target as Node)) close()
    }
    const escape = (event: KeyboardEvent) => {
      if (event.key !== 'Escape' || !menu.open) return
      close()
      menu.querySelector('summary')?.focus()
    }
    document.addEventListener('pointerdown', outside)
    document.addEventListener('keydown', escape)
    detach.current = () => {
      document.removeEventListener('pointerdown', outside)
      document.removeEventListener('keydown', escape)
      detach.current = null
    }
  }
  const close = () => {
    if (ref.current) ref.current.open = false
    detach.current?.()
  }
  useEffect(() => () => detach.current?.(), [])
  return (
    <details
      ref={ref}
      className={className ? `menu ${className}` : 'menu'}
      onToggle={(event) => {
        if (!event.currentTarget.open) detach.current?.()
      }}
    >
      <summary
        className={triggerClassName}
        aria-label={label}
        onClick={() => {
          if (!ref.current?.open) watch()
        }}
      >
        {trigger}
      </summary>
      <div
        className="menu__list"
        onClick={(event) => {
          if ((event.target as Element).closest('a, button')) close()
        }}
      >
        {children}
      </div>
    </details>
  )
}

export function Field({
  label,
  hint,
  children,
}: {
  label: string
  hint?: string | undefined
  children: ReactNode
}) {
  return (
    <label className="field">
      <span>
        {label}
        {hint && <span className="hint">{hint}</span>}
      </span>
      {children}
    </label>
  )
}

export function Alert({ children }: { children: ReactNode }) {
  if (!children) return null
  return (
    <p role="alert" className="notice notice--danger">
      {children}
    </p>
  )
}

export function Status({ children }: { children: ReactNode }) {
  if (!children) return null
  return (
    <p role="status" className="notice">
      {children}
    </p>
  )
}

export function when(value: string | null | undefined) {
  if (!value) return ''
  const date = new Date(value)
  const now = Date.now()
  const diff = (now - date.getTime()) / 1000
  if (diff < 60) return 'just now'
  if (diff < 3600) return `${Math.floor(diff / 60)} min ago`
  const sameDay = new Date(now).toDateString() === date.toDateString()
  const time = date.toLocaleTimeString(undefined, {
    hour: '2-digit',
    minute: '2-digit',
  })
  if (sameDay) return time
  return `${date.toLocaleDateString(undefined, { month: 'short', day: 'numeric' })} ${time}`
}
