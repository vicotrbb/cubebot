import { useQuery } from '@tanstack/react-query'
import { ArrowUpRight, CircleArrowUp, X } from 'lucide-react'
import { useState } from 'react'
import { api, ok } from '../../api/client'
import { useSession } from '../../app/queries'

const DISMISSED_KEY = 'cubebot:dismissed-release'

export function useInstallationAbout(enabled = true) {
  return useQuery({
    queryKey: ['installation-about'],
    queryFn: () => ok(api.GET('/api/installation/about')),
    refetchInterval: 60 * 60_000,
    enabled,
  })
}

function dismissed() {
  try {
    return localStorage.getItem(DISMISSED_KEY)
  } catch {
    return null
  }
}

export function UpdateHint() {
  const administrator = Boolean(useSession().data?.administrator)
  const about = useInstallationAbout(administrator)
  const [hidden, setHidden] = useState(dismissed)
  const release = about.data?.newer_release
  if (!release || hidden === release.version) return null
  const hide = () => {
    setHidden(release.version)
    try {
      localStorage.setItem(DISMISSED_KEY, release.version)
    } catch {
      return
    }
  }
  return (
    <section className="update-hint" aria-label="Update available">
      <CircleArrowUp className="update-hint__icon" aria-hidden="true" />
      <p className="update-hint__text">
        <strong>Cubebot {release.version} is out</strong>
        <span>You run {about.data?.version}</span>
      </p>
      <a
        className="update-hint__link"
        href={release.url}
        target="_blank"
        rel="noreferrer"
      >
        What’s new <ArrowUpRight aria-hidden="true" />
      </a>
      <button
        type="button"
        className="btn btn--ghost btn--icon update-hint__close"
        aria-label={`Hide until the version after ${release.version}`}
        title="Hide"
        onClick={hide}
      >
        <X />
      </button>
    </section>
  )
}
