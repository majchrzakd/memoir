import type { Extensions } from '@tiptap/core'
import StarterKit from '@tiptap/starter-kit'
import { TaskItem, TaskList } from '@tiptap/extension-list'
import { TableKit } from '@tiptap/extension-table'
import { Markdown } from '@tiptap/markdown'
import {
  ATTACHMENT_SCHEME,
  Attachments,
  attachmentImage,
  type AttachmentUpload,
} from './attachments'
import { SafeParagraph } from './safeParagraph'

export interface EditorOptions {
  /** URL to display an image attachment; tests leave it unset. */
  attachmentUrl?: (id: string) => string
  /** Enables pasting / dropping files. */
  upload?: AttachmentUpload
}

/**
 * The editor's feature set. Notes are stored as Markdown, so every node and
 * mark enabled here must survive a Markdown round-trip — see markdown.test.ts.
 */
export function editorExtensions(options: EditorOptions = {}): Extensions {
  return [
    StarterKit.configure({
      paragraph: false,
      link: {
        openOnClick: false,
        // Links to attachments are kept; other schemes follow TipTap's rules.
        isAllowedUri: (url, { defaultValidate }) =>
          url.startsWith(ATTACHMENT_SCHEME) || defaultValidate(url),
      },
    }),
    SafeParagraph,
    TaskList,
    TaskItem.configure({ nested: true }),
    TableKit,
    attachmentImage(options.attachmentUrl ?? ((id) => ATTACHMENT_SCHEME + id)),
    Attachments.configure({ upload: options.upload ?? null }),
    Markdown,
  ]
}
