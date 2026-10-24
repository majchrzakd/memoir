// @vitest-environment jsdom
//
// Markdown-style shortcuts typed into the editor ("## " → heading, …) must
// keep working with Memoir's extension set (e.g. the SafeParagraph swap).

import { Editor } from '@tiptap/core'
import { afterEach, describe, expect, it } from 'vitest'
import { editorExtensions } from './extensions'

let editor: Editor

afterEach(() => editor?.destroy())

/** Types text the way a keyboard does: through the editor's text-input handler. */
function type(text: string) {
  for (const ch of text) {
    const { from, to } = editor.state.selection
    const handled = editor.view.someProp('handleTextInput', (f) =>
      f(editor.view, from, to, ch, () => editor.state.tr.insertText(ch, from, to)),
    )
    if (!handled) editor.view.dispatch(editor.state.tr.insertText(ch, from, to))
  }
}

function typed(text: string): string {
  editor = new Editor({ extensions: editorExtensions(), content: '' })
  editor.commands.focus('start')
  type(text)
  return editor.getMarkdown()
}

describe('typing shortcuts', () => {
  it.each([
    ['## Title', '## Title'],
    ['- item', '- item'],
    ['1. first', '1. first'],
    ['[ ] todo', '- [ ] todo'],
    ['> quote', '> quote'],
    ['**bold** ', '**bold** '],
    ['`code` ', '`code` '],
  ])('%j becomes %j', (input, markdown) => {
    expect(typed(input).trim()).toBe(markdown.trim())
  })

  it('``` starts a code block', () => {
    typed('``` ')
    expect(editor.isActive('codeBlock')).toBe(true)
  })

  it('--- inserts a divider', () => {
    typed('---')
    expect(editor.getJSON().content?.some((n) => n.type === 'horizontalRule')).toBe(true)
  })
})
