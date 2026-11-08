import { admin } from '../../api/admin'
import {
  changesTo,
  grantsOn,
  type Grant,
  type Level,
  type Target,
} from './access'

export type Recipient = { kind: 'user' | 'group'; id: string }

/** Moves one person's or group's grants on a target to a level, revoking what it no longer needs. */
export async function applyLevel(
  recipient: Recipient,
  own: Grant[],
  target: Target,
  level: Exclude<Level, 'custom'>,
) {
  const { grant, revoke } = changesTo(grantsOn(own, target), target, level)
  for (const item of revoke) await admin.revokeGrant(item.id, item.revision)
  for (const need of grant)
    await admin.grant(
      recipient,
      { kind: need.scope, id: target.id } as Parameters<typeof admin.grant>[1],
      need.capability,
    )
}
