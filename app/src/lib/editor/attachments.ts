// Attachments in the editor. A note refers to an attachment by
// `attachment:<id>` — images as `![name](attachment:<id>)`, other files as
// links `[name](attachment:<id>)`. The document and the Markdown always keep
// that form; only what is drawn on screen uses the URL the backend serves.

import { Extension, mergeAttributes, type Editor, type JSONContent } from '@tiptap/core'
import Image from '@tiptap/extension-image'
import { Plugin, PluginKey, Selection } from '@tiptap/pm/state'
import { MAX_ATTACHMENT_SIZE, type AttachmentInfo } from '../api/types'

export const ATTACHMENT_SCHEME = 'attachment:'

/** The attachment id in `attachment:<id>`, or null for any other URL. */
export function attachmentId(url: string | null | undefined): string | null {
  if (!url?.startsWith(ATTACHMENT_SCHEME)) return null
  const id = url.slice(ATTACHMENT_SCHEME.length)
  return /^[0-9a-f]{64}$/.test(id) ? id : null
}

/** Image node showing `attachment:` images through `resolve(id)`. */
export function attachmentImage(resolve: (id: string) => string) {
  return Image.extend({
    addAttributes() {
      return {
        ...this.parent?.(),
        // Copy & paste inside the editor carries the displayed URL; map it
        // back so the document never stores it.
        src: {
          default: null,
          parseHTML: (el) => {
            const id = el.getAttribute('data-attachment')
            return id ? ATTACHMENT_SCHEME + id : el.getAttribute('src')
          },
        },
      }
    },
    renderHTML({ HTMLAttributes }) {
      const id = attachmentId(HTMLAttributes.src)
      return [
        'img',
        mergeAttributes(
          this.options.HTMLAttributes,
          HTMLAttributes,
          id ? { src: resolve(id), 'data-attachment': id, draggable: 'false' } : {},
        ),
      ]
    },
  })
}

export interface AttachmentUpload {
  upload: (file: File) => Promise<AttachmentInfo>
  onError: (error: unknown) => void
}

/** Uploads files and inserts them: images inline, other files as links. */
export async function insertAttachments(
  editor: Editor,
  files: File[],
  { upload, onError }: AttachmentUpload,
  pos?: number,
) {
  for (const file of files) {
    const name = file.name || 'attachment'
    if (file.size > MAX_ATTACHMENT_SIZE) {
      onError(`"${name}" is larger than ${MAX_ATTACHMENT_SIZE / 1024 / 1024} MB`)
      continue
    }
    try {
      const info = await upload(file)
      if (editor.isDestroyed) return
      const src = ATTACHMENT_SCHEME + info.id
      const content: JSONContent[] = info.mime.startsWith('image/')
        ? [{ type: 'image', attrs: { src, alt: name.replace(/[[\]]/g, '') } }]
        : [
            { type: 'text', text: name, marks: [{ type: 'link', attrs: { href: src } }] },
            { type: 'text', text: ' ' },
          ]
      const chain = editor.chain().focus()
      ;(pos === undefined
        ? chain.insertContent(content)
        : chain.insertContentAt(pos, content)
      ).run()
      // An inserted image ends up selected; continue after it, or the next
      // file would replace it.
      editor.commands.command(({ tr }) => {
        tr.setSelection(Selection.near(tr.doc.resolve(tr.selection.to), 1))
        return true
      })
      pos = undefined
    } catch (e) {
      onError(e)
    }
  }
}

/** Pasting or dropping files into the editor attaches them. */
export const Attachments = Extension.create<{ upload: AttachmentUpload | null }>({
  name: 'attachments',

  addOptions() {
    return { upload: null }
  },

  addProseMirrorPlugins() {
    const upload = this.options.upload
    const editor = this.editor
    if (!upload) return []
    return [
      new Plugin({
        key: new PluginKey('attachments'),
        props: {
          handlePaste: (_view, event) => {
            const data = event.clipboardData
            const files = [...(data?.files ?? [])]
            // Office apps put a picture of the copied text on the clipboard
            // too; when there is text, paste the text.
            if (!files.length || data?.getData('text/plain').trim()) return false
            void insertAttachments(editor, files, upload)
            return true
          },
          handleDrop: (view, event) => {
            const files = [...(event.dataTransfer?.files ?? [])]
            if (!files.length) return false
            event.preventDefault()
            const pos = view.posAtCoords({ left: event.clientX, top: event.clientY })?.pos
            void insertAttachments(editor, files, upload, pos)
            return true
          },
        },
      }),
    ]
  },
})
