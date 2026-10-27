// In-memory stand-in for the Rust backend, used only by `npm run dev` in a
// plain browser. Behaviour follows the real store closely enough to work on
// the UI; the real rules and their tests live in memoir-core.
//
// URL parameters: ?mock=first_run | locked (default: unlocked with samples).
// In the locked mock the passphrase is "correct horse".

import {
  MATCH_END,
  MATCH_START,
  MAX_ATTACHMENT_SIZE,
  type Api,
  type CloudStatus,
  type HistoryEntry,
  type SyncStatus,
  type Id,
  type Kind,
  type Note,
  type Placement,
  type Status,
  type TreeItem,
} from './types'

interface Obj {
  id: Id
  kind: Kind
  parent_id: Id | null
  title: string
  order: number
  body: string
  tags: string[]
  created_at: number
  updated_at: number
  trashed_at: number | null
  later_at: number | null
  due_date: string | null
}

const MOCK_PASSPHRASE = 'correct horse'

export function createMockApi(): Api {
  const objects = new Map<Id, Obj>()
  const history = new Map<Id, HistoryEntry[]>()
  // Attachment id → object URL of the file.
  const attachments = new Map<string, string>()
  let lastSynced: number | null = null
  const mockUpdate = new URLSearchParams(location.search).has('update')
    ? { version: '0.2.0', notes: 'Faster sync and a new icon.' }
    : null
  const syncListeners = new Set<(s: SyncStatus) => void>()
  const syncStatus = (): SyncStatus => ({
    state: cloud.connected ? 'idle' : 'off',
    pending: 0,
    last_synced_at: cloud.connected ? lastSynced : null,
    message: null,
  })

  const cloud: CloudStatus = {
    configured: true,
    connected: false,
    connecting: false,
    account: null,
    problem: null,
  }
  let cancelConnect = () => {}
  let seq = 0
  const newId = () => `MOCK${String(++seq).padStart(22, '0')}`
  const now = () => Date.now()

  const param = new URLSearchParams(location.search).get('mock')
  let status: Status =
    param === 'first_run' ? 'first_run' : param === 'locked' ? 'locked' : 'unlocked'
  if (status !== 'first_run') seed()

  function get(id: Id): Obj {
    const o = objects.get(id)
    if (!o) throw `item ${id} not found`
    return o
  }

  function hidden(o: Obj): boolean {
    for (
      let cur: Obj | undefined = o;
      cur;
      cur = cur.parent_id ? objects.get(cur.parent_id) : undefined
    ) {
      if (cur.trashed_at !== null) return true
    }
    return false
  }

  function siblings(parent: Id | null, except?: Id): Obj[] {
    return [...objects.values()]
      .filter((o) => o.parent_id === parent && o.id !== except)
      .sort((a, b) => a.order - b.order)
  }

  function place(p: Placement, moving?: Id): number {
    if (p.parent) {
      const parent = get(p.parent)
      if (parent.kind !== 'folder') throw `${p.parent} is not a folder`
      if (hidden(parent)) throw `${p.parent} is in the trash`
      for (
        let cur: Obj | undefined = parent;
        cur;
        cur = cur.parent_id ? objects.get(cur.parent_id) : undefined
      ) {
        if (cur.id === moving) throw "a folder can't be moved into itself or one of its subfolders"
      }
    }
    const sibs = siblings(p.parent, moving)
    const index = (id: Id) => {
      const i = sibs.findIndex((s) => s.id === id)
      if (i < 0) throw `${id} is not in the target folder`
      return i
    }
    const between = (a?: Obj, b?: Obj) =>
      a && b ? (a.order + b.order) / 2 : a ? a.order + 1 : b ? b.order - 1 : 0
    switch (p.position.type) {
      case 'first':
        return between(undefined, sibs[0])
      case 'last':
        return between(sibs[sibs.length - 1], undefined)
      case 'after': {
        const i = index(p.position.id)
        return between(sibs[i], sibs[i + 1])
      }
      case 'before': {
        const i = index(p.position.id)
        return between(sibs[i - 1], sibs[i])
      }
    }
  }

  function create(kind: Kind, p: Placement, title: string, body = ''): Id {
    const id = newId()
    const t = now()
    objects.set(id, {
      id,
      kind,
      parent_id: p.parent,
      title,
      order: place(p),
      body,
      tags: [],
      created_at: t,
      updated_at: t,
      trashed_at: null,
      later_at: null,
      due_date: null,
    })
    return id
  }

  // A reply is a note whose parent is a note.
  function isReply(o: Obj): boolean {
    return o.kind === 'note' && !!o.parent_id && objects.get(o.parent_id)?.kind === 'note'
  }

  function repliesOf(id: Id): Obj[] {
    return [...objects.values()]
      .filter((r) => r.parent_id === id && r.kind === 'note' && r.trashed_at === null)
      .sort((a, b) => a.created_at - b.created_at)
  }

  function toItem(o: Obj): TreeItem {
    const { id, kind, parent_id, title, updated_at, trashed_at } = o
    const replies = kind === 'note' ? repliesOf(id).length : 0
    return {
      id,
      kind,
      parent_id,
      title,
      sort_key: String(o.order),
      updated_at,
      trashed_at,
      replies,
    }
  }

  function seed() {
    const last = (parent: Id | null): Placement => ({ parent, position: { type: 'last' } })
    const personal = create('folder', last(null), 'Personal')
    const trips = create('folder', last(personal), 'Trips')
    const work = create('folder', last(null), 'Work')
    create('note', last(trips), 'Kraków weekend', KRAKOW)
    create(
      'note',
      last(personal),
      'Reading list',
      '- [x] Dune\n- [ ] The Rust Programming Language\n- [ ] Solaris',
    )
    create('note', last(work), 'Meeting notes', MEETING)
    create('note', last(null), 'Welcome to Memoir', WELCOME)
    for (let i = 1; i <= 40; i++) {
      create('note', last(work), `Daily log ${i}`, `Worked on item ${i}. Next: item ${i + 1}.`)
    }

    // Spread creation and edit times over the past weeks.
    const all = [...objects.values()]
    all.forEach((o, i) => {
      o.created_at = now() - (all.length - i) * 9 * 3600_000
      o.updated_at = o.created_at + (i % 5) * 3600_000
    })
    // A few notes on the Later list: one overdue, one due soon, one undated.
    const day = (offset: number) => new Date(now() + offset * 86400_000).toISOString().slice(0, 10)
    const byTitle = (t: string) => all.find((o) => o.title === t)!
    Object.assign(byTitle('Reading list'), { later_at: now(), due_date: day(3) })
    Object.assign(byTitle('Daily log 7'), { later_at: now(), due_date: day(-1) })
    Object.assign(byTitle('Meeting notes'), { later_at: now(), due_date: null })

    // A thread with a few replies, a day apart.
    const meeting = byTitle('Meeting notes')
    const updates = [
      'Budget review moved to **Thursday**.',
      'Beta is out 🎉\n\n- [x] release notes\n- [ ] announce in the newsletter',
      'Ania asked for the slides — sent.',
    ]
    updates.forEach((body, i) =>
      insertReply(meeting.id, body, meeting.created_at + (i + 1) * 86400_000),
    )
  }

  function insertReply(noteId: Id, body: string, at: number): Id {
    const id = newId()
    objects.set(id, {
      id,
      kind: 'note',
      parent_id: noteId,
      title: '',
      order: 0,
      body,
      tags: [],
      created_at: at,
      updated_at: at,
      trashed_at: null,
      later_at: null,
      due_date: null,
    })
    return id
  }

  const plain = (md: string) =>
    md
      .replace(/!?\[([^\]]*)\]\([^)]*\)/g, '$1')
      .replace(/[#*_`>|~]|- \[[ x]\]|^-{3,}$/gm, ' ')
      .replace(/\s+/g, ' ')
      .trim()
  const fold = (s: string) =>
    s.normalize('NFD').replace(/\p{M}/gu, '').replace(/ł/g, 'l').toLowerCase()

  const api: Api = {
    async status() {
      return status
    },
    async beginSetup(passphrase) {
      if ([...passphrase].length < 8) throw 'the passphrase must be at least 8 characters'
      status = 'confirm_recovery_key'
      return 'K7Q3F-9XW2M-H4TZ1-0PB8N-R6VDC-YJ5SA-E2GQK-M8XWT-3FH9P-ZB4ND-17CR0'
    },
    async finishSetup() {
      status = 'unlocked'
    },
    async unlock(passphrase) {
      if (passphrase !== MOCK_PASSPHRASE) throw 'wrong passphrase'
      status = 'unlocked'
    },
    async unlockWithRecoveryKey(recoveryKey) {
      if (recoveryKey.replace(/[\s-]/g, '').length !== 55)
        throw 'this recovery key has a typo (checksum mismatch)'
      status = 'unlocked'
    },
    async onFlushRequest() {
      return () => {}
    },

    async tree() {
      const kindOrder = (k: Kind) => (k === 'folder' ? 0 : 1)
      return [...objects.values()]
        .filter((o) => !hidden(o) && !isReply(o))
        .sort(
          (a, b) =>
            (a.parent_id ?? '').localeCompare(b.parent_id ?? '') ||
            kindOrder(a.kind) - kindOrder(b.kind) ||
            a.order - b.order,
        )
        .map(toItem)
    },
    async trashedItems() {
      return [...objects.values()]
        .filter(
          (o) => o.trashed_at !== null && !isReply(o) && !(o.parent_id && hidden(get(o.parent_id))),
        )
        .sort((a, b) => (b.trashed_at ?? 0) - (a.trashed_at ?? 0))
        .map(toItem)
    },
    async createFolder(placement, name) {
      if (!name.trim()) throw "folder name can't be empty"
      return create('folder', placement, name.trim())
    },
    async createNote(placement, title) {
      return create('note', placement, title)
    },
    async rename(id, title) {
      const o = get(id)
      if (o.kind === 'folder' && !title.trim()) throw "folder name can't be empty"
      o.title = o.kind === 'folder' ? title.trim() : title
      o.updated_at = now()
    },
    async moveItem(id, placement) {
      const o = get(id)
      o.order = place(placement, id)
      o.parent_id = placement.parent
    },
    async moveToTrash(id) {
      get(id).trashed_at ??= now()
    },
    async restore(id) {
      const o = get(id)
      const parent = o.parent_id ? objects.get(o.parent_id) : undefined
      if (o.parent_id && (!parent || hidden(parent))) {
        o.parent_id = null
        o.order = place({ parent: null, position: { type: 'last' } }, id)
      }
      o.trashed_at = null
    },
    async purge(id) {
      const doomed = [id]
      for (let i = 0; i < doomed.length; i++) {
        for (const o of objects.values()) if (o.parent_id === doomed[i]) doomed.push(o.id)
      }
      doomed.forEach((d) => objects.delete(d))
    },
    async emptyTrash() {
      const trashed = [...objects.values()].filter((o) => o.trashed_at !== null)
      for (const o of trashed) if (objects.has(o.id)) await api.purge(o.id)
      return trashed.length
    },

    async note(id): Promise<Note> {
      const o = get(id)
      if (o.kind !== 'note') throw `${id} is not a note`
      const { id: noteId, parent_id, title, body, tags, created_at, updated_at, trashed_at } = o
      const { later_at, due_date } = o
      return {
        id: noteId,
        parent_id,
        title,
        body,
        tags: [...tags],
        created_at,
        updated_at,
        trashed_at,
        later_at,
        due_date,
      }
    },
    async saveNote(id, title, body) {
      const o = get(id)
      if (o.title === title && o.body === body) return null
      // Like the real rule, compressed: keep the old text after a 5 s pause.
      if ((o.title || o.body) && now() - o.updated_at > 5000) {
        const list = history.get(id) ?? []
        list.unshift({ saved_at: o.updated_at, title: o.title, body: o.body })
        history.set(id, list.slice(0, 50))
      }
      Object.assign(o, { title, body, updated_at: now() })
      return null
    },
    async noteHistory(id) {
      return [...(history.get(id) ?? [])]
    },
    async restoreVersion(id, savedAt) {
      const version = history.get(id)?.find((v) => v.saved_at === savedAt)
      if (!version) throw `no version ${savedAt}`
      const o = get(id)
      history.get(id)?.unshift({ saved_at: o.updated_at, title: o.title, body: o.body })
      Object.assign(o, { title: version.title, body: version.body, updated_at: now() })
    },
    async search(query, limit) {
      const words = fold(query).split(/\s+/).filter(Boolean)
      if (!words.length) return []
      return [...objects.values()]
        .filter((o) => o.kind === 'note' && !hidden(o))
        .map((o) => ({ o, text: plain(o.body), hay: fold(`${o.title} ${plain(o.body)}`) }))
        .filter(({ hay }) => words.every((w) => hay.includes(w)))
        .slice(0, limit)
        .map(({ o, text }) => {
          const at = fold(text).indexOf(words[0])
          const start = Math.max(0, at - 40)
          const snippet =
            at < 0
              ? text.slice(0, 80)
              : (start > 0 ? '…' : '') +
                text.slice(start, at) +
                MATCH_START +
                text.slice(at, at + words[0].length) +
                MATCH_END +
                text.slice(at + words[0].length, at + 80)
          const thread = isReply(o) ? get(o.parent_id!) : null
          return thread
            ? {
                id: thread.id,
                parent_id: thread.parent_id,
                title: thread.title,
                snippet,
                reply_id: o.id,
              }
            : { id: o.id, parent_id: o.parent_id, title: o.title, snippet, reply_id: null }
        })
    },
    async setLater(id, later, dueDate) {
      const o = get(id)
      if (o.kind !== 'note') throw `${id} is not a note`
      if (later && dueDate && !/^\d{4}-\d{2}-\d{2}$/.test(dueDate))
        throw `"${dueDate}" is not a valid date (expected YYYY-MM-DD)`
      o.later_at = later ? (o.later_at ?? now()) : null
      o.due_date = later ? dueDate : null
    },
    async laterNotes() {
      return [...objects.values()]
        .filter((o) => o.kind === 'note' && o.later_at !== null && !hidden(o))
        .sort(
          (a, b) =>
            Number(a.due_date === null) - Number(b.due_date === null) ||
            (a.due_date ?? '').localeCompare(b.due_date ?? '') ||
            b.created_at - a.created_at,
        )
        .map(({ id, parent_id, title, created_at, updated_at, due_date }) => ({
          id,
          parent_id,
          title,
          created_at,
          updated_at,
          due_date,
        }))
    },

    async replies(noteId) {
      return repliesOf(noteId).map(({ id, body, created_at, updated_at }) => ({
        id,
        body,
        created_at,
        updated_at,
      }))
    },
    async addReply(noteId, body) {
      const note = get(noteId)
      if (note.kind !== 'note') throw `${noteId} is not a note`
      if (isReply(note)) throw `${noteId} is a reply; replies stay with their note`
      if (hidden(note)) throw `${noteId} is in the trash`
      return insertReply(noteId, body, now())
    },
    async saveReply(id, body) {
      await api.saveNote(id, '', body, { title: '', body })
      return null
    },
    async deleteReply(id) {
      if (!isReply(get(id))) throw `${id} is not a reply`
      objects.delete(id)
    },

    async addAttachment(file) {
      if (file.size > MAX_ATTACHMENT_SIZE) throw 'the file is too large'
      const digest = await crypto.subtle.digest('SHA-256', await file.arrayBuffer())
      const id = [...new Uint8Array(digest)].map((b) => b.toString(16).padStart(2, '0')).join('')
      if (!attachments.has(id)) attachments.set(id, URL.createObjectURL(file))
      return { id, size: file.size, mime: file.type || 'application/octet-stream' }
    },
    attachmentUrl(id) {
      return attachments.get(id) ?? ''
    },
    async openAttachment(id) {
      window.open(attachments.get(id), '_blank')
    },
    async exportNotebook(name) {
      await new Promise((r) => setTimeout(r, 600))
      const notes = [...objects.values()].filter((o) => o.kind === 'note' && !isReply(o))
      return {
        path: `C:\\Users\\you\\Documents\\${name}`,
        summary: {
          folders: [...objects.values()].filter((o) => o.kind === 'folder').length,
          notes: notes.length,
          attachments: 0,
          missing_attachments: 0,
        },
      }
    },
    async showExport() {},

    async appInfo() {
      return {
        version: '0.1.0 (mock)',
        log_dir: 'C:\\Users\\you\\AppData\\Local\\com.memoir.notebook\\logs',
      }
    },
    async showLogs() {},

    // ?update in the URL pretends a newer version exists.
    async checkForUpdate() {
      await new Promise((r) => setTimeout(r, 500))
      return mockUpdate
    },
    async availableUpdate() {
      return mockUpdate
    },
    async installUpdate() {
      await new Promise((r) => setTimeout(r, 800))
      location.reload()
    },
    async onUpdateAvailable() {
      return () => {}
    },

    async syncStatus() {
      return { ...syncStatus() }
    },
    async syncNow() {
      if (!cloud.connected) return
      const emit = (s: SyncStatus) => syncListeners.forEach((l) => l(s))
      emit({ ...syncStatus(), state: 'syncing' })
      setTimeout(() => {
        lastSynced = now()
        emit(syncStatus())
      }, 700)
    },
    async onSyncStatus(handler) {
      syncListeners.add(handler)
      return () => syncListeners.delete(handler)
    },
    async onSyncChanged() {
      return () => {}
    },

    async cloudStatus() {
      return { ...cloud }
    },
    async checkDrive() {
      return { ...cloud }
    },
    async connectDrive() {
      cloud.connecting = true
      await new Promise<void>((resolve, reject) => {
        const timer = setTimeout(resolve, 1500) // pretend the user signs in
        cancelConnect = () => {
          clearTimeout(timer)
          reject('Connecting was cancelled.')
        }
      }).finally(() => (cloud.connecting = false))
      Object.assign(cloud, { connected: true, account: 'you@example.com', problem: null })
      void api.syncNow()
      return { ...cloud }
    },
    async findDriveNotebook() {
      await api
        .connectDrive()
        .finally(() => Object.assign(cloud, { connected: false, account: null }))
      return 'you@example.com'
    },
    async openDriveNotebook(passphrase) {
      if (passphrase !== MOCK_PASSPHRASE) throw 'wrong passphrase'
      seed()
      status = 'unlocked'
      Object.assign(cloud, { connected: true, account: 'you@example.com', problem: null })
      void api.syncNow()
    },
    async forgetDriveNotebook() {},
    async cancelConnect() {
      cancelConnect()
    },
    async disconnectDrive() {
      Object.assign(cloud, { connected: false, account: null })
      syncListeners.forEach((l) => l(syncStatus()))
      return { ...cloud }
    },
    async saveAttachment(id, name) {
      const a = document.createElement('a')
      a.href = attachments.get(id) ?? ''
      a.download = name
      a.click()
      return true
    },
  }
  return api
}

const WELCOME = `# Welcome to Memoir

Your notes are **encrypted on this device** before they ever reach Google Drive.

## Try it

- [ ] Create a folder with the button in the sidebar
- [ ] Drag this note into it
- [ ] Press **Ctrl+K** and search for "kraków"

> Tip: everything saves automatically.
`

const KRAKOW = `## Plan

| Day | What |
| --- | --- |
| Sat | Wawel, Kazimierz |
| Sun | Wieliczka |

Pack: charger, passport, \`umbrella\`.`

const MEETING = `### Decisions

1. Ship the beta on Friday
2. Review the *budget* next week

\`\`\`
owner: Ania
\`\`\``
