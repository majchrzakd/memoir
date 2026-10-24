import type { Extensions } from '@tiptap/core'
import StarterKit from '@tiptap/starter-kit'
import { TaskItem, TaskList } from '@tiptap/extension-list'
import { TableKit } from '@tiptap/extension-table'
import Image from '@tiptap/extension-image'
import { Markdown } from '@tiptap/markdown'
import { SafeParagraph } from './safeParagraph'

/**
 * The editor's feature set. Notes are stored as Markdown, so every node and
 * mark enabled here must survive a Markdown round-trip — see markdown.test.ts.
 */
export function editorExtensions(): Extensions {
  return [
    StarterKit.configure({
      paragraph: false,
      link: { openOnClick: false },
    }),
    SafeParagraph,
    TaskList,
    TaskItem.configure({ nested: true }),
    TableKit,
    Image,
    Markdown,
  ]
}
