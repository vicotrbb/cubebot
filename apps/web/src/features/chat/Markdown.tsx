import type { Token, Tokens } from 'marked'
import { lazy, Suspense, useMemo, type ReactNode } from 'react'
import { CopyButton } from '../../components/primitives'
import { inlineImage, safeHref, tokensOf } from './tokens'
import './markdown.css'

const Diagram = lazy(() =>
  import('../../components/Diagram').then((module) => ({
    default: module.Diagram,
  })),
)
const MathView = lazy(() => import('./Math'))

/** Messages as Markdown, built as React elements: raw HTML in a message stays visible text and
 *  can never run. */
export function Markdown({ text }: { text: string }) {
  const tokens = useMemo(() => tokensOf(text), [text])
  return <div className="md">{blocks(tokens)}</div>
}

function blocks(tokens: Token[]) {
  return tokens.map((token, index) => <Node key={index} token={token} />)
}

function inline(tokens: Token[] | undefined, fallback = ''): ReactNode {
  return tokens ? blocks(tokens) : fallback
}

function Node({ token }: { token: Token }) {
  const render = RENDERERS[token.type]
  return render ? render(token as never) : token.raw
}

type Heading = 'h1' | 'h2' | 'h3' | 'h4' | 'h5' | 'h6'

const RENDERERS: Record<string, (token: never) => ReactNode> = {
  space: () => null,
  hr: () => <hr />,
  br: () => <br />,
  html: (token: Tokens.HTML) => token.raw,
  escape: (token: Tokens.Escape) => token.text,
  text: (token: Tokens.Text) => inline(token.tokens, token.text),
  paragraph: (token: Tokens.Paragraph) => <p>{inline(token.tokens)}</p>,
  heading: (token: Tokens.Heading) => {
    const Tag = `h${token.depth}` as Heading
    return <Tag>{inline(token.tokens)}</Tag>
  },
  strong: (token: Tokens.Strong) => <strong>{inline(token.tokens)}</strong>,
  em: (token: Tokens.Em) => <em>{inline(token.tokens)}</em>,
  del: (token: Tokens.Del) => <del>{inline(token.tokens)}</del>,
  codespan: (token: Tokens.Codespan) => <code>{token.text}</code>,
  blockquote: (token: Tokens.Blockquote) => (
    <blockquote>{blocks(token.tokens)}</blockquote>
  ),
  link: (token: Tokens.Link) => <LinkTo token={token} />,
  image: (token: Tokens.Image) => <ImageOf token={token} />,
  list: (token: Tokens.List) => <List token={token} />,
  code: (token: Tokens.Code) => <Code token={token} />,
  table: (token: Tokens.Table) => <Table token={token} />,
  mathBlock: (token: { text: string }) => <Formula text={token.text} block />,
  mathBlockAlt: (token: { text: string }) => (
    <Formula text={token.text} block />
  ),
  mathInline: (token: { text: string }) => <Formula text={token.text} />,
  mathInlineAlt: (token: { text: string }) => <Formula text={token.text} />,
}

function LinkTo({ token }: { token: Tokens.Link }) {
  const href = safeHref(token.href)
  if (!href) return inline(token.tokens, token.text)
  return (
    <a href={href} target="_blank" rel="noopener noreferrer nofollow">
      {inline(token.tokens, token.text)}
    </a>
  )
}

function ImageOf({ token }: { token: Tokens.Image }) {
  const source = inlineImage(token.href)
  if (source) return <img src={source} alt={token.text} loading="lazy" />
  const href = safeHref(token.href)
  const label = token.text || 'Image'
  if (!href) return label
  return (
    <a href={href} target="_blank" rel="noopener noreferrer nofollow">
      {label} (opens the image)
    </a>
  )
}

function List({ token }: { token: Tokens.List }) {
  const Tag = token.ordered ? 'ol' : 'ul'
  return (
    <Tag
      start={
        token.ordered && token.start !== 1 ? (token.start as number) : undefined
      }
    >
      {token.items.map((item, index) => (
        <li key={index} className={item.task ? 'md__task' : undefined}>
          {item.task && (
            <input
              type="checkbox"
              checked={Boolean(item.checked)}
              disabled
              aria-label={item.checked ? 'Done' : 'Not done'}
            />
          )}
          {blocks(item.tokens.filter((part) => part.type !== 'checkbox'))}
        </li>
      ))}
    </Tag>
  )
}

function Code({ token }: { token: Tokens.Code }) {
  const language = token.lang?.split(/\s/)[0]
  if (language === 'mermaid')
    return (
      <Suspense fallback={<div className="skeleton kb-diagram__wait" />}>
        <div className="md__diagram">
          <Diagram source={token.text} />
        </div>
      </Suspense>
    )
  return (
    <div className="md__code">
      <div className="md__code-head">
        <span>{language || 'text'}</span>
        <CopyButton text={token.text} label="Copy code" />
      </div>
      <pre tabIndex={0}>
        <code>{token.text}</code>
      </pre>
    </div>
  )
}

function Table({ token }: { token: Tokens.Table }) {
  const align = (cell: Tokens.TableCell) => cell.align ?? undefined
  return (
    <div className="md__table" tabIndex={0}>
      <table>
        <thead>
          <tr>
            {token.header.map((cell, index) => (
              <th key={index} style={{ textAlign: align(cell) }}>
                {inline(cell.tokens)}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {token.rows.map((row, at) => (
            <tr key={at}>
              {row.map((cell, index) => (
                <td key={index} style={{ textAlign: align(cell) }}>
                  {inline(cell.tokens)}
                </td>
              ))}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  )
}

function Formula({ text, block = false }: { text: string; block?: boolean }) {
  return (
    <Suspense fallback={<code>{text}</code>}>
      <MathView text={text} block={block} />
    </Suspense>
  )
}
