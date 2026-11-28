import { api, ok } from './client'
import type { components } from './schema'

type Schemas = components['schemas']
export type Standing = Schemas['Standing']
export type ProviderKind = Schemas['ProviderKind']

/** What the signed-in person manages about themselves. */
export const account = {
  standing: () => ok(api.GET('/api/identity/me/standing')),
  changePassword: (current: string, next: string) =>
    ok(
      api.POST('/api/identity/me/password', {
        body: { current, password: next },
      }),
    ),
  sessions: () => ok(api.GET('/api/identity/me/sessions')),
  endSession: (session: string) =>
    ok(
      api.DELETE('/api/identity/me/sessions/{id}', {
        params: { path: { id: session } },
      }),
    ),
  createToken: (name: string, days: number) =>
    ok(api.POST('/api/identity/me/tokens', { body: { name, days } })),
  enrollTwoFactor: () => ok(api.POST('/api/identity/me/two-factor')),
  confirmTwoFactor: (code: string) =>
    ok(api.POST('/api/identity/me/two-factor/confirm', { body: { code } })),
  disableTwoFactor: (code: string) =>
    ok(api.POST('/api/identity/me/two-factor/disable', { body: { code } })),
  redeemReset: (token: string, password: string) =>
    ok(api.POST('/api/identity/password-reset', { body: { token, password } })),
  providers: () => ok(api.GET('/api/me/providers')),
  saveProvider: (
    provider: ProviderKind,
    api_key: string,
    context_window?: number,
  ) =>
    ok(
      api.PUT('/api/me/providers/{provider}', {
        params: { path: { provider } },
        body: { api_key, ...(context_window ? { context_window } : {}) },
      }),
    ),
  forgetProvider: (provider: ProviderKind) =>
    ok(
      api.DELETE('/api/me/providers/{provider}', {
        params: { path: { provider } },
      }),
    ),
}
