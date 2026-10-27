<script lang="ts">
  import type { Id, Placement } from '../api'
  import { app } from '../app.svelte'
  import { keys, touch } from '../platform'
  import { fullTime, shortTime } from '../time'
  import { displayTitle, dropTarget, type DropZone, type Row } from '../tree'
  import ContextMenu, { type MenuItem } from '../ui/ContextMenu.svelte'
  import Icon from '../ui/Icon.svelte'

  /** Rows are virtualized, so their height is fixed (taller for fingers). */
  const rowHeight = $derived(touch.current ? 44 : 28)
  const OVERSCAN = 10
  const EXPAND_ON_HOVER_MS = 700

  let viewport = $state<HTMLDivElement>()
  let scrollTop = $state(0)
  let viewportHeight = $state(0)

  // Only rows near the visible area are rendered, so large trees stay fast.
  const first = $derived(Math.max(0, Math.floor(scrollTop / rowHeight) - OVERSCAN))
  const last = $derived(
    Math.min(app.rows.length, Math.ceil((scrollTop + viewportHeight) / rowHeight) + OVERSCAN),
  )
  const visible = $derived(app.rows.slice(first, last))
  const selectedIndex = $derived(app.rows.findIndex((r) => r.item.id === app.selectedId))

  let menu = $state<{ x: number; y: number; items: MenuItem[] } | null>(null)
  let dragId: Id | null = null
  let drop = $state<{ id: Id | null; zone: DropZone; placement: Placement } | null>(null)
  let hoverTimer: ReturnType<typeof setTimeout> | undefined
  let renameValue = $state('')

  // Keep the selected row in view.
  $effect(() => {
    if (!viewport || selectedIndex < 0) return
    const top = selectedIndex * rowHeight
    if (top < viewport.scrollTop) viewport.scrollTop = top
    else if (top + rowHeight > viewport.scrollTop + viewport.clientHeight)
      viewport.scrollTop = top + rowHeight - viewport.clientHeight
  })

  // Start renaming with the current title.
  $effect(() => {
    const id = app.renamingId
    if (id) renameValue = app.byId.get(id)?.title ?? ''
  })

  function select(row: Row) {
    app.selectedId = row.item.id
    if (row.item.kind === 'note') void app.openNote(row.item.id)
    else app.toggle(row.item.id)
  }

  function commitRename(id: Id) {
    if (app.renamingId === id) void app.rename(id, renameValue)
  }

  /** Action: focus the rename box with its text selected, once it is on screen. */
  function focusAndSelect(input: HTMLInputElement) {
    const frame = requestAnimationFrame(() => {
      input.focus()
      input.select()
    })
    return { destroy: () => cancelAnimationFrame(frame) }
  }

  function onkeydown(e: KeyboardEvent) {
    if (app.renamingId) return
    const rows = app.rows
    const row = rows[selectedIndex]
    const go = (i: number) => {
      const target = rows[Math.max(0, Math.min(rows.length - 1, i))]
      if (target) app.selectedId = target.item.id
    }
    switch (e.key) {
      case 'ArrowDown':
        go(selectedIndex + 1)
        break
      case 'ArrowUp':
        go(selectedIndex < 0 ? rows.length - 1 : selectedIndex - 1)
        break
      case 'Home':
        go(0)
        break
      case 'End':
        go(rows.length - 1)
        break
      case 'ArrowRight':
        if (row?.item.kind === 'folder') {
          if (!row.expanded) app.toggle(row.item.id, true)
          else if (row.hasChildren) go(selectedIndex + 1)
        }
        break
      case 'ArrowLeft':
        if (row?.expanded) app.toggle(row.item.id, false)
        else if (row?.item.parent_id) app.selectedId = row.item.parent_id
        break
      case 'Enter':
        if (row) select(row)
        break
      case 'F2':
        if (row) app.renamingId = row.item.id
        break
      case 'Delete':
        if (row) void app.trash(row.item.id)
        break
      case 'Backspace': // ⌘⌫ on a Mac, as in Finder
        if (!e.metaKey) return
        if (row) void app.trash(row.item.id)
        break
      case 'ContextMenu':
        if (row) openMenu(row, 24 + row.depth * 16, (selectedIndex + 1) * rowHeight - scrollTop)
        break
      default:
        return
    }
    e.preventDefault()
  }

  /** A shortcut to show in the menu; none on touch screens. */
  const hint = (shortcut: string) => (touch.current ? undefined : shortcut)

  function openMenu(row: Row | null, x: number, y: number) {
    const rect = viewport?.getBoundingClientRect()
    const at = { x: (rect?.left ?? 0) + x, y: (rect?.top ?? 0) + y }
    const item = row?.item
    let items: MenuItem[]
    if (!item) {
      items = [
        { label: 'New note', shortcut: hint(keys('Ctrl+N')), action: () => app.newNote(null) },
        {
          label: 'New folder',
          shortcut: hint(keys('Ctrl+Shift+N')),
          action: () => app.newFolder(null),
        },
      ]
    } else {
      app.selectedId = item.id
      items = [
        ...(item.kind === 'folder'
          ? [
              { label: 'New note here', action: () => app.newNote(item.id) },
              { label: 'New folder here', action: () => app.newFolder(item.id) },
            ]
          : [{ label: 'Open', action: () => app.openNote(item.id) }]),
        { label: 'Rename', shortcut: hint('F2'), action: () => (app.renamingId = item.id) },
        {
          label: 'Move to trash',
          shortcut: hint('Del'),
          danger: true,
          action: () => app.trash(item.id),
        },
      ]
    }
    menu = { ...at, items }
  }

  // ---- Drag & drop ----

  function ondragstart(e: DragEvent, row: Row) {
    dragId = row.item.id
    e.dataTransfer?.setData('text/plain', row.item.id)
    if (e.dataTransfer) e.dataTransfer.effectAllowed = 'move'
  }

  function ondragover(e: DragEvent, row: Row) {
    if (!dragId) return
    const rect = (e.currentTarget as HTMLElement).getBoundingClientRect()
    const target = dropTarget(row, (e.clientY - rect.top) / rect.height, dragId, app.byId)
    e.stopPropagation()
    if (!target) {
      drop = null
      return
    }
    e.preventDefault()
    if (drop?.id !== row.item.id || drop.zone !== target.zone) {
      clearTimeout(hoverTimer)
      if (target.zone === 'inside' && !row.expanded) {
        hoverTimer = setTimeout(() => app.toggle(row.item.id, true), EXPAND_ON_HOVER_MS)
      }
    }
    drop = { id: row.item.id, ...target }
  }

  /** Dropping on empty space moves to the end of the top level. */
  function ondragoverEmpty(e: DragEvent) {
    if (!dragId) return
    e.preventDefault()
    drop = { id: null, zone: 'inside', placement: { parent: null, position: { type: 'last' } } }
  }

  function ondrop(e: DragEvent) {
    e.preventDefault()
    if (dragId && drop) void app.move(dragId, drop.placement)
    ondragend()
  }

  function ondragend() {
    clearTimeout(hoverTimer)
    dragId = null
    drop = null
  }
