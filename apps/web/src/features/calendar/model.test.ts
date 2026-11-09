// @ts-expect-error bun provides this at test time
import { describe, expect, test } from 'bun:test'
import type { Schemas } from '../../api/types'
import {
  cadence,
  cronPhrase,
  busyOn,
  isDay,
  mergeDigests,
  shift,
  stretch,
  formatRange,
  layoutDay,
  localDay,
  nowIn,
  range,
  userZone,
  weekLanes,
} from './model'

type Item = Schemas['Item']
const SP = 'America/Sao_Paulo'

function item(start: string, end: string | null, allDay = false): Item {
  return {
    all_day: allDay,
    editable: true,
    end,
    id: `${start}-${end}`,
    kind: 'event',
    source: 'calendar:c:1',
    start,
    title: 'x',
  }
}

describe('range', () => {
  test('a month covers whole weeks, from the week start', () => {
    const grid = range('month', '2026-11-09', 'UTC', 1)
    expect(grid.days[0]).toBe('2026-10-26')
    expect(grid.days.at(-1)).toBe('2026-12-06')
    expect(grid.days).toHaveLength(42)
  })

  test('a month that fits five weeks has 35 days', () => {
    expect(range('month', '2026-09-15', 'UTC', 1).days).toHaveLength(35)
  })

  test('the week start can be Sunday', () => {
    expect(range('week', '2026-11-09', 'UTC', 0).days[0]).toBe('2026-11-08')
  })

  test('from and to are the viewer zone midnights', () => {
    const day = range('day', '2026-11-09', SP, 1)
    expect(day.days).toEqual(['2026-11-09'])
    expect(day.from).toBe('2026-11-09T03:00:00.000Z')
    expect(day.to).toBe('2026-11-10T03:00:00.000Z')
  })

  test('a day whose midnight does not exist starts at its first instant', () => {
    const zone = 'America/Santiago'
    expect(range('day', '2026-09-06', zone, 1).from).toBe(
      '2026-09-06T04:00:00.000Z',
    )
    expect(range('day', '2026-09-05', zone, 1).to).toBe(
      '2026-09-06T04:00:00.000Z',
    )
  })

  test('the week a clock change falls in is 169 hours', () => {
    const week = range('week', '2026-11-01', 'America/New_York', 1)
    expect(week.days).toHaveLength(7)
    const hours = (Date.parse(week.to) - Date.parse(week.from)) / 36e5
    expect(hours).toBe(169)
  })
})

describe('localDay', () => {
  test('an event across local midnight touches two days', () => {
    expect(
      localDay(item('2026-11-10T02:30:00Z', '2026-11-10T04:30:00Z'), SP),
    ).toEqual(['2026-11-09', '2026-11-10'])
  })

  test('an event ending exactly at midnight stays on its day', () => {
    expect(
      localDay(item('2026-11-09T20:00:00Z', '2026-11-10T03:00:00Z'), SP),
    ).toEqual(['2026-11-09'])
  })

  test('an all-day item excludes its end day', () => {
    expect(localDay(item('2026-11-09', '2026-11-11', true), SP)).toEqual([
      '2026-11-09',
      '2026-11-10',
    ])
  })

  test('an item without an end is on its start day', () => {
    expect(localDay(item('2026-11-09T12:00:00Z', null), SP)).toEqual([
      '2026-11-09',
    ])
  })
})

describe('layoutDay', () => {
  test('items starting close together share columns and a later one is alone', () => {
    const laid = layoutDay(
      [
        item('2026-11-09T13:00:00Z', '2026-11-09T15:00:00Z'),
        item('2026-11-09T13:10:00Z', '2026-11-09T14:30:00Z'),
        item('2026-11-09T13:20:00Z', '2026-11-09T14:45:00Z'),
        item('2026-11-09T18:00:00Z', '2026-11-09T19:00:00Z'),
      ],
      '2026-11-09',
      SP,
    )
    expect(laid.map((l) => [l.column, l.columns])).toEqual([
      [0, 3],
      [1, 3],
      [2, 3],
      [0, 1],
    ])
    expect(laid[0]).toMatchObject({ top: 600, height: 120 })
  })

  test('items starting far apart stack, each indented over the ones it overlaps', () => {
    const laid = layoutDay(
      [
        item('2026-11-09T13:00:00Z', '2026-11-09T15:00:00Z'),
        item('2026-11-09T13:30:00Z', '2026-11-09T14:00:00Z'),
        item('2026-11-09T14:15:00Z', '2026-11-09T14:45:00Z'),
      ],
      '2026-11-09',
      SP,
    )
    expect(laid.map((l) => [l.column, l.columns, l.indent])).toEqual([
      [0, 1, 0],
      [0, 1, 1],
      [0, 1, 1],
    ])
  })

  test('a point item gets a minimum height and all-day items are left out', () => {
    const laid = layoutDay(
      [
        item('2026-11-09T12:00:00Z', null),
        item('2026-11-09', '2026-11-10', true),
      ],
      '2026-11-09',
      SP,
    )
    expect(laid).toHaveLength(1)
    expect(laid[0]?.height).toBe(30)
  })

  test('tops follow the wall clock on a fall-back day', () => {
    const NY = 'America/New_York'
    const laid = layoutDay(
      [
        item('2026-11-01T14:00:00Z', '2026-11-01T15:00:00Z'),
        item('2026-11-02T04:30:00Z', '2026-11-02T04:45:00Z'),
      ],
      '2026-11-01',
      NY,
    )
    expect(laid[0]).toMatchObject({ top: 540, height: 60 })
    expect(laid[1]).toMatchObject({ top: 1410, height: 30 })
  })

  test('tops follow the wall clock on a spring-forward day', () => {
    const laid = layoutDay(
      [item('2026-03-08T19:00:00Z', '2026-03-08T20:00:00Z')],
      '2026-03-08',
      'America/New_York',
    )
    expect(laid[0]).toMatchObject({ top: 900, height: 60 })
  })

  test('a point item late in the day stays inside the grid', () => {
    const laid = layoutDay(
      [item('2026-11-10T02:50:00Z', null)],
      '2026-11-09',
      SP,
    )
    expect(laid[0]).toMatchObject({ top: 1410, height: 30 })
  })

  test('an item from the day before is cut at midnight', () => {
    const laid = layoutDay(
      [item('2026-11-10T02:30:00Z', '2026-11-10T04:30:00Z')],
      '2026-11-10',
      SP,
    )
    expect(laid[0]).toMatchObject({ top: 0, height: 90 })
  })
})

