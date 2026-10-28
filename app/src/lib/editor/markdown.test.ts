// @vitest-environment jsdom
//
// M0 spike 2: does the editor round-trip Markdown faithfully?
//
// Notes are stored as Markdown and synced with a 3-way text merge, so what
// matters most is *stability*: opening and re-saving a note must not keep
// rewriting it (that would create endless spurious sync revisions), and must
// not change its meaning.

import { Editor, type JSONContent } from '@tiptap/core'
import { afterAll, describe, expect, it } from 'vitest'
import { editorExtensions } from './extensions'

function load(content: string | JSONContent): Editor {
  return new Editor({
    extensions: editorExtensions(),
    content,
    contentType: typeof content === 'string' ? 'markdown' : 'json',
  })
}

function roundTrip(md: string): { md: string; doc: JSONContent } {
  const editor = load(md)
  const result = { md: editor.getMarkdown(), doc: editor.getJSON() }
  editor.destroy()
  return result
}

const samples: Record<string, string> = {
  headings: '# Title\n\n## Section\n\n### Subsection\n\nText.',
  inlineMarks:
    'Some **bold**, *italic*, ~~struck~~, `code` and a [link](https://example.com "Example").',
  bulletListNested: '- one\n- two\n  - two.a\n  - two.b\n- three',
  orderedListStart: '3. third\n4. fourth\n5. fifth',
  taskListNested: '- [ ] todo\n- [x] done\n  - [ ] nested todo\n  - [x] nested done',
  codeBlock: '```rust\nfn main() {\n    println!("# not a heading *nor* bold");\n}\n```',
  blockquote: '> quoted\n>\n> - with a list\n> - inside',
  table: '| Name | Qty |\n| --- | --- |\n| Apples | 3 |\n| Pears | 5 |',
  image: '![A cat](https://example.com/cat.png "Cat")',
  horizontalRule: 'above\n\n---\n\nbelow',
  noteLink:
    'See [the plan](note:01JB7Q2M3N4P5R6S7T8V9W0XYZ) and [**bold** one](note:01JB7Q2M3N4P5R6S7T8V9W0XY0).',
  hardBreak: 'line one  \nline two',
  escaping: 'Literal \\*stars\\*, \\_underscores\\_, \\# hash, \\[brackets\\] and a | pipe.',
  unicode: 'Zażółć gęślą jaźń — 日本語 — 🙂',
  attachmentImage: '![photo.png](attachment:' + 'a'.repeat(64) + ')',
  attachmentFile: 'See [report.pdf](attachment:' + 'b'.repeat(64) + ') for details.',
  listItemParagraphs: '- first paragraph\n\n  second paragraph\n- next item',
  mixedDocument:
    '# Trip\n\nPack:\n\n- [x] passport\n- [ ] charger\n\n> Remember **visa**.\n\n| Day | Plan |\n| --- | --- |\n| 1 | Arrive |\n\n```\nflight LO 123\n```',
}

const report: string[] = []

describe('markdown round-trip', () => {
  for (const [name, md0] of Object.entries(samples)) {
    it(`${name}: stable after one save`, () => {
      const first = roundTrip(md0)
      const second = roundTrip(first.md)

      // Re-saving an already-saved note must be a no-op.
      expect(second.md).toBe(first.md)
      // The saved Markdown must mean the same as what was loaded.
      expect(second.doc).toEqual(first.doc)
      expect(roundTrip(md0).doc).toEqual(first.doc)

      report.push(
        first.md === md0
          ? `  exact     ${name}`
          : `  rewritten ${name}\n${indent(md0, '      in : ')}\n${indent(first.md, '      out: ')}`,
      )
    })
  }

  it('content created in the editor survives save + reload', () => {
    const doc: JSONContent = {
      type: 'doc',
      content: [
        {
          type: 'paragraph',
          content: [
            { type: 'text', text: 'underlined', marks: [{ type: 'underline' }] },
            { type: 'text', text: ' and ' },
            { type: 'text', text: 'bold italic', marks: [{ type: 'bold' }, { type: 'italic' }] },
          ],
        },
      ],
    }
    const editor = load(doc)
    const md = editor.getMarkdown()
    const reloaded = roundTrip(md)
    editor.destroy()

    report.push(`  editor-made marks -> ${JSON.stringify(md)}`)
    const marks = (reloaded.doc.content?.[0]?.content ?? []).flatMap(
      (n) => n.marks?.map((m) => m.type) ?? [],
    )
    expect(marks).toContain('bold')
    expect(marks).toContain('italic')
    report.push(`  underline survives: ${marks.includes('underline')}`)
  })

  it('typed text that looks like Markdown syntax stays plain text', () => {
    const literals = [
      '# not a heading',
      '- not a list',
      '1. not a list',
      '> not a quote',
      '```not code',
      '---',
      '[ ] not a task',
      '| not | a table |',
      '<b>not html</b>',
      '*not italic*',
      '+ not a list',
      '2) not a list',
      '===',
      '    four leading spaces',
      '-5 °C outside',
    ]
    const afterHardBreak = ['===', '# heading?', '- item?', '> quote?']
    const doc: JSONContent = {
      type: 'doc',
      content: [
        ...literals.map((text) => ({ type: 'paragraph', content: [{ type: 'text', text }] })),
        ...afterHardBreak.map((text) => ({
          type: 'paragraph',
          content: [
            { type: 'text', text: 'first line' },
            { type: 'hardBreak' },
            { type: 'text', text },
          ],
        })),
      ],
    }
    const editor = load(doc)
    const original = editor.getJSON()
    const md = editor.getMarkdown()
    editor.destroy()

    report.push(`  literals saved as:\n${indent(md, '      ')}`)
    expect(roundTrip(md).doc).toEqual(original)
  })
})

afterAll(() => {
  console.log(`\nMarkdown round-trip report:\n${report.join('\n')}\n`)
})

function indent(text: string, prefix: string): string {
  return text
    .split('\n')
    .map((l) => prefix + JSON.stringify(l))
    .join('\n')
}
