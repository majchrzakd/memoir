<script lang="ts">
  import { untrack } from 'svelte'
  import type { Editor } from '@tiptap/core'
  import type { TreeItem } from '../api'
  import { app } from '../app.svelte'
  import { touch } from '../platform'
  import { displayTitle, folderPath } from '../tree'
  import Icon from '../ui/Icon.svelte'
  import { focusOnMount } from '../ui/focus'
  import { insertNoteLink } from './noteLinks'

  let { editor: given }: { editor: Editor } = $props()
  // Kept: the prop follows `app.linkingEditor`, which closing clears.
  const editor = untrack(() => given)

  let query = $state('')
  let active = $state(0)
  let list = $state<HTMLDivElement>()

  const fold = (s: string) => s.normalize('NFD').replace(/\p{M}/gu, '').toLowerCase()

  /** Notes whose title contains the query: those starting with it first,
   * then the most recently edited. */
  const matches = $derived.by(() => {
    const q = fold(query.trim())
    const scored: { item: TreeItem; rank: number }[] = []
    for (const item of app.items) {
      if (item.kind !== 'note' || item.id === app.openNoteId) continue
      const title = fold(displayTitle(item))
      const at = q ? title.indexOf(q) : 0
      if (at < 0) continue
      scored.push({ item, rank: at === 0 ? 0 : 1 })
    }
    scored.sort((a, b) => a.rank - b.rank || b.item.updated_at - a.item.updated_at)
    return scored.slice(0, 50).map((s) => s.item)
  })

  $effect(() => {
    void query
    active = 0
  })

  $effect(() => {
    list?.querySelector(`[data-index="${active}"]`)?.scrollIntoView({ block: 'nearest' })
  })

  function close() {
    app.linkingEditor = null
    editor.commands.focus()
  }

  function choose(item: TreeItem) {
    app.linkingEditor = null
    insertNoteLink(editor, item.id, item.title)
  }

  function onkeydown(e: KeyboardEvent) {
    if (e.key === 'Escape') close()
    else if (e.key === 'ArrowDown') active = Math.min(active + 1, matches.length - 1)
    else if (e.key === 'ArrowUp') active = Math.max(active - 1, 0)
    else if (e.key === 'Enter' && matches[active]) choose(matches[active])
    else return
    e.preventDefault()
  }
</script>

<div class="backdrop" role="presentation" onclick={close}>
  <div
    class="dialog"
    role="dialog"
    aria-modal="true"
    aria-label="Link to a note"
    tabindex="-1"
    onclick={(e) => e.stopPropagation()}
    {onkeydown}
  >
    <div class="field">
      <Icon name="link" />
      <input
        type="text"
        placeholder="Link to a note…"
        aria-label="Note title"
        bind:value={query}
        use:focusOnMount
      />
    </div>
    <div
      class="list"
      role="listbox"
      style:--row-height="{touch.current ? 44 : 32}px"
      bind:this={list}
    >
      {#each matches as item, i (item.id)}
        {@const path = folderPath(app.byId, item.id)}
        <button
          role="option"
          aria-selected={i === active}
          class:active={i === active}
          data-index={i}
          onmousemove={() => (active = i)}
          onclick={() => choose(item)}
        >
          <Icon name="note" />
          <span class="title">{displayTitle(item)}</span>
          {#if path}<span class="folder">{path}</span>{/if}
        </button>
      {:else}
        <p class="empty">No note has that title.</p>
      {/each}
    </div>
  </div>
</div>

<style>
  .dialog {
    display: flex;
    flex-direction: column;
    width: min(520px, calc(100vw - 32px));
    max-height: 60vh;
    padding: 0;
    overflow: hidden;
  }

  .field {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 10px 14px;
    border-bottom: 1px solid var(--border);
    color: var(--text-muted);
  }

  .field input {
    flex: 1;
    border: none;
    background: none;
    outline: none;
    font-size: 15px;
  }

  .list {
    overflow-y: auto;
    padding: 4px;
  }

  button {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    height: var(--row-height);
    padding: 0 10px;
    border: none;
    border-radius: 6px;
    background: none;
    color: var(--text);
    text-align: left;
  }

  button.active {
    background: var(--bg-hover);
  }

  button :global(svg) {
    flex: none;
    color: var(--text-muted);
  }

  .title {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .folder {
    flex: none;
    max-width: 40%;
    overflow: hidden;
    color: var(--text-muted);
    font-size: 12px;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .empty {
    margin: 0;
    padding: 12px;
    color: var(--text-muted);
  }
</style>
