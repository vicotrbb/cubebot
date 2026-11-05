import { useCallback, useEffect, useRef, useState } from 'react'
import { useQueryClient } from '@tanstack/react-query'
import { failure } from '../../api/client'
import { kbKeys, savePage, StaleSave, type Page } from './api'

const QUIET = 700

export type SaveState =
  | { status: 'saved' | 'waiting' | 'saving' }
  | { status: 'conflict'; revision: number }
  | { status: 'failed'; message: string }

type Draft = { title?: string; body?: string }

/** Saves a page a moment after the last keystroke, one request at a time, and never over someone
 *  else's newer revision: a refusal waits for the person to choose. */
export function useSave(page: Page, onTitle: () => void) {
  const client = useQueryClient()
  const id = page.id
  const [state, setState] = useState<SaveState>({ status: 'saved' })
  const revision = useRef(page.revision)
  const draft = useRef<Draft>({})
  const timer = useRef<number | undefined>(undefined)
  const running = useRef(false)
  // A refused save holds further ones until the person chooses.
  const conflicted = useRef(false)

  const send = useCallback(async () => {
    if (running.current) return
    running.current = true
    // What arrives while one request is out goes in the next.
    while (Object.keys(draft.current).length) {
      const sending = draft.current
      draft.current = {}
      setState({ status: 'saving' })
      try {
        const saved = await savePage(id, {
          expected_revision: revision.current,
          ...sending,
        })
        revision.current = saved.revision
        client.setQueryData<Page>(kbKeys.page(id), saved)
        if (sending.title !== undefined) onTitle()
      } catch (error) {
        // What failed to send stays, under anything typed since.
        draft.current = { ...sending, ...draft.current }
        running.current = false
        conflicted.current = error instanceof StaleSave
        setState(
          error instanceof StaleSave
            ? { status: 'conflict', revision: error.revision }
            : { status: 'failed', message: failure(error) },
        )
        return
      }
    }
    running.current = false
    setState({ status: 'saved' })
  }, [client, id, onTitle])

  const change = useCallback(
    (next: Draft) => {
      draft.current = { ...draft.current, ...next }
      if (conflicted.current) return
      setState({ status: 'waiting' })
      window.clearTimeout(timer.current)
      timer.current = window.setTimeout(() => void send(), QUIET)
    },
    [send],
  )

  /** Writes what was typed over the newer revision. */
  const keepMine = useCallback(() => {
    if (state.status !== 'conflict') return
    conflicted.current = false
    revision.current = state.revision
    void send()
  }, [send, state])

  /** Drops what was typed and takes the current page. */
  const discard = useCallback((current: number) => {
    window.clearTimeout(timer.current)
    conflicted.current = false
    draft.current = {}
    revision.current = current
    setState({ status: 'saved' })
  }, [])

  /** Takes a revision someone else saved, when it is one: true if `current` is newer than
   *  anything this page has read or saved, and nothing is waiting to be saved over it. */
  const follow = useCallback((current: number) => {
    const busy = running.current || Object.keys(draft.current).length > 0
    if (busy || current <= revision.current) return false
    revision.current = current
    return true
  }, [])

  // What is typed survives leaving the page or closing the tab.
  useEffect(() => {
    const flush = () => {
      // A refused save's revision is stale, so this one would only be refused too.
      if (conflicted.current || !Object.keys(draft.current).length) return
      const body = JSON.stringify({
        expected_revision: revision.current,
        ...draft.current,
      })
      draft.current = {}
      void fetch(`/api/kb/pages/${id}`, {
        method: 'PATCH',
        body,
        credentials: 'same-origin',
        headers: { 'content-type': 'application/json' },
        // Past the browser's keepalive limit the tab must stay open for it.
        keepalive: body.length < 60_000,
      }).catch(() => undefined) // The page is gone: nobody is left to tell.
    }
    window.addEventListener('pagehide', flush)
    return () => {
      window.removeEventListener('pagehide', flush)
      window.clearTimeout(timer.current)
      // A save under way sends what is left when it returns, at the revision it gets back.
      if (!running.current) flush()
    }
  }, [id])

  return { state, change, keepMine, discard, follow, retry: send }
}
