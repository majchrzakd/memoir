import { convertFileSrc, invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import type { Api, Id, SyncStatus } from './types'

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
  saveNote: (id, title, body, base) =>
    invoke('save_note', { id, title, body, baseTitle: base.title, baseBody: base.body }),
  search: (query, limit) => invoke('search', { query, limit }),
  noteHistory: (id) => invoke('note_history', { id }),
  restoreVersion: (id, savedAt) => invoke('restore_version', { id, savedAt }),
  setLater: (id, later, dueDate) => invoke('set_later', { id, later, dueDate }),
  laterNotes: () => invoke('later_notes'),
  replies: (noteId) => invoke('replies', { noteId }),
  addReply: (noteId, body) => invoke('add_reply', { noteId, body }),
  saveReply: (id, body, baseBody) => invoke('save_reply', { id, body, baseBody }),
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

  cloudStatus: () => invoke('cloud_status'),
  checkDrive: () => invoke('check_drive'),
  connectDrive: () => invoke('connect_drive'),
  cancelConnect: () => invoke('cancel_connect'),
  disconnectDrive: () => invoke('disconnect_drive'),
  findDriveNotebook: () => invoke('find_drive_notebook'),
  openDriveNotebook: (passphrase, remember) =>
    invoke('open_drive_notebook', { passphrase, rememberKey: remember }),
  forgetDriveNotebook: () => invoke('forget_drive_notebook'),

  exportNotebook: (name) => invoke('export_notebook', { name }),
  showExport: (path) => invoke('show_export', { path }),

  appInfo: () => invoke('app_info'),
  showLogs: () => invoke('show_logs'),

  syncStatus: () => invoke('sync_status'),
  syncNow: () => invoke('sync_now'),
  onSyncStatus: (handler) => listen<SyncStatus>('sync-status', (e) => handler(e.payload)),
  onSyncChanged: (handler) => listen<Id[]>('sync-changed', (e) => handler(e.payload)),
}