describe('text', () => {
  test('cron phrases', () => {
    expect(cronPhrase('0 9 * * 1-5')).toBe('Every weekday at 09:00')
    expect(cronPhrase('30 17 * * *')).toBe('Every day at 17:30')
    expect(cronPhrase('0 * * * *')).toBe('Every hour')
    expect(cronPhrase('0 9 * * 1')).toBe('Every Monday at 09:00')
    expect(cronPhrase('*/7 3 1 * *')).toBeNull()
    expect(cronPhrase('60 25 * * *')).toBeNull()
    expect(cronPhrase('0 9 * * 9')).toBeNull()
  })

  test('the viewer zone falls back to the browser zone', () => {
    expect(userZone(SP)).toBe(SP)
    expect(userZone('')).toBe(Intl.DateTimeFormat().resolvedOptions().timeZone)
  })

  test('a range is shown in the zone', () => {
    expect(formatRange(item('2026-11-09', '2026-11-10', true), SP)).toBe(
      'All day',
    )
    expect(
      formatRange(item('2026-11-09T12:00:00Z', '2026-11-09T13:30:00Z'), SP),
    ).toBe('09:00 – 10:30')
  })
})

describe('weekLanes', () => {
  const week = range('week', '2026-11-09', SP, 1).days
  const lanesOf = (laid: ReturnType<typeof weekLanes>) =>
    laid.bars.map((bar) => [bar.item.id, bar.first, bar.last, bar.lane])

  test('a spanning event takes the first lane and timed items stack under it', () => {
    const offsite = { ...item('2026-11-11', '2026-11-14', true), id: 'off' }
    const meeting = {
      ...item('2026-11-12T13:00:00Z', '2026-11-12T14:00:00Z'),
      id: 'm',
    }
    const early = { ...item('2026-11-09T12:00:00Z', null), id: 'e' }
    expect(lanesOf(weekLanes([early, meeting, offsite], week, SP, 4))).toEqual([
      ['off', 2, 4, 0],
      ['e', 0, 0, 0],
      ['m', 3, 3, 1],
    ])
  })

  test('an event from the week before starts at the first column', () => {
    const long = item('2026-11-05', '2026-11-11', true)
    expect(lanesOf(weekLanes([long], week, SP, 3))[0]?.slice(1)).toEqual([
      0, 1, 0,
    ])
  })

  test('overflow keeps the last lane for the count of hidden items', () => {
    const many = [1, 2, 3, 4].map((at) => ({
      ...item(`2026-11-10T1${at}:00:00Z`, null),
      id: `t${at}`,
    }))
    const laid = weekLanes(many, week, SP, 3)
    expect(laid.bars.map((bar) => bar.item.id)).toEqual(['t1', 't2'])
    expect(laid.more).toEqual([0, 2, 0, 0, 0, 0, 0])
  })

  test('a longer digest bar goes over a shorter event, and timed items start under both', () => {
    const offsite = { ...item('2026-11-11', '2026-11-14', true), id: 'off' }
    const digest = {
      ...item('2026-11-08', '2026-11-15', true),
      id: 'd',
      kind: 'digest',
    }
    const early = { ...item('2026-11-09T12:00:00Z', null), id: 'e' }
    expect(lanesOf(weekLanes([early, digest, offsite], week, SP, 5))).toEqual([
      ['d', 0, 5, 0],
      ['off', 2, 4, 1],
      ['e', 0, 0, 1],
    ])
  })

  test('a digest goes under an event of the same days, and over timed items', () => {
    const digest = {
      ...item('2026-11-09', '2026-11-10', true),
      id: 'd',
      kind: 'digest',
    }
    const timed = { ...item('2026-11-09T12:00:00Z', null), id: 't' }
    const allDay = { ...item('2026-11-09', '2026-11-10', true), id: 'a' }
    expect(
      lanesOf(weekLanes([timed, digest, allDay], week, SP, 5)).map((l) => l[0]),
    ).toEqual(['a', 'd', 't'])
  })
})

