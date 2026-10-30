import { describe, expect, it } from 'vitest'
import type { TreeItem } from './api/types'
import {
  ancestorIds,
  childrenMap,
  displayTitle,
  dropTarget,
  folderPath,
  moveTargets,
  snippetParts,
  visibleRows,
} from './tree'

const item = (
  id: string,
  kind: 'folder' | 'note',
  parent: string | null,
  title = id,
): TreeItem => ({
  id,
  kind,
  parent_id: parent,
  title,
  sort_key: 'a0',
  updated_at: 0,
  trashed_at: null,
  replies: 0,
  protected: false,
  locked: false,
})

// Work/
//   Projects/
//     Plan
//   Notes-A
// Inbox
const items = [
  item('work', 'folder', null, 'Work'),
  item('inbox', 'note', null, 'Inbox'),
  item('projects', 'folder', 'work', 'Projects'),
  item('a', 'note', 'work'),
  item('plan', 'note', 'projects'),
]
const byId = new Map(items.map((i) => [i.id, i]))
const children = childrenMap(items)

describe('visibleRows', () => {
  it('shows only the top level when nothing is expanded', () => {
    expect(visibleRows(children, new Set()).map((r) => r.item.id)).toEqual(['work', 'inbox'])
  })

  it('shows contents of expanded folders in order with depth', () => {
    const rows = visibleRows(children, new Set(['work', 'projects']))
    expect(rows.map((r) => [r.item.id, r.depth])).toEqual([
      ['work', 0],
      ['projects', 1],
      ['plan', 2],
      ['a', 1],
      ['inbox', 0],
    ])
    expect(rows[0]).toMatchObject({ hasChildren: true, expanded: true })
  })

  it('hides contents of a collapsed folder inside an expanded one', () => {
    expect(visibleRows(children, new Set(['projects'])).map((r) => r.item.id)).toEqual([
      'work',
      'inbox',
    ])
  })
})

describe('paths', () => {
  it('lists ancestors nearest first and builds a folder path', () => {
    expect(ancestorIds(byId, 'plan')).toEqual(['projects', 'work'])
    expect(folderPath(byId, 'plan')).toBe('Work / Projects')
    expect(folderPath(byId, 'inbox')).toBe('')
  })
})

describe('dropTarget', () => {
  const rows = visibleRows(children, new Set(['work', 'projects']))
  const row = (id: string) => rows.find((r) => r.item.id === id)!

  it('drops into the middle of a folder', () => {
    expect(dropTarget(row('projects'), 0.5, 'inbox', byId)).toEqual({
      zone: 'inside',
      placement: { parent: 'projects', position: { type: 'last' } },
    })
  })

  it('drops before or after a note', () => {
    expect(dropTarget(row('a'), 0.2, 'inbox', byId)?.placement).toEqual({
      parent: 'work',
      position: { type: 'before', id: 'a' },
    })
    expect(dropTarget(row('a'), 0.8, 'inbox', byId)?.placement).toEqual({
      parent: 'work',
      position: { type: 'after', id: 'a' },
    })
  })

  it('treats the bottom of an expanded folder as inside it', () => {
    expect(dropTarget(row('projects'), 0.9, 'inbox', byId)?.zone).toBe('inside')
  })

  it('refuses dropping a folder into itself or its descendants', () => {
    expect(dropTarget(row('work'), 0.5, 'work', byId)).toBeNull()
    expect(dropTarget(row('projects'), 0.5, 'work', byId)).toBeNull()
    expect(dropTarget(row('plan'), 0.2, 'work', byId)).toBeNull()
    expect(dropTarget(row('inbox'), 0.2, 'work', byId)).not.toBeNull()
  })
})

describe('snippetParts', () => {
  it('splits highlighted matches', () => {
    expect(snippetParts('…we visit \u0001Kraków\u0002 and \u0001krakow\u0002!')).toEqual([
      { text: '…we visit ', match: false },
      { text: 'Kraków', match: true },
      { text: ' and ', match: false },
      { text: 'krakow', match: true },
      { text: '!', match: false },
    ])
    expect(snippetParts('plain')).toEqual([{ text: 'plain', match: false }])
  })
})

it('names untitled items', () => {
  expect(displayTitle({ title: '  ', kind: 'note' })).toBe('Untitled')
  expect(displayTitle({ title: '', kind: 'folder' })).toBe('Untitled folder')
  expect(displayTitle({ title: 'X', kind: 'note' })).toBe('X')
})

describe('moveTargets', () => {
  const targets = (id: string) =>
    moveTargets(items, id).map((t) => `${'  '.repeat(t.depth)}${t.title}${t.current ? ' *' : ''}`)

  it('lists the top level and every folder, marking the current one', () => {
    expect(targets('plan')).toEqual(['Top level', '  Work', '    Projects *'])
    expect(targets('inbox')).toEqual(['Top level *', '  Work', '    Projects'])
  })

  it('leaves out a folder and everything inside it', () => {
    expect(targets('work')).toEqual(['Top level *'])
    expect(targets('projects')).toEqual(['Top level', '  Work *'])
  })
})
