// Application state and the actions the UI performs. Components read from
// `app` and call its methods; all persistence goes through `api`.

import { SvelteSet } from 'svelte/reactivity'
import { api, type Id, type Placement, type Status, type TreeItem } from './api'
import { ancestorIds, childrenMap, visibleRows } from './tree'

type Screen = 'loading' | Status
type View = 'note' | 'trash'

const EXPANDED_KEY = 'memoir.expanded'
const OPEN_NOTE_KEY = 'memoir.openNote'

function load<T>(key: string, fallback: T): T {
  try {
    const raw = localStorage.getItem(key)
    return raw ? (JSON.parse(raw) as T) : fallback
  } catch {
    return fallback
  }
}

function persist(key: string, value: unknown) {
  try {
    localStorage.setItem(key, JSON.stringify(value))
  } catch {
    // UI preferences only; losing them is harmless.
  }
}

/** Readable text for an error from the backend ("wrong passphrase" → "Wrong passphrase"). */
export function errorMessage(e: unknown): string {
  const text = typeof e === 'string' ? e : e instanceof Error ? e.message : String(e)
  return text.charAt(0).toUpperCase() + text.slice(1)
}

interface ConfirmRequest {
  message: string
  confirmLabel: string
  resolve: (ok: boolean) => void
}

class App {
  screen = $state<Screen>('loading')
  view = $state<View>('note')

  items = $state.raw<TreeItem[]>([])
  expanded = new SvelteSet<Id>(load<Id[]>(EXPANDED_KEY, []))
  selectedId = $state<Id | null>(null)
  openNoteId = $state<Id | null>(load<Id | null>(OPEN_NOTE_KEY, null))
  renamingId = $state<Id | null>(null)

  searchOpen = $state(false)
  confirmRequest = $state<ConfirmRequest | null>(null)
  error = $state<string | null>(null)

  byId = $derived(new Map(this.items.map((i) => [i.id, i])))
  rows = $derived(visibleRows(childrenMap(this.items), this.expanded))

  /** Saves edits waiting for autosave; set by the editor. */
  flushEditor: () => Promise<void> = async () => {}

  async start() {
    try {
      const status = await api.status()
      // A setup interrupted before the recovery key was confirmed starts over.
      this.screen = status === 'confirm_recovery_key' ? 'first_run' : status
      if (this.screen === 'unlocked') await this.enter()
    } catch (e) {
      this.fail(e)
    }
  }

  /** Called once the vault is unlocked. */
  async enter() {
    await this.refreshTree()
    if (this.openNoteId && this.byId.get(this.openNoteId)?.kind !== 'note') this.openNoteId = null
    this.selectedId = this.openNoteId
    this.screen = 'unlocked'
  }

  fail(e: unknown) {
    this.error = errorMessage(e)
  }

  async refreshTree() {
    this.items = await api.tree()
  }

  // ---- Tree state --------------------------------------------------------

  toggle(id: Id, open = !this.expanded.has(id)) {
    if (open) this.expanded.add(id)
    else this.expanded.delete(id)
    persist(EXPANDED_KEY, [...this.expanded])
  }

  /** Expands the folders containing `id` and selects it. */
  reveal(id: Id) {
    for (const a of ancestorIds(this.byId, id)) this.expanded.add(a)
    persist(EXPANDED_KEY, [...this.expanded])
    this.selectedId = id
  }

  /** Folder that new items go into, based on the selection. */
  currentFolder(): Id | null {
    const sel = this.selectedId ? this.byId.get(this.selectedId) : undefined
    if (!sel) return null
    return sel.kind === 'folder' ? sel.id : sel.parent_id
  }

  // ---- Actions -----------------------------------------------------------

  async openNote(id: Id) {
    if (id === this.openNoteId && this.view === 'note') return
    await this.flushEditor()
    this.view = 'note'
    this.openNoteId = id
    this.reveal(id)
    persist(OPEN_NOTE_KEY, id)
  }

  async newNote(parent: Id | null = this.currentFolder()) {
    await this.run(async () => {
      const id = await api.createNote({ parent, position: { type: 'first' } }, '')
      await this.refreshTree()
      await this.openNote(id)
    })
  }

  async newFolder(parent: Id | null = this.currentFolder()) {
    await this.run(async () => {
      const id = await api.createFolder({ parent, position: { type: 'last' } }, 'New folder')
      await this.refreshTree()
      this.reveal(id)
      this.renamingId = id
    })
  }

  async rename(id: Id, title: string) {
    this.renamingId = null
    const item = this.byId.get(id)
    if (!item || item.title === title) return
    if (id === this.openNoteId) await this.flushEditor()
    await this.run(async () => {
      await api.rename(id, title)
      await this.refreshTree()
    })
  }

  /** Updates a title in the tree after the editor saved it. */
  titleSaved(id: Id, title: string) {
    if (this.byId.get(id)?.title === title) return
    this.items = this.items.map((i) => (i.id === id ? { ...i, title } : i))
  }

  async move(id: Id, placement: Placement) {
    await this.run(async () => {
      await api.moveItem(id, placement)
      await this.refreshTree()
      this.reveal(id)
    })
  }

  async trash(id: Id) {
    const item = this.byId.get(id)
    if (!item) return
    if (
      this.openNoteId &&
      (this.openNoteId === id || ancestorIds(this.byId, this.openNoteId).includes(id))
    ) {
      await this.flushEditor()
      this.openNoteId = null
    }
    // Select a neighbour so keyboard users can keep going.
    const index = this.rows.findIndex((r) => r.item.id === id)
    await this.run(async () => {
      await api.moveToTrash(id)
      await this.refreshTree()
      this.selectedId = this.rows[Math.min(index, this.rows.length - 1)]?.item.id ?? null
    })
  }

  /** Runs an action, reporting failures instead of throwing. */
  async run(action: () => Promise<void>) {
    try {
      await action()
    } catch (e) {
      this.fail(e)
      await this.refreshTree().catch(() => {})
    }
  }

  confirm(message: string, confirmLabel: string): Promise<boolean> {
    return new Promise((resolve) => {
      this.confirmRequest = {
        message,
        confirmLabel,
        resolve: (ok) => {
          this.confirmRequest = null
          resolve(ok)
        },
      }
    })
  }
}

export const app = new App()
