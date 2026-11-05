import type { components } from '../../api/schema'
import { Field } from '../../components/primitives'
import { PROVIDER_OPTIONS } from './call'

type Provider = components['schemas']['Provider']
type Voice = components['schemas']['Voice']

const VOICES: [Voice, string][] = [
  ['female', 'Female'],
  ['male', 'Male'],
]

export function VoiceChoice({
  value,
  onChange,
  inherit,
}: {
  value: Voice | null
  onChange: (voice: Voice | null) => void
  inherit?: string
}) {
  const options: [Voice | null, string][] = inherit
    ? [[null, inherit], ...VOICES]
    : VOICES
  return (
    <fieldset className="voice-choice">
      <legend>Voice</legend>
      <div className="voice-choice__options">
        {options.map(([voice, label]) => (
          <label key={label}>
            <input
              type="radio"
              name="voice"
              checked={value === voice}
              onChange={() => onChange(voice)}
            />
            <span>{label}</span>
          </label>
        ))}
      </div>
    </fieldset>
  )
}

export function ProviderField({
  label,
  value,
  onChange,
  inherit,
}: {
  label: string
  value: Provider | null
  onChange: (provider: Provider | null) => void
  inherit?: string
}) {
  return (
    <Field label={label}>
      <select
        value={value ?? ''}
        onChange={(event) =>
          onChange((event.target.value || null) as Provider | null)
        }
      >
        {inherit && <option value="">{inherit}</option>}
        {PROVIDER_OPTIONS.map(([provider, name]) => (
          <option key={provider} value={provider}>
            {name}
          </option>
        ))}
      </select>
    </Field>
  )
}
