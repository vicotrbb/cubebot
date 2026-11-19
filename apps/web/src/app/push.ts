import { useCallback, useEffect, useState } from 'react'
import { api, ok } from '../api/client'

export type PushState =
  'checking' | 'unsupported' | 'install' | 'denied' | 'off' | 'on'

const supported = () =>
  typeof window !== 'undefined' &&
  'serviceWorker' in navigator &&
  'PushManager' in window &&
  'Notification' in window

const appleMobile = () =>
  /iPhone|iPad|iPod/.test(navigator.userAgent) ||
  (navigator.platform === 'MacIntel' && navigator.maxTouchPoints > 1)

async function subscription() {
  if (!supported()) return null
  const worker = await navigator.serviceWorker.getRegistration()
  return (await worker?.pushManager.getSubscription()) ?? null
}

async function remember(device: PushSubscription) {
  const { endpoint, keys } = device.toJSON()
  if (!endpoint || !keys?.p256dh || !keys.auth)
    throw new Error('This browser gave an incomplete subscription.')
  await ok(
    api.PUT('/api/push/subscriptions', {
      body: { endpoint, p256dh: keys.p256dh, auth: keys.auth },
    }),
  )
}

const bytes = (base64url: string) =>
  Uint8Array.from(
    atob(base64url.replaceAll('-', '+').replaceAll('_', '/')),
    (char) => char.charCodeAt(0),
  )

const sameKey = (current: ArrayBuffer | null, key: Uint8Array) =>
  current?.byteLength === key.length &&
  new Uint8Array(current).every((byte, index) => byte === key[index])

async function readState(): Promise<PushState> {
  if (!supported()) return appleMobile() ? 'install' : 'unsupported'
  if (Notification.permission === 'denied') return 'denied'
  if (Notification.permission !== 'granted') return 'off'
  return (await subscription()) ? 'on' : 'off'
}

export async function syncPush() {
  if (!supported() || Notification.permission !== 'granted') return
  const device = await subscription()
  if (device) await remember(device)
}

export async function forgetPush() {
  const device = await subscription().catch(() => null)
  if (!device) return
  await ok(
    api.DELETE('/api/push/subscriptions', {
      body: { endpoint: device.endpoint },
    }),
  ).catch(() => undefined)
  await device.unsubscribe().catch(() => false)
}

async function turnOn(): Promise<PushState> {
  const permission = await Notification.requestPermission()
  if (permission !== 'granted')
    return permission === 'denied' ? 'denied' : 'off'
  const [worker, settings] = await Promise.all([
    navigator.serviceWorker.ready,
    ok(api.GET('/api/push')),
  ])
  const key = bytes(settings.public_key)
  let device = await worker.pushManager.getSubscription()
  if (device && !sameKey(device.options.applicationServerKey, key)) {
    await device.unsubscribe()
    device = null
  }
  device ??= await worker.pushManager.subscribe({
    userVisibleOnly: true,
    applicationServerKey: key,
  })
  await remember(device)
  return 'on'
}

export function usePush() {
  const [state, setState] = useState<PushState>('checking')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  useEffect(() => {
    void readState().then(setState)
  }, [])
  const run = useCallback(async (change: () => Promise<PushState>) => {
    setBusy(true)
    setError('')
    try {
      setState(await change())
    } catch (caught) {
      setError(
        caught instanceof DOMException || !(caught instanceof Error)
          ? 'This browser could not turn notifications on. Try again in a moment.'
          : caught.message,
      )
      setState(await readState())
    } finally {
      setBusy(false)
    }
  }, [])
  return {
    state,
    busy,
    error,
    enable: () => run(turnOn),
    disable: () =>
      run(async () => {
        await forgetPush()
        return readState()
      }),
  }
}
