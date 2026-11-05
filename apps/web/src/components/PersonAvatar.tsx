import { useAvatars } from '../app/queries'

export function PersonAvatar({
  account,
  login,
  name = login,
  className = '',
}: {
  account?: string | undefined
  login?: string | undefined
  name?: string | undefined
  className?: string
}) {
  const avatars = useAvatars()
  const found = avatars.data?.find(
    (avatar) => avatar.account === account || avatar.login === login,
  )
  const classes = `avatar-initial ${className}`.trim()
  if (found)
    return (
      <img
        className={classes}
        src={`/api/identity/accounts/${found.account}/avatar?v=${found.revision}`}
        alt=""
        aria-hidden="true"
        decoding="async"
      />
    )
  return (
    <span className={classes} aria-hidden="true">
      {(name || '?').slice(0, 1).toUpperCase()}
    </span>
  )
}
