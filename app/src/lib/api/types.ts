// Mirrors the Rust types serialized by the Tauri commands
// (app/src-tauri/src/commands.rs, crates/memoir-core/src/store/mod.rs).

export type Id = string
export type Kind = 'folder' | 'note'

export type Status = 'first_run' | 'confirm_recovery_key' | 'locked' | 'unlocked'

export interface TreeItem {
  id: Id
  kind: Kind
  /** null = top level */
  parent_id: Id | null
  title: string
  sort_key: string
  updated_at: number
  trashed_at: number | null
}

export interface Note {
  id: Id
  parent_id: Id | null
  title: string
  /** Markdown */
  body: string
  tags: string[]
  created_at: number
  updated_at: number
  trashed_at: number | null
}

/** Matches in `snippet` are wrapped in MATCH_START / MATCH_END. */
export interface SearchHit {
  id: Id
  parent_id: Id | null
  title: string
  snippet: string
}

export const MATCH_START = '\u0001'
export const MATCH_END = '\u0002'

export type Position =
  { type: 'first' } | { type: 'last' } | { type: 'before'; id: Id } | { type: 'after'; id: Id }

export interface Placement {
  parent: Id | null
  position: Position
}

export interface Api {
  status(): Promise<Status>
  /** Returns the recovery key to show; nothing is saved until finishSetup. */
  beginSetup(passphrase: string): Promise<string>
  finishSetup(): Promise<void>
  unlock(passphrase: string, remember: boolean): Promise<void>
  unlockWithRecoveryKey(
    recoveryKey: string,
    newPassphrase: string,
    remember: boolean,
  ): Promise<void>
  /** Called when the app is closing; the handler must save pending edits. */
  onFlushRequest(handler: () => Promise<void>): Promise<() => void>

  tree(): Promise<TreeItem[]>
  trashedItems(): Promise<TreeItem[]>
  createFolder(placement: Placement, name: string): Promise<Id>
  createNote(placement: Placement, title: string): Promise<Id>
  rename(id: Id, title: string): Promise<void>
  moveItem(id: Id, placement: Placement): Promise<void>
  moveToTrash(id: Id): Promise<void>
  restore(id: Id): Promise<void>
  purge(id: Id): Promise<void>
  emptyTrash(): Promise<number>

  note(id: Id): Promise<Note>
  saveNote(id: Id, title: string, body: string): Promise<void>
  search(query: string, limit: number): Promise<SearchHit[]>
}
