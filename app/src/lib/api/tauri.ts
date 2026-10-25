import { convertFileSrc, invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import type { Api } from './types'

/** The real backend: Tauri commands (app/src-tauri/src/commands.rs). */
export const tauriApi: Api = {
  status: () => invoke('status'),
  beginSetup: (passphrase) => invoke('begin_setup', { passphrase }),
  finishSetup: () => invoke('finish_setup'),
  unlock: (passphrase, remember) => invoke('unlock', { passphrase, remember }),
  unlockWithRecoveryKey: (recoveryKey, newPassphrase, remember) =>
    invoke('unlock_with_recovery_key', { recoveryKey, newPassphrase, remember }),
  onFlushRequest: (handler) =>
    listen('flush-editor', async () => {
      try {
        await handler()
      } finally {
        await invoke('editor_flushed')
      }
    }),

  tree: () => invoke('tree'),
  trashedItems: () => invoke('trashed_items'),
  createFolder: (placement, name) => invoke('create_folder', { placement, name }),
  createNote: (placement, title) => invoke('create_note', { placement, title }),
  rename: (id, title) => invoke('rename', { id, title }),
  moveItem: (id, placement) => invoke('move_item', { id, placement }),
  moveToTrash: (id) => invoke('move_to_trash', { id }),
  restore: (id) => invoke('restore', { id }),
  purge: (id) => invoke('purge', { id }),
  emptyTrash: () => invoke('empty_trash'),

  note: (id) => invoke('note', { id }),
  saveNote: (id, title, body) => invoke('save_note', { id, title, body }),
  search: (query, limit) => invoke('search', { query, limit }),
  setLater: (id, later, dueDate) => invoke('set_later', { id, later, dueDate }),
  laterNotes: () => invoke('later_notes'),
  replies: (noteId) => invoke('replies', { noteId }),
  addReply: (noteId, body) => invoke('add_reply', { noteId, body }),
  saveReply: (id, body) => invoke('save_reply', { id, body }),
  deleteReply: (id) => invoke('delete_reply', { id }),

  // The file goes over IPC as raw bytes, not JSON.
  addAttachment: async (file) =>
    invoke('add_attachment', new Uint8Array(await file.arrayBuffer()), {
      headers: { 'x-mime': file.type || 'application/octet-stream' },
    }),
  // Served by the `attachment` URI scheme (app/src-tauri/src/attachments.rs).
  attachmentUrl: (id) => convertFileSrc(id, 'attachment'),
  openAttachment: (id, name) => invoke('open_attachment', { id, name }),
  saveAttachment: (id, name) => invoke('save_attachment', { id, name }),
}
