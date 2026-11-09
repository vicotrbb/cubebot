import {
  keepPreviousData,
  useMutation,
  useQuery,
  useQueryClient,
} from '@tanstack/react-query'
import { api, ApiError, ok } from '../../api/client'
import type { Schemas } from '../../api/types'
import { answer } from '../connections/api'

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

/** Runs a write, then reads the calendar again, after a failure too: it may have come from a stale copy. */
export function useWrite() {
  const client = useQueryClient()
  return useMutation({
    mutationFn: (write: () => Promise<unknown>) => write(),
    onSettled: () => client.invalidateQueries({ queryKey: CALENDAR_KEY }),
  })
}

const READ_ONLY =
  'Cubebot may only read this calendar. An administrator can allow changes in Platform › Connections.'
const STALE =
  'This event changed in its calendar after you opened it, so nothing was saved. Open it again to see the latest.'

/** The vendor's own reason on a refusal; a 403 means the connection's level stops writes. */
async function written<T>(
  result: Promise<{ data?: T; error?: unknown; response: Response }>,
) {
  try {
    return await answer(result)
  } catch (error) {
    if (error instanceof ApiError && error.status === 403)
      throw new Error(READ_ONLY, { cause: error })
    // CalDAV refuses a write over a newer copy (its etag no longer matches).
    if (
      error instanceof Error &&
      /changed since it was read/i.test(error.message)
    )
      throw new Error(STALE, { cause: error })
    throw error
  }
}

export const createEvent = (body: Schemas['NewEvent']) =>
  written(api.POST('/api/calendar/events', { body }))

export const changeEvent = (body: Schemas['ChangeEvent']) =>
  written(api.PUT('/api/calendar/events', { body }))

export const removeEvent = (body: Schemas['RemoveEvent']) =>
  written(api.DELETE('/api/calendar/events', { body }))

/** Names an event or its occurrence as its detail does; a CalDAV occurrence is read in its own zone. */
export function eventRef(
  detail: Schemas['EventDetail'],
  zone: string,
  scope: Schemas['EventScope'],
) {
  return {
    connection: detail.connection,
    calendar: detail.calendar,
    event: detail.event,
    etag: detail.etag ?? null,
    occurrence: detail.occurrence ?? null,
    series: detail.series ?? null,
    time_zone: detail.time_zone ?? zone,
    scope,
  }
}
