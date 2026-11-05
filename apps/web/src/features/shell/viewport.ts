import { useEffect, useState } from 'react'

const PHONE = '(max-width: 820px)'

export function useIsPhone() {
  const [phone, setPhone] = useState(
    () => typeof window !== 'undefined' && matchMedia(PHONE).matches,
  )
  useEffect(() => {
    const query = matchMedia(PHONE)
    const update = () => setPhone(query.matches)
    update()
    query.addEventListener('change', update)
    return () => query.removeEventListener('change', update)
  }, [])
  return phone
}
