// @vitest-environment jsdom
//
// Markdown written by the Evernote importer (crates/memoir-core/src/import/
// enml.rs) must be exactly what the editor writes back: otherwise merely
// opening an imported note would rewrite it and create a sync revision.
// The samples are outputs from the importer's own tests.

import { Editor } from '@tiptap/core'
import { describe, expect, it } from 'vitest'
import { editorExtensions } from './extensions'

function resave(md: string): string {
  const editor = new Editor({
    extensions: editorExtensions(),
    content: md,
    contentType: 'markdown',
  })
  const out = editor.getMarkdown()
  editor.destroy()
  return out
}

const id = 'a'.repeat(64)

const imported: Record<string, string> = {
  formatting: 'Hello **bold** and *it*, ~~gone~~, `x*y`\n\nSecond line',
  styledSpans: '**Loud** *soft*',
  hardBreak: 'one  \ntwo',
  headingsAndRule: '# Title\n\n### Small\n\n---',
  escapes:
    '\\# not a heading\n\n1\\. not a list\n\n\\- nor this\n\na \\*b\\* \\[c\\] &lt;d&gt; \\~e\\~ | f\\_g',
  links: '[Example](https://example.com/a%20b) [https://x.org](https://x.org)',
  bulletNested: '- one\n- two\n  - two.a',
  ordered: '3. third\n4. fourth',
  checklist: '- [x] Milk\n- [ ] Bread\n\nAfter',
  code: '```\nfn main() {\n  let x = 1; // *keep*\n}\n```',
  quote: '> quoted\n>\n> more',
  // Inside a note, as the editor writes a table that isn't the only thing.
  table:
    'Fruit:\n\n\n| **Name**   | Qty |\n| ---------- | --- |\n| Apples red | 3   |\n| Pears      |     |\n\n\nAfter',
  attachments: `Day 1\n\n![colosseum [1].png](attachment:${id})\n\n[report \\_v2\\_.pdf](attachment:${id})`,
  encrypted: '*\\[Encrypted text from Evernote, not imported\\]*',
}

describe('imported Markdown is what the editor writes', () => {
  for (const [name, md] of Object.entries(imported)) {
    it(name, () => {
      expect(resave(md)).toBe(md)
    })
  }
})
