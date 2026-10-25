import { app } from '../app.svelte'
import { attachmentId } from './attachments'

/** The attachment (file link or image) an event happened on, if any. */
function attachmentAt(event: Event): { id: string; name: string } | null {
  const target = event.target as HTMLElement
  const link = target.closest<HTMLAnchorElement>('a[href^="attachment:"]')
  const linkId = attachmentId(link?.getAttribute('href'))
  if (link && linkId) return { id: linkId, name: link.textContent?.trim() || 'attachment' }
  const img = target.closest<HTMLImageElement>('img[data-attachment]')
  if (img?.dataset.attachment) return { id: img.dataset.attachment, name: img.alt || 'image' }
  return null
}

/**
 * Svelte action for an editor's container: double-click opens an
 * attachment, right-click offers Open / Save as. A single click only places
 * the cursor (and never follows the link).
 */
export function attachmentActions(node: HTMLElement) {
  const onclick = (e: MouseEvent) => {
    if (attachmentAt(e)) e.preventDefault()
  }
  const ondblclick = (e: MouseEvent) => {
    const found = attachmentAt(e)
    if (!found) return
    e.preventDefault()
    e.stopPropagation()
    void app.openAttachment(found.id, found.name)
  }
  const oncontextmenu = (e: MouseEvent) => {
    const found = attachmentAt(e)
    if (!found) return
    e.preventDefault()
    e.stopPropagation()
    app.attachmentMenu = { x: e.clientX, y: e.clientY, ...found }
  }
  node.addEventListener('click', onclick, true)
  node.addEventListener('dblclick', ondblclick, true)
  node.addEventListener('contextmenu', oncontextmenu, true)
  return {
    destroy() {
      node.removeEventListener('click', onclick, true)
      node.removeEventListener('dblclick', ondblclick, true)
      node.removeEventListener('contextmenu', oncontextmenu, true)
    },
  }
}
