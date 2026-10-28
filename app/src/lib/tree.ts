// Pure helpers for the sidebar tree (kept free of Svelte for unit tests).

import type { Id, Placement, TreeItem } from './api'

export interface Row {
  item: TreeItem
  depth: number
  hasChildren: boolean
  expanded: boolean
}

/** Children of each parent (null = top level), in the order given. */
export function childrenMap(items: readonly TreeItem[]): Map<Id | null, TreeItem[]> {
  const map = new Map<Id | null, TreeItem[]>()
  for (const item of items) {
    const list = map.get(item.parent_id)
    if (list) list.push(item)
    else map.set(item.parent_id, [item])
  }
  return map
}

/** The rows currently visible: top level plus contents of expanded folders. */
export function visibleRows(
  children: Map<Id | null, TreeItem[]>,
  expanded: ReadonlySet<Id>,
): Row[] {
  const rows: Row[] = []
  const stack: { item: TreeItem; depth: number }[] = (children.get(null) ?? [])
    .map((item) => ({ item, depth: 0 }))
    .reverse()
  while (stack.length) {
    const { item, depth } = stack.pop()!
    const kids = children.get(item.id) ?? []
    const isOpen = item.kind === 'folder' && expanded.has(item.id)
    rows.push({ item, depth, hasChildren: kids.length > 0, expanded: isOpen })
    if (isOpen) {
      for (let i = kids.length - 1; i >= 0; i--) stack.push({ item: kids[i], depth: depth + 1 })
    }
  }
  return rows
}

/** Ancestor folder ids of `id`, nearest first. */
export function ancestorIds(byId: ReadonlyMap<Id, TreeItem>, id: Id): Id[] {
  const out: Id[] = []
  const seen = new Set<Id>([id])
  for (
    let p = byId.get(id)?.parent_id ?? null;
    p && !seen.has(p);
    p = byId.get(p)?.parent_id ?? null
  ) {
    out.push(p)
    seen.add(p)
  }
  return out
}

/** "Personal / Trips" for an item inside those folders. */
export function folderPath(byId: ReadonlyMap<Id, TreeItem>, id: Id): string {
  return ancestorIds(byId, id)
    .reverse()
    .map((a) => byId.get(a)?.title ?? '')
    .join(' / ')
}

/** A folder (or the top level, id null) offered by "Move to…". */
export interface MoveTarget {
  id: Id | null
  title: string
  depth: number
  /** Where the item is now. */
  current: boolean
}

/**
 * Where `id` can be moved: the top level, then every folder in tree order. A
 * folder can't go into itself or anything inside it.
 */
export function moveTargets(items: readonly TreeItem[], id: Id): MoveTarget[] {
  const children = childrenMap(items)
  const parent = items.find((i) => i.id === id)?.parent_id ?? null
  const targets: MoveTarget[] = [
    { id: null, title: 'Top level', depth: 0, current: parent === null },
  ]
  const visit = (folder: Id | null, depth: number) => {
    for (const item of children.get(folder) ?? []) {
      if (item.kind !== 'folder' || item.id === id) continue
      targets.push({ id: item.id, title: displayTitle(item), depth, current: item.id === parent })
      visit(item.id, depth + 1)
    }
  }
  visit(null, 1)
  return targets
}

export type DropZone = 'before' | 'inside' | 'after'

/**
 * Where a dragged item lands when released over `row` at `fraction` (0 = top
 * edge, 1 = bottom edge) of the row. Folders accept drops in their middle
 * half; notes split at the middle. Returns null for impossible drops.
 */
export function dropTarget(
  row: Row,
  fraction: number,
  dragged: Id,
  byId: ReadonlyMap<Id, TreeItem>,
): { zone: DropZone; placement: Placement } | null {
  const target = row.item
  if (target.id === dragged) return null

  let zone: DropZone
  if (target.kind === 'folder') {
    zone = fraction < 0.25 ? 'before' : fraction > 0.75 && !row.expanded ? 'after' : 'inside'
  } else {
    zone = fraction < 0.5 ? 'before' : 'after'
  }

  const parent = zone === 'inside' ? target.id : target.parent_id
  // A folder can't go into itself or anything inside it.
  if (parent && (parent === dragged || ancestorIds(byId, parent).includes(dragged))) return null

  const placement: Placement =
    zone === 'inside'
      ? { parent, position: { type: 'last' } }
      : { parent, position: { type: zone, id: target.id } }
  return { zone, placement }
}

/** Splits a search snippet into plain and highlighted parts. */
export function snippetParts(snippet: string): { text: string; match: boolean }[] {
  const parts: { text: string; match: boolean }[] = []
  for (const [i, chunk] of snippet.split('\u0001').entries()) {
    const [matched, rest] = i === 0 ? [undefined, chunk] : chunk.split('\u0002', 2)
    if (matched) parts.push({ text: matched, match: true })
    if (rest) parts.push({ text: rest, match: false })
  }
  return parts
}

export function displayTitle(item: { title: string; kind: string }): string {
  return item.title.trim() || (item.kind === 'folder' ? 'Untitled folder' : 'Untitled')
}
