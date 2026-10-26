// Application state and the actions the UI performs. Components read from
// `app` and call its methods; all persistence goes through `api`.

import { SvelteSet } from 'svelte/reactivity'
import { api, type Id, type Placement, type Status, type SyncStatus, type TreeItem } from './api'
import type { EditorOptions } from './editor/extensions'
import { ancestorIds, childrenMap, visibleRows } from './tree'

type Screen = 'loading' | Status
type View = 'note' | 'trash' | 'settings'

const EXPANDED_KEY = 'memoir.expanded'
const OPEN_NOTE_KEY = 'memoir.openNote'
const THREAD_LAYOUT_KEY = 'memoir.threadLayout'

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

  sidebarTab = $state<'notes' | 'later'>('notes')

  /** Where a note's thread is shown: under the note, or in a side column. */
  threadLayout = $state<'below' | 'side'>(load(THREAD_LAYOUT_KEY, 'below'))
  /** Reply to scroll to once its thread is shown (set by search). */
  focusReplyId = $state<Id | null>(null)
  /** Unsent replies per note. Memory only: browser storage is not encrypted. */
  replyDrafts = new Map<Id, string>()
  /** Bumped whenever the Later list may have changed. */
  laterVersion = $state(0)

  sync = $state<SyncStatus>({ state: 'off', pending: 0, last_synced_at: null, message: null })
  /** Bumped when sync brought changes from other devices; `syncedIds` says which. */
  syncVersion = $state(0)
  syncedIds = new Set<Id>()

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
    await this.listenToSync()
    await this.refreshTree()
    if (this.openNoteId && this.byId.get(this.openNoteId)?.kind !== 'note') this.openNoteId = null
    this.selectedId = this.openNoteId
    this.screen = 'unlocked'
  }

  private syncListening = false

  private async listenToSync() {
    if (this.syncListening) return
    this.syncListening = true
    await api.onSyncStatus((status) => (this.sync = status))
    await api.onSyncChanged((ids) => void this.syncedChanges(ids))
    this.sync = await api.syncStatus()
  }

  /** Other devices changed these notes or folders. */
  private async syncedChanges(ids: Id[]) {
    this.syncedIds = new Set(ids)
    this.syncVersion++
    this.laterChanged()
    await this.refreshTree().catch((e) => this.fail(e))
  }

  syncNow() {
    void api.syncNow().catch((e) => this.fail(e))
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

  /** Updates the tree (title, edit time) after the editor saved a note. */
  noteSaved(id: Id, title: string, updatedAt: number) {
    this.items = this.items.map((i) => (i.id === id ? { ...i, title, updated_at: updatedAt } : i))
    this.laterChanged()
  }

  // ---- Attachments ---------------------------------------------------------

  /** Right-click menu for an attachment in a note. */
  attachmentMenu = $state<{ x: number; y: number; id: string; name: string } | null>(null)

  /** Editor options shared by notes, replies and the reply box. */
  editorOptions(): EditorOptions {
    return {
      attachmentUrl: (id) => api.attachmentUrl(id),
      upload: { upload: (file) => api.addAttachment(file), onError: (e) => this.fail(e) },
    }
  }

  async openAttachment(id: string, name: string) {
    await this.run(() => api.openAttachment(id, name))
  }

  async saveAttachment(id: string, name: string) {
    await this.run(async () => {
      await api.saveAttachment(id, name)
    })
  }

  /** Updates a note's reply count in the tree. */
  replyCountChanged(id: Id, replies: number) {
    this.items = this.items.map((i) => (i.id === id ? { ...i, replies } : i))
  }

  setThreadLayout(layout: 'below' | 'side') {
    this.threadLayout = layout
    persist(THREAD_LAYOUT_KEY, layout)
  }

  /** Tells the Later list to reload. */
  laterChanged() {
    this.laterVersion++
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
