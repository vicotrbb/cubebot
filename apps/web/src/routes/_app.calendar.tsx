import { createFileRoute } from '@tanstack/react-router'
import { Calendar } from '../features/calendar/Calendar'
import type { View } from '../features/calendar/model'

const DAY = /^\d{4}-\d{2}-\d{2}$/
// The pattern alone lets through days like 2026-02-31.
const isDay = (value: unknown): value is string =>
  typeof value === 'string' &&
  DAY.test(value) &&
  !Number.isNaN(Date.parse(value)) &&
  new Date(value).toISOString().startsWith(value)
const VIEWS = ['day', 'week', 'month'] as const

export const Route = createFileRoute('/_app/calendar')({
  validateSearch: (
    search: Record<string, unknown>,
  ): { view?: View | undefined; date?: string | undefined } => ({
    view: VIEWS.find((view) => view === search.view),
    date: isDay(search.date) ? search.date : undefined,
  }),
  component: CalendarRoute,
})

function CalendarRoute() {
  const { view, date } = Route.useSearch()
  const navigate = Route.useNavigate()
  return (
    <Calendar
      view={view ?? 'week'}
      date={date}
      onGo={(place) => void navigate({ search: place, replace: true })}
    />
  )
}
