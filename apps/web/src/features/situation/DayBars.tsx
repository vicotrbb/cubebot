import { useState } from 'react'
import { fillDays } from './format'

type Day = { day: string; tokens: number; cost_usd: number }

const WIDTH = 720
const HEIGHT = 112

function dayLabel(day: string) {
  return new Date(`${day}T00:00:00Z`).toLocaleDateString(undefined, {
    month: 'short',
    day: 'numeric',
    timeZone: 'UTC',
  })
}

/** One measure, by day, for 30 days: thin rounded bars on a baseline, labelled at the ends, with
 *  a table behind it for anyone who cannot read the marks. */
export function DayBars({
  days,
  measure,
  format,
  label,
}: {
  days: Day[]
  measure: 'cost_usd' | 'tokens'
  format: (value: number) => string
  label: string
}) {
  const [hover, setHover] = useState<number | null>(null)
  const filled = fillDays(
    days,
    (day) => ({ day, tokens: 0, cost_usd: 0 }),
    new Date(),
  )
  const peak = Math.max(...filled.map((day) => day[measure]), 0)
  const slot = WIDTH / filled.length
  const shown = hover === null ? null : filled[hover]
  const plot = HEIGHT - 1
  return (
    <figure className="bars">
      <div className="bars__read" aria-hidden="true">
        {shown ? (
          <>
            <strong>{format(shown[measure])}</strong> {dayLabel(shown.day)}
          </>
        ) : (
          <>
            <strong>{format(peak)}</strong> peak day
          </>
        )}
      </div>
      <svg
        viewBox={`0 0 ${WIDTH} ${HEIGHT}`}
        preserveAspectRatio="none"
        role="img"
        aria-label={`${label}, last 30 days; peak ${format(peak)}`}
        onPointerLeave={() => setHover(null)}
      >
        <line
          x1="0"
          x2={WIDTH}
          y1={plot + 0.5}
          y2={plot + 0.5}
          className="bars__base"
        />
        {filled.map((day, index) => {
          const value = day[measure]
          const height =
            peak > 0 && value > 0 ? Math.max(2, (value / peak) * (plot - 4)) : 0
          return (
            <g key={day.day}>
              <rect
                x={index * slot + 2}
                width={Math.max(2, slot - 4)}
                y={plot - height}
                height={height}
                rx={Math.min(3, height / 2)}
                className={hover === index ? 'bars__bar is-on' : 'bars__bar'}
              />
              {/* The whole column answers, however short its bar. */}
              <rect
                x={index * slot}
                width={slot}
                y="0"
                height={plot}
                fill="transparent"
                onPointerEnter={() => setHover(index)}
                onPointerDown={() => setHover(index)}
              />
            </g>
          )
        })}
      </svg>
      <div className="bars__ticks" aria-hidden="true">
        <span>{dayLabel(filled[0]?.day ?? '')}</span>
        <span>Today</span>
      </div>
      <div className="sr-only">
        <table>
          <caption>{label} by day</caption>
          <tbody>
            {filled
              .filter((day) => day[measure] > 0)
              .map((day) => (
                <tr key={day.day}>
                  <th scope="row">{dayLabel(day.day)}</th>
                  <td>{format(day[measure])}</td>
                </tr>
              ))}
          </tbody>
        </table>
      </div>
    </figure>
  )
}
