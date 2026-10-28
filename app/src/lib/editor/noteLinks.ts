import { Extension, InputRule, type Editor } from '@tiptap/core'
import { NOTE_LINK_SCHEME, type Id } from '../api/types'

/** The note a `note:<id>` link points at, if `href` is one. */
export function linkedNoteId(href: string | null | undefined): Id | null {
  if (!href?.startsWith(NOTE_LINK_SCHEME)) return null
  const id = href.slice(NOTE_LINK_SCHEME.length)
  return /^[0-9A-Za-z]{26}$/.test(id) ? id : null
}

/**
 * Links the selection to a note, or (with nothing selected) inserts the
 * note's title as a link. Typing after it continues as plain text.
 */
export function insertNoteLink(editor: Editor, id: Id, title: string) {
  const href = NOTE_LINK_SCHEME + id
  // At once (focus() waits a frame), so the next keystroke lands in the note.
  editor.view.focus()
  if (!editor.state.selection.empty) {
    editor.chain().focus().extendMarkRange('link').setLink({ href }).run()
    return
  }
  editor
    .chain()
    .focus()
    .insertContent({
      type: 'text',
      text: title || 'Untitled',
      marks: [{ type: 'link', attrs: { href } }],
    })
    .unsetMark('link')
    .run()
}

export interface NoteLinkOptions {
  /** Called when `[[` is typed (the brackets are removed first). */
  onTrigger: ((editor: Editor) => void) | null
}

/** Typing `[[` asks for a note to link to, as in many note apps. */
export const NoteLinkTrigger = Extension.create<NoteLinkOptions>({
  name: 'noteLinkTrigger',

  addOptions() {
    return { onTrigger: null }
  },

  addInputRules() {
    const onTrigger = this.options.onTrigger
    if (!onTrigger) return []
    const editor = this.editor
    return [
      new InputRule({
        find: /\[\[$/,
        handler: ({ range, commands }) => {
          commands.deleteRange(range)
          onTrigger(editor)
        },
      }),
    ]
  },
})
