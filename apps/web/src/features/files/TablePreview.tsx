import { parseTable } from './viewer'

const TABLE_ROWS = 2000

export function TablePreview({
  text,
  separator = ',',
}: {
  text: string
  separator?: ',' | '\t'
}) {
  const rows = parseTable(text, separator)
  const [head, ...body] = rows
  if (!head) return <p className="viewer__cut">This table is empty.</p>
  return (
    <div className="viewer__table">
      <table className="table">
        <thead>
          <tr>
            {head.map((cell, index) => (
              <th key={index}>{cell}</th>
            ))}
          </tr>
        </thead>
        <tbody>
          {body.slice(0, TABLE_ROWS).map((row, at) => (
            <tr key={at}>
              {row.map((cell, index) => (
                <td key={index}>{cell}</td>
              ))}
            </tr>
          ))}
        </tbody>
      </table>
      {body.length > TABLE_ROWS && (
        <p className="viewer__cut">
          Showing {TABLE_ROWS.toLocaleString()} of{' '}
          {body.length.toLocaleString()} rows.
        </p>
      )}
    </div>
  )
}
