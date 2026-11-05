// @ts-expect-error bun provides this at test time
import { describe, expect, test } from 'bun:test'
import { renderToStaticMarkup } from 'react-dom/server'
import { Markdown } from './Markdown'

describe('Markdown', () => {
  test('a code fence named like an inherited property is only code', () => {
    for (const language of ['constructor', '__proto__', 'toString']) {
      const html = renderToStaticMarkup(
        <Markdown text={`\`\`\`${language}\nsecret\n\`\`\``} />,
      )
      expect(html).toContain('secret')
    }
  })
})