describe('now and cadence', () => {
  test('now is the zone day and its wall-clock minute', () => {
    expect(nowIn(SP, Date.parse('2026-11-10T02:30:00Z'))).toEqual({
      day: '2026-11-09',
      minute: 23 * 60 + 30,
    })
  })

  test('a digest says how often its routine runs', () => {
    expect(cadence('*/5 * * * *')).toBe('every 5 min')
    expect(cadence('* * * * *')).toBe('every minute')
    expect(cadence('*/10 9-17 * * *')).toBe('many times a day')
  })
})

describe('digests', () => {
  const digest = (routine: string, day: string, next: string): Item => ({
    ...item(day, next, true),
    id: `digest:${routine}:${day}`,
    kind: 'digest',
    routine: {
      agent: 'a',
      routine,
      schedule: '*/5 * * * *',
      time_zone: 'UTC',
      runs: 288,
    },
  })

  test("a routine's digests on consecutive days become one item over them", () => {
    const merged = mergeDigests([
      digest('r', '2026-11-09', '2026-11-10'),
      digest('s', '2026-11-09', '2026-11-10'),
      item('2026-11-09T12:00:00Z', null),
      digest('r', '2026-11-10', '2026-11-11'),
      digest('r', '2026-11-12', '2026-11-13'),
    ])
    expect(merged.map((m) => [m.id, m.start, m.end, m.days?.length])).toEqual([
      ['digest:r:2026-11-09', '2026-11-09', '2026-11-11', 2],
      ['digest:s:2026-11-09', '2026-11-09', '2026-11-10', 1],
      ['2026-11-09T12:00:00Z-null', '2026-11-09T12:00:00Z', null, undefined],
      ['digest:r:2026-11-12', '2026-11-12', '2026-11-13', 1],
    ])
  })

  test('days out of order start a new bar rather than a wrong one', () => {
    const merged = mergeDigests([
      digest('r', '2026-11-10', '2026-11-11'),
      digest('r', '2026-11-09', '2026-11-10'),
    ])
    expect(merged.map((m) => [m.start, m.end])).toEqual([
      ['2026-11-10', '2026-11-11'],
      ['2026-11-09', '2026-11-10'],
    ])
  })

  test('a day with only a digest is not busy', () => {
    const week = range('week', '2026-11-09', SP, 1).days
    const { bars } = weekLanes(
      [
        ...mergeDigests([
          digest('r', '2026-11-09', '2026-11-10'),
          digest('r', '2026-11-10', '2026-11-11'),
        ]),
        { ...item('2026-11-10T12:00:00Z', null), id: 't' },
      ],
      week,
      SP,
      Infinity,
    )
    expect(busyOn(bars, 0)).toEqual([])
    expect(busyOn(bars, 1).map((bar) => bar.item.id)).toEqual(['t'])
  })

  test('merging leaves the items it was given unchanged', () => {
    const first = digest('r', '2026-11-09', '2026-11-10')
    mergeDigests([first, digest('r', '2026-11-10', '2026-11-11')])
    expect(first.end).toBe('2026-11-10')
  })
})

describe('navigation', () => {
  test('a month step keeps the day number where the month has it', () => {
    expect(shift('month', '2026-01-31', 1)).toBe('2026-02-28')
    expect(shift('month', '2026-03-31', -1)).toBe('2026-02-28')
    expect(shift('month', '2026-12-15', 1)).toBe('2027-01-15')
    expect(shift('week', '2026-11-09', -1)).toBe('2026-11-02')
    expect(shift('day', '2026-12-31', 1)).toBe('2027-01-01')
  })

  test('the three-day week runs from the chosen day', () => {
    const three = stretch('2026-11-09', 3, SP)
    expect(three.days).toEqual(['2026-11-09', '2026-11-10', '2026-11-11'])
    expect(three.from).toBe('2026-11-09T03:00:00.000Z')
    expect(three.to).toBe('2026-11-12T03:00:00.000Z')
  })

  test('only real days are accepted from the address', () => {
    expect(isDay('2026-11-09')).toBe(true)
    expect(isDay('2024-02-29')).toBe(true)
    expect(isDay('2026-02-29')).toBe(false)
    expect(isDay('2026-13-01')).toBe(false)
    expect(isDay('2026-1-9')).toBe(false)
    expect(isDay(20261109)).toBe(false)
  })
})
