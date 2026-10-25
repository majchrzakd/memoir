// In-memory stand-in for the Rust backend, used only by `npm run dev` in a
// plain browser. Behaviour follows the real store closely enough to work on
// the UI; the real rules and their tests live in memoir-core.
//
// URL parameters: ?mock=first_run | locked (default: unlocked with samples).
// In the locked mock the passphrase is "correct horse".

import {
  MATCH_END,
  MATCH_START,
  type Api,
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

  function toItem(o: Obj): TreeItem {
    const { id, kind, parent_id, title, updated_at, trashed_at } = o
    return { id, kind, parent_id, title, sort_key: String(o.order), updated_at, trashed_at }
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
        .filter((o) => !hidden(o))
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
        .filter((o) => o.trashed_at !== null && !(o.parent_id && hidden(get(o.parent_id))))
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
      if (o.title === title && o.body === body) return
      Object.assign(o, { title, body, updated_at: now() })
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
          return { id: o.id, parent_id: o.parent_id, title: o.title, snippet }
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
