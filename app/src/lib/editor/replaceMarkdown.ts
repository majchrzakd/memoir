import type { Editor } from '@tiptap/core'

/**
 * Replaces an editor's content with newer text (merged with another
 * device's changes), keeping the cursor about where it was. Doesn't count as
 * an edit, so it doesn't trigger a save.
 */
export function replaceMarkdown(editor: Editor, markdown: string) {
  const { from, to } = editor.state.selection
  editor.commands.setContent(markdown, { contentType: 'markdown', emitUpdate: false })
  const size = editor.state.doc.content.size
  editor.commands.setTextSelection({ from: Math.min(from, size), to: Math.min(to, size) })
}
