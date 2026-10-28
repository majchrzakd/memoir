// Application state and the actions the UI performs. Components read from
// `app` and call its methods; all persistence goes through `api`.

import { SvelteSet } from 'svelte/reactivity'
import {
  api,
  type Id,
  type Placement,
  type Status,
  type SyncStatus,
  type TreeItem,
  type UpdateInfo,
  type WorkspaceList,
} from './api'
import type { Editor } from '@tiptap/core'
import type { EditorOptions } from './editor/extensions'
import { isIOS } from './platform'
import { ancestorIds, childrenMap, visibleRows } from './tree'

type Screen = 'loading' | Status
type View = 'note' | 'trash' | 'settings'

/** Window event: sync brought changes (e.g. images that were missing). */
export const SYNCED_EVENT = 'memoir-synced'

const EXPANDED_KEY = 'memoir.expanded'
const AUTO_LOCK_KEY = 'memoir.autoLockMinutes'
const OPEN_NOTE_KEY = 'memoir.openNote'
/** The workspace in the app's data folder itself (app/src-tauri/src/workspaces.rs). */
const FIRST_WORKSPACE = 'default'

/** Per-workspace preference keys; the first workspace keeps the old names. */
function workspaceKey(key: string, workspace: string): string {
  return workspace === FIRST_WORKSPACE ? key : `${key}/${workspace}`
}
const WORKSPACE_KEYS = [EXPANDED_KEY, OPEN_NOTE_KEY]
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

