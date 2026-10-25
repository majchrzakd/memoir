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
  /** Number of replies in the note's thread (0 for folders). */
  replies: number
}

export interface Note {
  id: Id
  parent_id: Id | null
  title: string
  /** Markdown */
  body: string
  tags: string[]
  created_at: number
  /** Last change to the content (title, body, tags). */
  updated_at: number
  trashed_at: number | null
  /** When the note was put on the Later list; null = not on it. */
  later_at: number | null
  /** Due date for a Later note, YYYY-MM-DD. */
  due_date: string | null
}

export interface LaterItem {
  id: Id
  parent_id: Id | null
  title: string
  created_at: number
  updated_at: number
  due_date: string | null
}

/** Matches in `snippet` are wrapped in MATCH_START / MATCH_END. */
export interface SearchHit {
  id: Id
  parent_id: Id | null
  title: string
  snippet: string
  /** Set when the match is in a reply; `id` is then the thread's note. */
  reply_id: Id | null
}

/** A reply in a note's thread (a note without a title). */
export interface Reply {
  id: Id
  /** Markdown */
  body: string
  created_at: number
  updated_at: number
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
  /** Puts a note on the Later list or takes it off (which clears the due date). */
  setLater(id: Id, later: boolean, dueDate: string | null): Promise<void>
  /** Later notes: dated first (soonest first), then by creation, newest first. */
  laterNotes(): Promise<LaterItem[]>

  /** Replies in a note's thread, oldest first. */
  replies(noteId: Id): Promise<Reply[]>
  addReply(noteId: Id, body: string): Promise<Id>
  saveReply(id: Id, body: string): Promise<void>
  /** Deletes a reply for good. */
  deleteReply(id: Id): Promise<void>

  /** Encrypts and stores a file; reference it in Markdown as `attachment:<id>`. */
  addAttachment(file: File): Promise<AttachmentInfo>
  /** URL an <img> can load an image attachment from. */
  attachmentUrl(id: string): string
  /** Opens an attachment in its default program. */
  openAttachment(id: string, name: string): Promise<void>
  /** Asks where to save an attachment; false if cancelled. */
  saveAttachment(id: string, name: string): Promise<boolean>

  /** Google Drive connection. */
  cloudStatus(): Promise<CloudStatus>
  /** Verifies the connection with Google (network). */
  checkDrive(): Promise<CloudStatus>
  /** Opens the browser for sign-in; resolves when connected. */
  connectDrive(): Promise<CloudStatus>
  cancelConnect(): Promise<void>
  disconnectDrive(): Promise<CloudStatus>
}

export interface CloudStatus {
  /** This build knows a Google OAuth client. */
  configured: boolean
  connected: boolean
  connecting: boolean
  account: string | null
  problem: string | null
}

export interface AttachmentInfo {
  /** 64 hex digits */
  id: string
  size: number
  mime: string
}

/** Largest attachment accepted (the backend enforces it too). */
export const MAX_ATTACHMENT_SIZE = 100 * 1024 * 1024
