<script lang="ts">
  import type { LaterItem } from '../api'
  import { app } from '../app.svelte'
  import { dueLabel } from '../time'
  import { displayTitle, folderPath } from '../tree'

  let { items }: { items: LaterItem[] } = $props()

  const firstUndated = $derived(items.findIndex((i) => !i.due_date))
</script>

<div class="list">
  {#if items.length}
    <ul aria-label="Later">
      {#each items as item, i (item.id)}
        {@const due = item.due_date ? dueLabel(item.due_date) : null}
        {@const path = folderPath(app.byId, item.id)}
        <li>
          {#if i === firstUndated && i > 0}
            <div class="divider">No due date</div>
          {/if}
          <button
            class="item"
            class:open={item.id === app.openNoteId && app.view === 'note'}
            onclick={() => app.openNote(item.id)}
          >
            <span class="title" class:untitled={!item.title.trim()}>
              {displayTitle({ title: item.title, kind: 'note' })}
            </span>
            {#if due}
              <span class="due" class:overdue={due.overdue} class:today={due.today}>{due.text}</span
              >
            {/if}
            {#if path}<span class="path">{path}</span>{/if}
          </button>
        </li>
      {/each}
    </ul>
  {:else}
    <p class="hint muted">
      Nothing for later. Open a note and click <strong>Later</strong> to keep it here, optionally with
      a due date.
    </p>
  {/if}
</div>

<style>
  .list {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 0 6px 24px;
  }

  ul {
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .item {
    display: grid;
    grid-template-columns: 1fr auto;
    gap: 0 8px;
    width: 100%;
    padding: 6px 10px;
    border: none;
    border-radius: 6px;
    background: none;
    text-align: left;
  }

  .item:hover {
    background: var(--bg-hover);
  }

  .item.open {
    background: var(--bg-selected);
  }

  .title {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .untitled {
    color: var(--text-muted);
    font-style: italic;
  }

  .due {
    font-size: 12px;
    color: var(--text-muted);
    white-space: nowrap;
    align-self: center;
  }

  .due.today {
    color: var(--accent);
    font-weight: 600;
  }

  .due.overdue {
    color: var(--danger);
    font-weight: 600;
  }

  .path {
    grid-column: 1 / -1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: 12px;
    color: var(--text-muted);
  }

  .divider {
    margin: 10px 10px 4px;
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    color: var(--text-muted);
  }

  .hint {
    padding: 8px 10px;
    font-size: 13px;
  }
</style>
