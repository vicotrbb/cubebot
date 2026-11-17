import { useEffect, useState } from 'react'

const PHONE = '(max-width: 820px)'

export function useIsPhone(media = PHONE) {
  const [phone, setPhone] = useState(
    () => typeof window !== 'undefined' && matchMedia(media).matches,
  )
  useEffect(() => {
    const query = matchMedia(media)
    const update = () => setPhone(query.matches)
    update()
    query.addEventListener('change', update)
    return () => query.removeEventListener('change', update)
  }, [media])
  return phone
}
