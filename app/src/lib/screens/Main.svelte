<script lang="ts">
  import { app } from '../app.svelte'
  import Editor from '../editor/Editor.svelte'
  import SearchPalette from '../SearchPalette.svelte'
  import Sidebar from '../sidebar/Sidebar.svelte'
  import SettingsView from '../SettingsView.svelte'
  import TrashView from '../TrashView.svelte'

  const WIDTH_KEY = 'memoir.sidebarWidth'
  const MIN_WIDTH = 200
  const MAX_WIDTH = 560

  function storedWidth(): number {
    try {
      return Number(localStorage.getItem(WIDTH_KEY)) || 280
    } catch {
      return 280
    }
  }

  let width = $state(storedWidth())
  let resizing = $state(false)

  function startResize(e: PointerEvent) {
    resizing = true
    ;(e.currentTarget as HTMLElement).setPointerCapture(e.pointerId)
  }

  function resize(e: PointerEvent) {
    if (resizing) width = Math.round(Math.min(MAX_WIDTH, Math.max(MIN_WIDTH, e.clientX)))
  }

  function endResize() {
    resizing = false
    try {
      localStorage.setItem(WIDTH_KEY, String(width))
    } catch {
      // preference only
    }
  }

  function onkeydown(e: KeyboardEvent) {
    const ctrl = e.ctrlKey || e.metaKey
    if (ctrl && (e.key === 'k' || e.key === 'p')) {
      app.searchOpen = true
    } else if (ctrl && e.shiftKey && e.key.toLowerCase() === 'n') {
      void app.newFolder()
    } else if (ctrl && e.key === 'n') {
      void app.newNote()
    } else {
      return
    }
    e.preventDefault()
  }
</script>

<svelte:window {onkeydown} />

<div class="layout" class:resizing style:grid-template-columns="{width}px 0 1fr">
  <Sidebar />
  <div
    class="splitter"
    role="separator"
    aria-orientation="vertical"
    aria-label="Resize sidebar"
    onpointerdown={startResize}
    onpointermove={resize}
    onpointerup={endResize}
  ></div>
  <main class="content">
    {#if app.view === 'trash'}
      <TrashView />
    {:else if app.view === 'settings'}
      <SettingsView />
    {:else if app.openNoteId}
      <Editor noteId={app.openNoteId} />
    {:else}
      <div class="empty muted">
        <p>Select a note, or press <kbd>Ctrl+N</kbd> to create one.</p>
      </div>
    {/if}
  </main>
</div>

{#if app.searchOpen}
  <SearchPalette />
{/if}

<style>
  .layout {
    display: grid;
    /* One row exactly the window's height; panes scroll inside it. */
    grid-template-rows: minmax(0, 1fr);
    height: 100vh;
  }

  .layout.resizing {
    cursor: col-resize;
    user-select: none;
  }

  .splitter {
    position: relative;
    z-index: 1;
    width: 0;
    border-left: 1px solid var(--border);
  }

  .splitter::after {
    content: '';
    position: absolute;
    inset: 0 -4px;
    cursor: col-resize;
  }

  .content {
    position: relative;
    min-width: 0;
    overflow: hidden;
  }

  .empty {
    display: grid;
    place-items: center;
    height: 100%;
  }

  kbd {
    padding: 1px 5px;
    border: 1px solid var(--border);
    border-radius: 4px;
    font: inherit;
    font-size: 12px;
  }
</style>
