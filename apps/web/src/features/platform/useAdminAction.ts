import { useQueryClient } from '@tanstack/react-query'
import { useState } from 'react'
import { failure } from '../../api/client'

export function useAdminAction(invalidate: string[]) {
  const client = useQueryClient()
  const [error, setError] = useState('')
  const [notice, setNotice] = useState('')
  const [busy, setBusy] = useState(false)
  async function run<T>(
    action: () => Promise<T>,
    done: string,
  ): Promise<T | undefined> {
    setBusy(true)
    setError('')
    setNotice('')
    try {
      const result = await action()
      await Promise.all(
        invalidate.map((key) => client.invalidateQueries({ queryKey: [key] })),
      )
      setNotice(done)
      return result
    } catch (error) {
      setError(failure(error))
      return undefined
    } finally {
      setBusy(false)
    }
  }
  return { run, error, notice, busy }
}
