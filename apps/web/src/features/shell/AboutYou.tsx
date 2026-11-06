import { useQuery, useQueryClient } from '@tanstack/react-query'
import { useMemo, useState } from 'react'
import { api, failure, ok } from '../../api/client'
import type { components } from '../../api/schema'
import { Alert, Field, Skeleton, Status } from '../../components/primitives'

type Profile = components['schemas']['Profile']

const ABOUT_LIMIT = 4000
const PROFILE_KEY = ['profile']

export function useProfile() {
  return useQuery({
    queryKey: PROFILE_KEY,
    queryFn: () => ok(api.GET('/api/identity/me/profile')),
  })
}

/** The profile agents read: who you are, when you are, and how you like to work. */
export function AboutYou() {
  const profile = useProfile()
  return (
    <section className="profile__about" aria-labelledby="about-you">
      <div>
        <h3 id="about-you">For your agents</h3>
        <p className="faint small">
          Agents you can reach read this, and it travels with every message you
          send them. Write it the way you would brief a new teammate.
        </p>
      </div>
      {profile.isError && <Alert>{failure(profile.error)}</Alert>}
      {profile.isPending && <Skeleton lines={4} />}
      {profile.data && <ProfileForm saved={profile.data} />}
    </section>
  )
}

function ProfileForm({ saved }: { saved: Profile }) {
  const client = useQueryClient()
  const [draft, setDraft] = useState<Profile>(saved)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const [notice, setNotice] = useState('')
  const zones = useMemo(() => Intl.supportedValuesOf('timeZone'), [])
  const here = Intl.DateTimeFormat().resolvedOptions().timeZone
  const changed = JSON.stringify(draft) !== JSON.stringify(saved)
  const set =
    (key: keyof Profile) => (event: { target: { value: string } }) => {
      setNotice('')
      setDraft({ ...draft, [key]: event.target.value })
    }
  async function save() {
    setBusy(true)
    setError('')
    setNotice('')
    try {
      const stored = await ok(
        api.PUT('/api/identity/me/profile', { body: draft }),
      )
      client.setQueryData(PROFILE_KEY, stored)
      setDraft(stored)
      setNotice('Saved. Agents see it from your next message.')
    } catch (caught) {
      setError(failure(caught))
    } finally {
      setBusy(false)
    }
  }
  return (
    <form
      className="form profile__form"
      onSubmit={(event) => {
        event.preventDefault()
        void save()
      }}
    >
      <div className="profile__pair">
        <Field label="What to call you">
          <input
            value={draft.name ?? ''}
            maxLength={80}
            autoComplete="name"
            placeholder="Your name"
            onChange={set('name')}
          />
        </Field>
        <Field label="Pronouns">
          <input
            value={draft.pronouns ?? ''}
            maxLength={40}
            placeholder="she/her, he/him, they/them…"
            onChange={set('pronouns')}
          />
        </Field>
      </div>
      <div className="profile__pair">
        <Field label="What you do">
          <input
            value={draft.role ?? ''}
            maxLength={120}
            autoComplete="organization-title"
            placeholder="Platform engineer"
            onChange={set('role')}
          />
        </Field>
        <Field label="Reply in">
          <input
            value={draft.language ?? ''}
            maxLength={40}
            placeholder="English"
            onChange={set('language')}
          />
        </Field>
      </div>
      <div className="profile__zone">
        <Field label="Time zone">
          <select value={draft.time_zone ?? ''} onChange={set('time_zone')}>
            <option value="">Not set</option>
            {zones.map((zone) => (
              <option key={zone} value={zone}>
                {zone.replaceAll('_', ' ')}
              </option>
            ))}
          </select>
        </Field>
        {draft.time_zone !== here && (
          <button
            type="button"
            className="link-btn small"
            onClick={() => set('time_zone')({ target: { value: here } })}
          >
            Use this device’s, {here.replaceAll('_', ' ')}
          </button>
        )}
      </div>
      <Field
        label="What to know about you"
        hint={`${(draft.about ?? '').length} / ${ABOUT_LIMIT}`}
      >
        <textarea
          rows={6}
          maxLength={ABOUT_LIMIT}
          value={draft.about ?? ''}
          placeholder="I run the homelab and care about uptime. Keep replies short and skip the preamble. Ask before spending money. I'm usually offline after 22:00."
          onChange={set('about')}
        />
      </Field>
      <Alert>{error}</Alert>
      <Status>{notice}</Status>
      <div className="actions">
        <button className="btn btn--primary" disabled={busy || !changed}>
          {busy ? 'Saving…' : 'Save profile'}
        </button>
      </div>
    </form>
  )
}
