// @vitest-environment jsdom
//
// Links between notes: `[[` opens the picker, and a chosen note is inserted
// as a `note:<id>` link that typing doesn't extend.

import { Editor } from '@tiptap/core'
import { afterEach, describe, expect, it } from 'vitest'
import { editorExtensions } from './extensions'
import { insertNoteLink, linkedNoteId } from './noteLinks'

const ID = '01JB7Q2M3N4P5R6S7T8V9W0XYZ'
let editor: Editor

afterEach(() => editor?.destroy())

function type(text: string) {
  for (const ch of text) {
    const { from, to } = editor.state.selection
    const handled = editor.view.someProp('handleTextInput', (f) =>
      f(editor.view, from, to, ch, () => editor.state.tr.insertText(ch, from, to)),
    )
    if (!handled) editor.view.dispatch(editor.state.tr.insertText(ch, from, to))
  }
}

describe('note links', () => {
  it('[[ asks for a note, then the link is inserted at the cursor', () => {
    let asked = 0
    editor = new Editor({
      extensions: editorExtensions({ pickNoteLink: () => asked++ }),
      content: '',
    })
    editor.commands.focus('start')
    type('See [[')
    expect(asked).toBe(1)
    expect(editor.getText()).toBe('See ')

    insertNoteLink(editor, ID, 'The plan')
    // Text arriving in one piece (some keyboards, automation) too.
    const { from } = editor.state.selection
    editor.view.someProp('handleTextInput', (f) =>
      f(editor.view, from, from, 'x [[', () => editor.state.tr.insertText('x [[', from)),
    )
    expect(asked).toBe(2)
    editor.commands.deleteRange({ from, to: editor.state.selection.from })
    type(' today')
    expect(editor.getMarkdown()).toBe(`See [The plan](note:${ID}) today`)
  })

  it('a single [ and [[ in code stay text', () => {
    let asked = 0
    editor = new Editor({
      extensions: editorExtensions({ pickNoteLink: () => asked++ }),
      content: '',
    })
    editor.commands.focus('start')
    type('a [b] ')
    editor.commands.toggleCode()
    type('x[[y')
    expect(asked).toBe(0)
    expect(editor.getMarkdown()).toBe('a \\[b\\] `x[[y`')
  })

  it('links the selected text', () => {
    editor = new Editor({ extensions: editorExtensions(), content: 'read this first' })
    editor.commands.setTextSelection({ from: 6, to: 10 })
    insertNoteLink(editor, ID, 'Ignored')
    expect(editor.getMarkdown()).toBe(`read [this](note:${ID}) first`)
  })

  it('recognises note links', () => {
    expect(linkedNoteId(`note:${ID}`)).toBe(ID)
    expect(linkedNoteId('note:../etc')).toBeNull()
    expect(linkedNoteId('https://example.com')).toBeNull()
    expect(linkedNoteId(null)).toBeNull()
  })
})
