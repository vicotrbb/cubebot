import {
  ACCESSORIES,
  Cube,
  EYES,
  PAINTS,
  type Accessory,
  type Avatar,
  type Eyes,
  type Mood,
  type Paint,
} from '../../components/Cube'

/** Picks the block's paint, eyes and one accessory; the live block previews the choice. */
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
        <Cube avatar={value} mood={mood} size={76} follow />
      </div>
      <div className="avatar-builder__choices">
        <Choice
          label="Paint"
          options={PAINTS}
          value={value.paint}
          onPick={(paint: Paint) => onChange({ ...value, paint })}
          render={(paint) => (
            <span
              className={`swatch cube--${paint}`}
              style={{ background: `var(--${paint})` }}
            />
          )}
        />
        <Choice
          label="Eyes"
          options={EYES}
          value={value.eyes}
          onPick={(eyes: Eyes) => onChange({ ...value, eyes })}
          render={(eyes) => <span className="small">{eyes}</span>}
        />
        <Choice
          label="Accessory"
          options={ACCESSORIES}
          value={value.accessory}
          onPick={(accessory: Accessory) => onChange({ ...value, accessory })}
          render={(accessory) => <span className="small">{accessory}</span>}
        />
      </div>
    </div>
  )
}

function Choice<T extends string>({
  label,
  options,
  value,
  onPick,
  render,
}: {
  label: string
  options: readonly T[]
  value: T
  onPick: (value: T) => void
  render: (value: T) => React.ReactNode
}) {
  return (
    <div className="field" role="radiogroup" aria-label={label}>
      <span>{label}</span>
      <div className="avatar-builder__row">
        {options.map((option) => (
          <button
            key={option}
            type="button"
            role="radio"
            aria-checked={option === value}
            aria-label={option}
            className="avatar-builder__opt"
            onClick={() => onPick(option)}
          >
            {render(option)}
          </button>
        ))}
      </div>
    </div>
  )
}
