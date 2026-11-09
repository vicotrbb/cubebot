import { keepPreviousData, useQuery } from '@tanstack/react-query'
import { api, ok } from '../../api/client'

export const CALENDAR_KEY = ['calendar']

/** One range of the merged calendar; the previous range stays on screen while the next loads. */
export function useCalendar(from: string, to: string, timeZone: string) {
  return useQuery({
    queryKey: [...CALENDAR_KEY, from, to, timeZone],
    queryFn: () =>
      ok(
        api.GET('/api/calendar', {
          params: { query: { from, to, time_zone: timeZone } },
        }),
      ),
    placeholderData: keepPreviousData,
    staleTime: 60_000,
  })
}