</script>

<div
  class="viewport"
  class:drop-root={drop?.id === null}
  role="tree"
  aria-label="Notebook"
  tabindex="0"
  aria-activedescendant={app.selectedId ? `row-${app.selectedId}` : undefined}
  style:--row-height="{rowHeight}px"
  bind:this={viewport}
  bind:clientHeight={viewportHeight}
  onscroll={() => (scrollTop = viewport?.scrollTop ?? 0)}
  {onkeydown}
  oncontextmenu={(e) => {
    e.preventDefault()
    const rect = viewport!.getBoundingClientRect()
    openMenu(null, e.clientX - rect.left, e.clientY - rect.top)
  }}
  ondragover={ondragoverEmpty}
  {ondrop}
>
  <div class="spacer" style:height="{app.rows.length * rowHeight}px">
    <div class="window" style:transform="translateY({first * rowHeight}px)">
      {#each visible as row (row.item.id)}
        {@const item = row.item}
        <!-- Keyboard interaction is handled by the tree container (roving selection). -->
        <!-- svelte-ignore a11y_click_events_have_key_events -->
        <div
          id="row-{item.id}"
          class="row"
          class:selected={item.id === app.selectedId}
          class:open={item.id === app.openNoteId && app.view === 'note'}
          class:drop-before={drop?.id === item.id && drop.zone === 'before'}
          class:drop-after={drop?.id === item.id && drop.zone === 'after'}
          class:drop-inside={drop?.id === item.id && drop.zone === 'inside'}
          style:padding-left="{6 + row.depth * 16}px"
          role="treeitem"
          aria-level={row.depth + 1}
          aria-selected={item.id === app.selectedId}
          aria-expanded={item.kind === 'folder' ? row.expanded : undefined}
          tabindex="-1"
          draggable={!touch.current && app.renamingId !== item.id}
          onclick={() => select(row)}
          oncontextmenu={(e) => {
            e.preventDefault()
            e.stopPropagation()
            const rect = viewport!.getBoundingClientRect()
            openMenu(row, e.clientX - rect.left, e.clientY - rect.top)
          }}
          ondragstart={(e) => ondragstart(e, row)}
          ondragover={(e) => ondragover(e, row)}
          {ondrop}
          {ondragend}
        >
          {#if item.kind === 'folder'}
            <button
              class="chevron"
              class:expanded={row.expanded}
              class:empty={!row.hasChildren}
              tabindex="-1"
              aria-label={row.expanded ? 'Collapse' : 'Expand'}
              onclick={(e) => {
                e.stopPropagation()
                app.toggle(item.id)
              }}
            >
              <Icon name="chevron" size={14} />
            </button>
            <Icon name={row.expanded ? 'folderOpen' : 'folder'} />
          {:else}
            <span class="chevron-space"></span>
            <Icon name="note" />
          {/if}

          {#if app.renamingId === item.id}
            <input
              class="rename"
              type="text"
              bind:value={renameValue}
              use:focusAndSelect
              onclick={(e) => e.stopPropagation()}
              onkeydown={(e) => {
                e.stopPropagation()
                if (e.key === 'Enter') commitRename(item.id)
                if (e.key === 'Escape') {
                  app.renamingId = null
                  viewport?.focus()
                }
              }}
              onblur={() => commitRename(item.id)}
            />
          {:else}
            <span class="title" class:untitled={!item.title.trim()}>{displayTitle(item)}</span>
            {#if item.replies > 0}
              <span
                class="replies"
                title="{item.replies} {item.replies === 1 ? 'reply' : 'replies'}"
                aria-label="{item.replies} {item.replies === 1 ? 'reply' : 'replies'}"
              >
                <Icon name="thread" size={13} />{item.replies}
              </span>
            {/if}
            {#if item.kind === 'note'}
              <span class="time" title="Edited {fullTime(item.updated_at)}">
                {shortTime(item.updated_at)}
              </span>
            {/if}
            <!-- Touch screens have no right-click: the row's menu is here. -->
            {#if touch.current}
              <button
                class="more"
                tabindex="-1"
                aria-label="More"
                onclick={(e) => {
                  e.stopPropagation()
                  const at = e.currentTarget.getBoundingClientRect()
                  const rect = viewport!.getBoundingClientRect()
                  openMenu(row, at.left - rect.left, at.bottom - rect.top)
                }}
              >
                <Icon name="more" size={20} />
              </button>
            {/if}
          {/if}
        </div>
      {/each}
    </div>
  </div>
  {#if app.rows.length === 0}
    <p class="no-notes muted">
      No notes yet. {touch.current
        ? 'Tap the new-note button to create one.'
        : `Press ${keys('Ctrl+N')} to create one.`}
    </p>
  {/if}
</div>

{#if menu}
  <ContextMenu {...menu} onclose={() => (menu = null)} />
{/if}

<style>
  .viewport {
    position: relative;
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    outline: none;
    padding-bottom: 24px;
  }

  .viewport.drop-root {
    background: var(--bg-hover);
  }

  .spacer {
    position: relative;
  }

  .window {
    position: absolute;
    inset: 0 0 auto 0;
  }

  .row {
    position: relative;
    display: flex;
    align-items: center;
    gap: 6px;
    height: var(--row-height);
    margin: 0 6px;
    padding-right: 8px;
    border-radius: 6px;
    color: var(--text);
    cursor: default;
    user-select: none;
    white-space: nowrap;
  }

  .row :global(svg) {
    flex: none;
    color: var(--text-muted);
  }

  .row:hover {
    background: var(--bg-hover);
  }

  .row.selected {
    background: var(--bg-selected);
  }

  .viewport:focus-visible .row.selected {
    outline: 1px solid var(--accent);
  }

  .row.open .title {
    font-weight: 600;
  }

  .row.drop-inside {
    background: var(--bg-selected);
    outline: 2px dashed var(--accent);
  }

  .row.drop-before::before,
  .row.drop-after::after {
    content: '';
    position: absolute;
    left: 0;
    right: 0;
    height: 2px;
    background: var(--accent);
  }

  .row.drop-before::before {
    top: -1px;
  }

  .row.drop-after::after {
    bottom: -1px;
  }

  .title {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .replies {
    flex: none;
    display: inline-flex;
    align-items: center;
    gap: 2px;
    font-size: 11px;
    color: var(--text-muted);
  }

  .time {
    flex: none;
    margin-left: 4px;
    font-size: 11px;
    color: var(--text-muted);
  }

  /* Not enough room next to titles in a narrow sidebar. */
  @container (max-width: 230px) {
    .time {
      display: none;
    }
  }

  .untitled {
    color: var(--text-muted);
    font-style: italic;
  }

  .chevron,
  .chevron-space {
    flex: none;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 16px;
    height: 16px;
    padding: 0;
    border: none;
    background: none;
  }

  .chevron :global(svg) {
    transition: transform 0.12s;
  }

  .chevron.expanded :global(svg) {
    transform: rotate(90deg);
  }

  .chevron.empty {
    opacity: 0.35;
  }

  .rename {
    height: 22px;
    padding: 0 6px !important;
  }

  .more {
    flex: none;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 36px;
    height: 36px;
    margin-right: -6px;
    padding: 0;
    border: none;
    background: none;
    color: var(--text-muted);
  }

  .no-notes {
    padding: 8px 16px;
    font-size: 13px;
  }
</style>
