<script lang="ts">
  import { Editor } from '@tiptap/core'
  import { onMount } from 'svelte'
  import { api, type HistoryEntry, type Id, type NoteText } from '../api'
  import { app } from '../app.svelte'
  import { fullTime, sinceTime } from '../time'
  import { editorExtensions } from './extensions'
  import { replaceMarkdown } from './replaceMarkdown'

  let {
    noteId,
    current,
    onclose,
    onrestore,
  }: {
    noteId: Id
    /** The note as the editor shows it now, listed first. */
    current: NoteText
    onclose: () => void
    onrestore: (savedAt: number) => Promise<void>
  } = $props()

  /** An entry in the list: an earlier version, or the current text (no saved_at). */
  type Item = HistoryEntry | (NoteText & { saved_at: null })

  // svelte-ignore state_referenced_locally
  const currentItem: Item = { ...current, saved_at: null }
  let versions = $state.raw<HistoryEntry[] | null>(null)
  const items = $derived<Item[]>([currentItem, ...(versions ?? [])])
  // Raw: compared by identity with the list entries.
  let selected = $state.raw<Item | null>(null)
  let busy = $state(false)
  let host = $state<HTMLDivElement>()
  let closeButton = $state<HTMLButtonElement>()
  let preview: Editor | null = null
  const now = Date.now()

  onMount(() => {
    closeButton?.focus()
    void api
      .noteHistory(noteId)
      .then((list) => {
        versions = list
        // The newest earlier version: what restoring would most likely bring back.
        show(list[0] ?? currentItem)
      })
      .catch((e) => {
        app.fail(e)
        onclose()
      })
    return () => preview?.destroy()
  })

  function show(version: Item) {
    selected = version
    if (!host) return
    if (!preview) {
      // Read-only, rendered like the note itself.
      preview = new Editor({
        element: host,
        extensions: editorExtensions(app.editorOptions()),
        content: version.body,
        contentType: 'markdown',
        editable: false,
        editorProps: { attributes: { class: 'prose' } },
      })
    } else {
      replaceMarkdown(preview, version.body)
    }
  }

  async function restore() {
    if (selected?.saved_at == null) return
    busy = true
    try {
      await onrestore(selected.saved_at)
      onclose()
    } catch (e) {
      app.fail(e)
    } finally {
      busy = false
    }
  }

  function onkeydown(e: KeyboardEvent) {
    if (e.key === 'Escape') onclose()
    if (!selected || (e.key !== 'ArrowDown' && e.key !== 'ArrowUp')) return
    e.preventDefault()
    const i = items.indexOf(selected) + (e.key === 'ArrowDown' ? 1 : -1)
    if (i >= 0 && i < items.length) show(items[i])
  }
</script>

<div class="backdrop" role="presentation" onclick={onclose}>
  <div
    class="dialog history"
    role="dialog"
    aria-modal="true"
    aria-label="History"
    tabindex="-1"
    onclick={(e) => e.stopPropagation()}
    {onkeydown}
  >
    <header>
      <h2>History</h2>
    </header>
    <div class="body">
      <ul class="list" aria-label="Versions">
        <li>
          <button
            class="current"
            class:selected={selected === currentItem}
            aria-current={selected === currentItem}
            onclick={() => show(currentItem)}
          >
            <span>Current version</span>
            <small>What the note says now</small>
          </button>
        </li>
        {#if versions && !versions.length}
          <li class="empty">
            No earlier versions yet. Memoir keeps one when you come back to a note after a 10-minute
            pause, and every 10 minutes while you keep editing (the last 50). Changes from other
            devices are kept the same way.
          </li>
        {/if}
        {#each versions ?? [] as version (version.saved_at)}
          {@const since = sinceTime(version.saved_at, now)}
          {@const full = fullTime(version.saved_at)}
          <li>
            <button
              class:selected={selected === version}
              aria-current={selected === version}
              title={full}
              onclick={() => show(version)}
            >
              <span>{since}</span>
              {#if full !== since}<small>{full}</small>{/if}
            </button>
          </li>
        {/each}
      </ul>
      <div class="preview">
        {#if selected}
          <h3>{selected.title || 'Untitled'}</h3>
        {/if}
        <div bind:this={host}></div>
      </div>
    </div>
    <footer>
      <span class="muted">Kept on this device only.</span>
      <span class="spacer"></span>
      <button bind:this={closeButton} onclick={onclose}>Close</button>
      <button
        class="primary"
        disabled={selected?.saved_at == null || busy}
        title={selected?.saved_at == null ? 'This is the current version' : undefined}
        onclick={restore}
      >
        {busy ? 'Restoring…' : 'Restore this version'}
      </button>
    </footer>
  </div>
</div>

<style>
  .history {
    display: flex;
    flex-direction: column;
    width: min(880px, calc(100vw - 32px));
    height: min(640px, calc(100vh - 64px));
    padding: 0;
  }

  header,
  footer {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 14px 18px;
  }

  header {
    border-bottom: 1px solid var(--border);
  }

  footer {
    border-top: 1px solid var(--border);
  }

  h2 {
    margin: 0;
    font-size: 16px;
  }

  .spacer {
    flex: 1;
  }

  .body {
    display: grid;
    grid-template-columns: 220px minmax(0, 1fr);
    min-height: 0;
    flex: 1;
  }

  .list {
    margin: 0;
    padding: 8px;
    overflow-y: auto;
    list-style: none;
    border-right: 1px solid var(--border);
  }

  .list button {
    display: flex;
    flex-direction: column;
    width: 100%;
    padding: 6px 10px;
    border: none;
    background: none;
    text-align: left;
  }

  .list button:hover {
    background: var(--bg-hover);
  }

  .list button.selected {
    background: var(--bg-selected);
  }

  .list small {
    color: var(--text-muted);
    font-size: 12px;
  }

  .empty {
    padding: 6px 10px;
    color: var(--text-muted);
    font-size: 12px;
  }

  .current {
    margin-bottom: 4px;
    border-bottom: 1px solid var(--border) !important;
    border-radius: 6px 6px 0 0;
  }

  .preview {
    padding: 18px 24px;
    overflow-y: auto;
  }

  h3 {
    margin: 0 0 12px;
    font-size: 20px;
  }
</style>
