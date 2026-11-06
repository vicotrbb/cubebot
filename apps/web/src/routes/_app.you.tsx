import { useQueryClient } from '@tanstack/react-query'
import { createFileRoute, useNavigate } from '@tanstack/react-router'
import { ImageUp, LogOut, Trash2 } from 'lucide-react'
import { useRef, useState } from 'react'
import { ApiError, failure } from '../api/client'
import {
  keys,
  signOut,
  useAvatars,
  useInstallation,
  usePreferences,
  useSavePreferences,
  useSession,
} from '../app/queries'
import { applyTheme, type Theme } from '../app/theme'
import { PersonAvatar } from '../components/PersonAvatar'
import { Alert } from '../components/primitives'
import { AboutYou, useProfile } from '../features/shell/AboutYou'
import '../features/shell/profile.css'

export const Route = createFileRoute('/_app/you')({ component: You })

const THEMES: [Theme, string][] = [
  ['system', 'Match system'],
  ['light', 'Light'],
  ['dark', 'Dark'],
]

function You() {
  const session = useSession()
  const installation = useInstallation()
  const preferences = usePreferences()
  const save = useSavePreferences()
  const navigate = useNavigate()
  const theme = preferences.data?.theme ?? 'system'
  const profile = useProfile()
  const name =
    profile.data?.name || session.data?.name || session.data?.login || 'You'
  return (
    <div className="page">
      <header className="page__head">
        <h1>Profile</h1>
      </header>
      <div className="page__body page__body--narrow profile">
        <section className="profile__card" aria-label="Your profile">
          <PersonAvatar
            account={session.data?.id}
            name={name}
            className="profile__avatar"
          />
          <div className="profile__who">
            <h2>{name}</h2>
            <p className="muted">
              {session.data?.administrator ? 'Administrator' : 'Member'} of{' '}
              {installation.data?.name ?? 'Cubebot'}
            </p>
            {session.data && <PictureControls account={session.data.id} />}
          </div>
        </section>

        <AboutYou />

        <section className="profile__section" aria-labelledby="appearance">
          <div>
            <h3 id="appearance">Appearance</h3>
            <p className="faint small">
              Saved to your account, so every device follows it.
            </p>
          </div>
          <div className="tabs" role="radiogroup" aria-label="Theme">
            {THEMES.map(([value, label]) => (
              <button
                key={value}
                type="button"
                role="radio"
                aria-checked={theme === value}
                onClick={() => {
                  applyTheme(value)
                  if (preferences.data)
                    save.mutate({ ...preferences.data, theme: value })
                }}
              >
                {label}
              </button>
            ))}
          </div>
        </section>

        <section className="profile__section" aria-labelledby="session">
          <div>
            <h3 id="session">Session</h3>
            <p className="faint small">
              Signed in as <strong>{session.data?.login}</strong> on{' '}
              {installation.data?.controller.replaceAll('_', ' ')}.
            </p>
          </div>
          <button
            type="button"
            className="btn"
            onClick={async () => {
              await signOut()
              await navigate({ to: '/sign-in' })
            }}
          >
            <LogOut />
            Sign out
          </button>
        </section>
      </div>
    </div>
  )
}

const SIDE = 256

async function square(file: File): Promise<Blob> {
  const bitmap = await createImageBitmap(file)
  const side = Math.min(bitmap.width, bitmap.height)
  const canvas = document.createElement('canvas')
  canvas.width = SIDE
  canvas.height = SIDE
  const context = canvas.getContext('2d')
  if (!context) throw new Error('This browser cannot resize pictures.')
  context.imageSmoothingQuality = 'high'
  context.drawImage(
    bitmap,
    (bitmap.width - side) / 2,
    (bitmap.height - side) / 2,
    side,
    side,
    0,
    0,
    SIDE,
    SIDE,
  )
  bitmap.close()
  return new Promise((resolve, reject) =>
    canvas.toBlob(
      (blob) =>
        blob
          ? resolve(blob)
          : reject(new Error('The picture could not be read.')),
      'image/png',
    ),
  )
}

function PictureControls({ account }: { account: string }) {
  const client = useQueryClient()
  const avatars = useAvatars()
  const input = useRef<HTMLInputElement>(null)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const has = avatars.data?.some((avatar) => avatar.account === account)
  async function change(body: Blob | null) {
    setBusy(true)
    setError('')
    try {
      const response = await fetch('/api/identity/me/avatar', {
        method: body ? 'PUT' : 'DELETE',
        credentials: 'same-origin',
        headers: body ? { 'content-type': body.type } : {},
        body,
      })
      if (!response.ok) throw new ApiError(response.status)
      await client.invalidateQueries({ queryKey: keys.avatars })
    } catch (caught) {
      setError(
        caught instanceof ApiError && caught.status === 400
          ? 'Choose a JPEG, PNG or similar picture.'
          : failure(caught),
      )
    } finally {
      setBusy(false)
    }
  }
  return (
    <div className="profile__picture">
      <div className="actions">
        <button
          type="button"
          className="btn btn--sm"
          disabled={busy}
          onClick={() => input.current?.click()}
        >
          <ImageUp />
          {busy ? 'Saving…' : has ? 'Change picture' : 'Upload picture'}
        </button>
        {has && (
          <button
            type="button"
            className="btn btn--ghost btn--sm"
            disabled={busy}
            onClick={() => void change(null)}
          >
            <Trash2 />
            Remove
          </button>
        )}
      </div>
      <input
        ref={input}
        type="file"
        accept="image/*"
        hidden
        onChange={async (event) => {
          const file = event.target.files?.[0]
          event.target.value = ''
          if (!file) return
          try {
            await change(await square(file))
          } catch {
            setError('That file is not a picture this browser can read.')
          }
        }}
      />
      <Alert>{error}</Alert>
    </div>
  )
}
