import { app, SYNCED_EVENT } from '../app.svelte'
import { attachmentId } from './attachments'
import { linkedNoteId } from './noteLinks'

/** The note a link the event happened on points at, if any. */
function noteLinkAt(event: Event): { id: string; link: HTMLAnchorElement } | null {
  const link = (event.target as HTMLElement).closest<HTMLAnchorElement>('a[href^="note:"]')
  const id = linkedNoteId(link?.getAttribute('href'))
  return link && id ? { id, link } : null
}

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
 * the cursor (and never follows the link). A click on a link to another
 * note opens that note.
 */
export function attachmentActions(node: HTMLElement) {
  const onclick = (e: MouseEvent) => {
    if (attachmentAt(e)) e.preventDefault()
    const note = noteLinkAt(e)
    if (note && e.button === 0) {
      e.preventDefault()
      e.stopPropagation()
      void app.openLinkedNote(note.id)
    }
  }
  // Says where a note link goes (or that the note is gone).
  const onmouseover = (e: MouseEvent) => {
    const note = noteLinkAt(e)
    if (!note) return
    const target = app.byId.get(note.id)
    note.link.title =
      target?.kind === 'note'
        ? `Open “${target.title || 'Untitled'}”`
        : 'The linked note was deleted or is in the trash'
    note.link.classList.toggle('missing', target?.kind !== 'note')
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
  // An image not on this device (yet): a labelled placeholder instead of a
  // broken image, tried again whenever sync brings something in.
  const onerror = (e: Event) => {
    const img = e.target as HTMLElement
    if (!(img instanceof HTMLImageElement) || !img.dataset.attachment) return
    img.classList.add('missing')
    img.title = "This image isn't on this device yet. It appears once it's downloaded."
  }
  const onload = (e: Event) => {
    const img = e.target as HTMLElement
    if (img instanceof HTMLImageElement && img.classList.contains('missing')) {
      img.classList.remove('missing')
      img.removeAttribute('title')
    }
  }
  const retry = () => {
    for (const img of node.querySelectorAll<HTMLImageElement>('img.missing')) {
      const url = new URL(img.src)
      url.searchParams.set('retry', String(Date.now()))
      img.src = url.toString()
    }
  }
  node.addEventListener('click', onclick, true)
  node.addEventListener('mouseover', onmouseover)
  node.addEventListener('dblclick', ondblclick, true)
  node.addEventListener('contextmenu', oncontextmenu, true)
  node.addEventListener('error', onerror, true)
  node.addEventListener('load', onload, true)
  window.addEventListener(SYNCED_EVENT, retry)
  return {
    destroy() {
      node.removeEventListener('click', onclick, true)
      node.removeEventListener('mouseover', onmouseover)
      node.removeEventListener('dblclick', ondblclick, true)
      node.removeEventListener('contextmenu', oncontextmenu, true)
      node.removeEventListener('error', onerror, true)
      node.removeEventListener('load', onload, true)
      window.removeEventListener(SYNCED_EVENT, retry)
    },
  }
}