/** Drops a removed workspace's preferences. */
function forgetWorkspace(workspace: string) {
  try {
    for (const key of WORKSPACE_KEYS) localStorage.removeItem(workspaceKey(key, workspace))
  } catch {
    // As above.
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
  /** Phone layout: the notes list is shown instead of the note, trash or settings. */
  listShown = $state(true)
  /** iOS, in the background: everything is hidden from the app switcher. */
  covered = $state(false)

  /** Workspaces on this device; each is a separate notebook. */
  workspaces = $state<WorkspaceList>({ active: FIRST_WORKSPACE, workspaces: [] })
  /** The workspace to go back to if adding one is cancelled. */
  previousWorkspace = $state<string | null>(null)
  workspaceName = $derived(
    this.workspaces.workspaces.find((w) => w.id === this.workspaces.active)?.name ?? 'Memoir',
  )

  items = $state.raw<TreeItem[]>([])
  expanded = new SvelteSet<Id>()
  selectedId = $state<Id | null>(null)
  openNoteId = $state<Id | null>(null)
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

  /** A newer version, ready to install (null: none known). */
  update = $state<UpdateInfo | null>(null)
  /** The user closed the update notice for this session. */
  updateDismissed = $state(false)

  /** Lock after this many idle minutes (0: never). A per-device preference. */
  autoLockMinutes = $state<number>(load(AUTO_LOCK_KEY, 0))
  private lastActivity = Date.now()

  searchOpen = $state(false)
  /** The item "Move to…" is choosing a folder for. */
  movingId = $state<Id | null>(null)
  confirmRequest = $state<ConfirmRequest | null>(null)
  error = $state<string | null>(null)

  byId = $derived(new Map(this.items.map((i) => [i.id, i])))
  rows = $derived(visibleRows(childrenMap(this.items), this.expanded))

  /** Saves edits waiting for autosave; set by the editor. */
  flushEditor: () => Promise<void> = async () => {}

  async start() {
    try {
      await this.loadWorkspaces()
      const status = await api.status()
      // A setup interrupted before the recovery key was confirmed starts over.
      this.screen = status === 'confirm_recovery_key' ? 'first_run' : status
      if (this.screen === 'unlocked') await this.enter()
    } catch (e) {
      this.fail(e)
    }
  }

  private async loadWorkspaces() {
    this.workspaces = await api.workspaces()
    const ws = this.workspaces.active
    this.expanded.clear()
    for (const id of load<Id[]>(workspaceKey(EXPANDED_KEY, ws), [])) this.expanded.add(id)
    this.openNoteId = load<Id | null>(workspaceKey(OPEN_NOTE_KEY, ws), null)
  }

  private persistExpanded() {
    persist(workspaceKey(EXPANDED_KEY, this.workspaces.active), [...this.expanded])
  }

  /** Called once the vault is unlocked. */
  async enter() {
    this.previousWorkspace = null
    this.noteActivity()
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
    await api.onUpdateAvailable((update) => (this.update = update))
    this.update = await api.availableUpdate()
    // iOS has no "closing": leaving the app is the moment to save and push
    // (it may be suspended a few seconds later), and to cover the notes so
    // the app switcher's snapshot doesn't show them. Coming back pulls, or
    // locks instead if the auto-lock time passed while timers were suspended.
    document.addEventListener('visibilitychange', () => {
      this.covered = isIOS && document.hidden
      if (this.screen !== 'unlocked') return
      if (document.hidden) {
        this.flushEditor().then(
          () => this.syncNow(),
          (e) => this.fail(e),
        )
      } else if (!this.checkIdle()) {
        this.syncNow()
      }
    })
  }

  /** Other devices changed these notes or folders. */
  private async syncedChanges(ids: Id[]) {
    this.syncedIds = new Set(ids)
    this.syncVersion++
    window.dispatchEvent(new Event(SYNCED_EVENT))
    this.laterChanged()
    await this.refreshTree().catch((e) => this.fail(e))
  }

  /** Saves everything, installs the update and restarts. */
  async installUpdate() {
    await this.flushEditor()
    await api.installUpdate()
  }

  syncNow() {
    void api.syncNow().catch((e) => this.fail(e))
  }

  /** Saves, then closes the notebook: back to the unlock screen. */
  async lock() {
    if (this.screen !== 'unlocked') return
    await this.flushEditor().catch(() => {})
    try {
      await api.lock()
    } catch (e) {
      this.fail(e)
      return
    }
    this.closeNotebook()
    this.screen = 'locked'
  }

  /** Nothing from the notebook stays in memory. */
  private closeNotebook() {
    this.items = []
    this.replyDrafts.clear()
    this.searchOpen = false
    this.movingId = null
    this.confirmRequest?.resolve(false)
    this.attachmentMenu = null
    this.linkingEditor = null
    this.newWorkspaceOpen = false
    this.view = 'note'
    this.listShown = true
  }

  // ---- Workspaces ----------------------------------------------------------

  /** Saves and closes the open workspace, then opens `id`. */
  async switchWorkspace(id: string) {
    if (id === this.workspaces.active || this.screen === 'loading') return
    if (!(await this.saved())) return
    await this.openWorkspace(() => api.switchWorkspace(id))
  }

  /**
   * Adds a workspace and shows its first-run screen (new notebook or one from
   * Drive). The "New workspace" dialog stays open if the open note can't be saved.
   */
  async addWorkspace(name: string) {
    if (this.screen === 'loading' || !(await this.saved())) return
    this.newWorkspaceOpen = false
    const previous = this.workspaces.active
    await this.openWorkspace(async () => {
      await api.addWorkspace(name)
      return api.status()
    })
    if (this.screen === 'first_run') this.previousWorkspace = previous
  }

  /** Gives up on a workspace being added: back to the one before, and it's removed. */
  async cancelNewWorkspace() {
    const abandoned = this.workspaces.active
    const back =
      this.previousWorkspace ?? this.workspaces.workspaces.find((w) => w.id !== abandoned)?.id
    if (!back) return
    this.previousWorkspace = null
    await this.openWorkspace(() => api.switchWorkspace(back))
    await this.removeWorkspace(abandoned)
  }

  async renameWorkspace(id: string, name: string) {
    try {
      await api.renameWorkspace(id, name)
      this.workspaces = await api.workspaces()
    } catch (e) {
      this.fail(e)
    }
  }

  async removeWorkspace(id: string) {
    try {
      await api.removeWorkspace(id)
      forgetWorkspace(id)
      this.workspaces = await api.workspaces()
    } catch (e) {
      this.fail(e)
    }
  }

  /** Saves the open note before its notebook closes; false (error shown) if that failed. */
  private async saved(): Promise<boolean> {
    try {
      await this.flushEditor()
      return true
    } catch (e) {
      this.fail(e)
      return false
    }
  }

  private async openWorkspace(open: () => Promise<Status>) {
    this.closeNotebook()
    this.screen = 'loading'
    try {
      const status = await open()
      await this.loadWorkspaces()
      this.screen = status === 'confirm_recovery_key' ? 'first_run' : status
      if (this.screen === 'unlocked') await this.enter()
    } catch (e) {
      this.fail(e)
      // Show whichever workspace is open now.
      await this.start()
    }
  }

  setAutoLock(minutes: number) {
    this.autoLockMinutes = minutes
    persist(AUTO_LOCK_KEY, minutes)
    this.noteActivity()
  }

  /** Keyboard, mouse or touch use; resets the auto-lock countdown. */
  noteActivity() {
    this.lastActivity = Date.now()
  }

  /** Called every few seconds: locks once idle long enough (and says so). */
  checkIdle(now = Date.now()): boolean {
    const minutes = this.autoLockMinutes
    if (minutes > 0 && this.screen === 'unlocked' && now - this.lastActivity >= minutes * 60_000) {
      void this.lock()
      return true
    }
    return false
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
    this.persistExpanded()
  }

  /** Expands the folders containing `id` and selects it. */
  reveal(id: Id) {
    for (const a of ancestorIds(this.byId, id)) this.expanded.add(a)
    this.persistExpanded()
    this.selectedId = id
  }

  /** Folder that new items go into, based on the selection. */
  currentFolder(): Id | null {
    const sel = this.selectedId ? this.byId.get(this.selectedId) : undefined
    if (!sel) return null
    return sel.kind === 'folder' ? sel.id : sel.parent_id
  }

  // ---- Actions -----------------------------------------------------------

  /** Shows the trash or settings (on a phone, instead of the list). */
  show(view: View) {
    this.view = view
    this.listShown = false
  }

  /** Phone layout: back to the notes list. */
  back() {
    this.listShown = true
  }

  async openNote(id: Id) {
    this.listShown = false
    if (id === this.openNoteId && this.view === 'note') return
    await this.flushEditor()
    this.view = 'note'
    this.openNoteId = id
    this.reveal(id)
    persist(workspaceKey(OPEN_NOTE_KEY, this.workspaces.active), id)
  }

  /** Follows a link to another note (`note:<id>`). */
  async openLinkedNote(id: Id) {
    const item = this.byId.get(id)
    if (item?.kind === 'note') {
      this.sidebarTab = 'notes'
      await this.openNote(id).catch((e) => this.fail(e))
    } else {
      this.fail(
        item ? 'That link points at a folder.' : 'The linked note was deleted or is in the trash.',
      )
    }
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
      pickNoteLink: (editor) => (this.linkingEditor = editor),
    }
  }

  /** The editor the "link to a note" picker inserts into (null: closed). */
  linkingEditor = $state.raw<Editor | null>(null)
  /** "New workspace" dialog shown. */
  newWorkspaceOpen = $state(false)

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

  /** "Move to…": to the end of `parent` (null: the top level). */
  async moveInto(id: Id, parent: Id | null) {
    this.movingId = null
    if (this.byId.get(id)?.parent_id === parent) return
    await this.move(id, { parent, position: { type: 'last' } })
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
