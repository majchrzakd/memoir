// @vitest-environment jsdom

import { Editor } from '@tiptap/core'
import { afterEach, describe, expect, it, vi } from 'vitest'
import type { AttachmentInfo } from '../api/types'
import { attachmentId, insertAttachments } from './attachments'
import { editorExtensions } from './extensions'

const IMG = 'a'.repeat(64)
const PDF = 'b'.repeat(64)

let editor: Editor
afterEach(() => editor?.destroy())

function file(name: string, type: string, size = 10): File {
  return new File([new Uint8Array(size)], name, { type })
}

function fakeUpload() {
  return vi.fn(async (f: File): Promise<AttachmentInfo> => ({
    id: f.type.startsWith('image/') ? IMG : PDF,
    size: f.size,
    mime: f.type,
  }))
}

describe('attachmentId', () => {
  it('accepts only attachment: URLs with a well-formed id', () => {
    expect(attachmentId(`attachment:${IMG}`)).toBe(IMG)
    expect(attachmentId('attachment:../x')).toBeNull()
    expect(attachmentId('https://example.com/a.png')).toBeNull()
    expect(attachmentId(null)).toBeNull()
  })
})

describe('inserting attachments', () => {
  it('puts images inline and other files as links', async () => {
    editor = new Editor({ extensions: editorExtensions(), content: '' })
    const upload = fakeUpload()
    await insertAttachments(
      editor,
      [file('cat [1].png', 'image/png'), file('report.pdf', 'application/pdf')],
      { upload, onError: (e) => expect.fail(String(e)) },
    )
    const md = editor.getMarkdown()
    expect(md).toContain(`![cat 1.png](attachment:${IMG})`)
    expect(md).toContain(`[report.pdf](attachment:${PDF})`)
    expect(upload).toHaveBeenCalledTimes(2)
  })

  it('refuses files over the size limit without uploading', async () => {
    editor = new Editor({ extensions: editorExtensions(), content: '' })
    const upload = fakeUpload()
    const errors: unknown[] = []
    const huge = file('big.bin', 'application/octet-stream')
    Object.defineProperty(huge, 'size', { value: 200 * 1024 * 1024 })
    await insertAttachments(editor, [huge], { upload, onError: (e) => errors.push(e) })
    expect(upload).not.toHaveBeenCalled()
    expect(String(errors[0])).toContain('larger than 100 MB')
  })
})

describe('displaying attachments', () => {
  it('shows images from the resolved URL but stores attachment:', () => {
    editor = new Editor({
      extensions: editorExtensions({ attachmentUrl: (id) => `http://attachment.localhost/${id}` }),
      content: `![cat.png](attachment:${IMG})`,
      contentType: 'markdown',
    })
    const img = editor.view.dom.querySelector('img')!
    expect(img.getAttribute('src')).toBe(`http://attachment.localhost/${IMG}`)
    expect(img.dataset.attachment).toBe(IMG)
    expect(editor.getMarkdown().trim()).toBe(`![cat.png](attachment:${IMG})`)
  })

  it('maps copied images back to attachment: when pasted as HTML', () => {
    editor = new Editor({ extensions: editorExtensions(), content: '' })
    editor.commands.setContent(
      `<img src="http://attachment.localhost/${IMG}" data-attachment="${IMG}" alt="x">`,
    )
    expect(editor.getMarkdown().trim()).toBe(`![x](attachment:${IMG})`)
  })
})

describe('pasting', () => {
  function paste(files: File[], text = ''): boolean {
    const clipboardData = {
      files,
      getData: (type: string) => (type === 'text/plain' ? text : ''),
    } as unknown as DataTransfer
    const event = new Event('paste') as ClipboardEvent
    Object.defineProperty(event, 'clipboardData', { value: clipboardData })
    return !!editor.view.someProp('handlePaste', (f) =>
      f(editor.view, event, editor.state.doc.slice(0)),
    )
  }

  it('attaches pasted files, but prefers text when there is any', () => {
    const upload = fakeUpload()
    editor = new Editor({
      extensions: editorExtensions({ upload: { upload, onError: () => {} } }),
      content: '',
    })
    expect(paste([file('shot.png', 'image/png')])).toBe(true)
    expect(upload).toHaveBeenCalledTimes(1)
    // e.g. copying from Word: text plus a picture of it.
    expect(paste([file('word.png', 'image/png')], 'some copied text')).toBe(false)
    expect(paste([])).toBe(false)
    expect(upload).toHaveBeenCalledTimes(1)
  })
})
