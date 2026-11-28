import { useState } from 'react'

/** A one-time code from an authenticator app, or a recovery code in its place. */
export function CodeField({
  name = 'code',
  autoFocus = false,
  disabled = false,
}: {
  name?: string
  autoFocus?: boolean
  disabled?: boolean
}) {
  const [recovery, setRecovery] = useState(false)
  const id = `${name}-field`
  return (
    <div className="field">
      <span>
        <label htmlFor={id}>
          {recovery ? 'Recovery code' : 'Code from your authenticator app'}
        </label>
      </span>
      <input
        key={String(recovery)}
        id={id}
        name={name}
        required
        disabled={disabled}
        autoFocus={autoFocus || recovery}
        inputMode={recovery ? 'text' : 'numeric'}
        autoComplete={recovery ? 'off' : 'one-time-code'}
        autoCapitalize="none"
        spellCheck={false}
        maxLength={recovery ? 16 : 8}
        placeholder={recovery ? 'ab12c-3de45' : '123456'}
      />
      <button
        type="button"
        className="link-btn small code-field__switch"
        onClick={() => setRecovery(!recovery)}
      >
        {recovery ? 'Use your authenticator app' : 'Use a recovery code'}
      </button>
    </div>
  )
}

/** What someone typed, without the spaces some apps show inside a code. */
export function codeOf(form: FormData, name = 'code') {
  return String(form.get(name) ?? '').replace(/\s+/g, '')
}
