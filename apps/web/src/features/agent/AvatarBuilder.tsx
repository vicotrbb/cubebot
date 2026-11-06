import { useRef } from 'react'
import {
  ACCESSORIES,
  Cube,
  EYES,
  Face,
  PAINTS,
  type Accessory,
  type Avatar,
  type Eyes,
  type Mood,
  type Paint,
} from '../../components/Cube'

const named = (value: string) =>
  value.charAt(0).toUpperCase() + value.slice(1).replace('_', ' ')

export function AvatarBuilder({
  value,
  mood = 'idle',
  onChange,
}: {
  value: Avatar
  mood?: Mood
  onChange: (next: Avatar) => void
}) {
  return (
    <div className="avatar-builder">
      <div className="avatar-builder__preview">
        <Cube avatar={value} mood={mood} size={64} follow />
      </div>
      <Choice
        area="paint"
        label="Paint"
        current={value.color ? 'Your color' : named(value.paint)}
        options={PAINTS}
        value={value.color ? null : value.paint}
        onPick={(paint: Paint) => onChange({ ...value, paint, color: null })}
        render={(paint) => (
          <span
            className="avatar-builder__swatch"
            style={{ background: `var(--${paint})` }}
          />
        )}
        after={
          <Picker
            color={value.color ?? null}
            onPick={(color) => onChange({ ...value, color })}
          />
        }
      />
      <Choice
        area="eyes"
        label="Eyes"
        current={named(value.eyes)}
        options={EYES}
        value={value.eyes}
        onPick={(eyes: Eyes) => onChange({ ...value, eyes })}
        render={(eyes) => <Face look={{ ...value, eyes }} part="eyes" />}
      />
      <Choice
        area="wear"
        label="Accessory"
        current={named(value.accessory)}
        options={ACCESSORIES}
        value={value.accessory}
        onPick={(accessory: Accessory) => onChange({ ...value, accessory })}
        render={(accessory) => (
          <Face look={{ ...value, accessory }} part="accessory" />
        )}
      />
    </div>
  )
}

/** The rainbow block: any color through the platform's own picker, previewed live. */
function Picker({
  color,
  onPick,
}: {
  color: string | null
  onPick: (color: string) => void
}) {
  const input = useRef<HTMLInputElement>(null)
  return (
    <span className="avatar-builder__custom">
      <button
        type="button"
        role="radio"
        aria-checked={color !== null}
        aria-label="Pick any color"
        title="Pick any color"
        className="avatar-builder__opt avatar-builder__rainbow"
        onClick={() => {
          const field = input.current
          if (!field) return
          try {
            field.showPicker()
          } catch {
            field.click()
          }
        }}
      >
        <span
          className="avatar-builder__swatch"
          style={(color ? { '--pick': color } : {}) as React.CSSProperties}
        />
        {color ?? 'Any color'}
      </button>
      <input
        ref={input}
        type="color"
        tabIndex={-1}
        aria-hidden="true"
        value={color ?? '#3fa7a0'}
        onInput={(event) => onPick(event.currentTarget.value)}
        onChange={(event) => onPick(event.currentTarget.value)}
      />
    </span>
  )
}

function Choice<T extends string>({
  area,
  label,
  current,
  options,
  value,
  onPick,
  render,
  after,
}: {
  area: string
  label: string
  current: string
  options: readonly T[]
  value: T | null
  onPick: (value: T) => void
  render: (value: T) => React.ReactNode
  after?: React.ReactNode
}) {
  return (
    <div
      className={`avatar-builder__group avatar-builder__group--${area}`}
      role="radiogroup"
      aria-label={label}
    >
      <span className="avatar-builder__label">
        {label} <span className="faint">{current}</span>
      </span>
      <div className="avatar-builder__row">
        {options.map((option) => (
          <button
            key={option}
            type="button"
            role="radio"
            aria-checked={option === value}
            aria-label={named(option)}
            title={named(option)}
            className="avatar-builder__opt"
            onClick={() => onPick(option)}
          >
            {render(option)}
          </button>
        ))}
        {after}
      </div>
    </div>
  )
}
