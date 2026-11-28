import type { Standing } from '../../api/account'

export const PASSWORD_MIN = 12

/** Why a new password cannot be saved yet, or '' when it can. */
export function passwordProblem(password: string, confirm: string) {
  if ([...password].length < PASSWORD_MIN)
    return `Use at least ${PASSWORD_MIN} characters.`
  if (password !== confirm) return 'The two passwords do not match.'
  return ''
}

const ROLES: Record<Standing['role'], { label: string; detail: string }> = {
  administrator: { label: 'Administrator', detail: '' },
  auditor: { label: 'Auditor', detail: 'Reads everything, changes nothing.' },
  member: { label: 'Member', detail: '' },
  guest: { label: 'Guest', detail: 'Uses only what is shared with you.' },
}

/** The person's place in the installation, in plain words. */
export function roleOf(standing: Pick<Standing, 'role' | 'owner'>) {
  const role = ROLES[standing.role]
  return standing.owner ? { ...role, label: 'Installation owner' } : role
}

/** A base32 secret in groups of four, easier to type into an authenticator. */
export function groupedSecret(secret: string) {
  return (secret.replace(/\s+/g, '').match(/.{1,4}/g) ?? []).join(' ')
}

/** The recovery codes as a text file someone keeps somewhere safe. */
export function recoveryFile(codes: string[], login: string, site: string) {
  return [
    `Cubebot recovery codes for ${login} on ${site}`,
    'Each code signs you in once in place of an authenticator code.',
    '',
    ...codes,
    '',
  ].join('\n')
}

/** Seconds since the epoch, as the API sends them, in the console's relative wording. */
export function fromUnix(seconds: number | null | undefined) {
  return seconds ? new Date(seconds * 1000).toISOString() : null
}

/** What a sign-in refusal means, from the body the login endpoint answers with. */
export function signInRefusal(status: number, body: unknown) {
  const error = (body as { error?: unknown } | undefined)?.error
  if (status === 401 && error === 'two_factor_required') return 'code'
  if (status === 401) return 'wrong'
  return 'other'
}
