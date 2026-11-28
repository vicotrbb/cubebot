import { useSession, useStanding } from '../../app/queries'

/** Who is looking at a platform page: auditors read it, only administrators change it. */
export function useViewer() {
  const session = useSession()
  const standing = useStanding()
  return {
    id: session.data?.id ?? '',
    writer: session.data?.role === 'administrator',
    owner: Boolean(standing.data?.owner),
  }
}
