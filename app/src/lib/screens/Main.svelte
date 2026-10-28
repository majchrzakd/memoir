<script lang="ts">
  import { onMount } from 'svelte'
  import { api, type MenuCommand } from '../api'
  import { app } from '../app.svelte'
  import Editor from '../editor/Editor.svelte'
  import { keys, phone, touch } from '../platform'
  import SearchPalette from '../SearchPalette.svelte'
  import Sidebar from '../sidebar/Sidebar.svelte'
  import SettingsView from '../SettingsView.svelte'
  import TrashView from '../TrashView.svelte'
  import Icon from '../ui/Icon.svelte'

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

  function run(command: MenuCommand) {
    switch (command) {
      case 'search':
        app.searchOpen = true
        break
      case 'new-folder':
        void app.newFolder()
        break
      case 'new-note':
        void app.newNote()
        break
      case 'lock':
        void app.lock()
        break
      case 'settings':
        app.show('settings')
        break
      default: {
        // Every command is handled (compile error otherwise).
        const unhandled: never = command
        console.warn('unknown menu command', unhandled)
      }
    }
  }

  function onkeydown(e: KeyboardEvent) {
    const ctrl = e.ctrlKey || e.metaKey
    if (ctrl && (e.key === 'k' || e.key === 'p')) {
      run('search')
    } else if (ctrl && e.shiftKey && e.key.toLowerCase() === 'n') {
      run('new-folder')
    } else if (ctrl && e.key === 'n') {
      run('new-note')
    } else if (ctrl && !e.shiftKey && e.key.toLowerCase() === 'l') {
      run('lock')
    } else {
      return
    }
    e.preventDefault()
  }

  // macOS menu bar; its shortcuts never reach onkeydown.
  onMount(() => {
    const stop = api.onMenuCommand(run)
    return () => void stop.then((unlisten) => unlisten())
  })

  // Auto-lock (Settings → Security): any use of the window counts as activity.
  onMount(() => {
    const active = () => app.noteActivity()
    const events = ['pointerdown', 'pointermove', 'keydown', 'wheel', 'touchstart'] as const
    for (const name of events) window.addEventListener(name, active, { passive: true })
    const timer = setInterval(() => app.checkIdle(), 15_000)
    return () => {
      for (const name of events) window.removeEventListener(name, active)
      clearInterval(timer)
    }
  })
</script>

<svelte:window {onkeydown} />

<!-- On a phone one pane shows at a time; both stay mounted so the editor keeps its state. -->
<div
  class="layout"
  class:resizing
  class:phone={phone.current}
  class:list-shown={app.listShown}
  style:grid-template-columns={phone.current ? null : `${width}px 0 1fr`}
>
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
    {#if phone.current}
      <div class="phone-bar">
        <button class="back" onclick={() => app.back()}>
          <Icon name="chevron" size={16} />Notes
        </button>
      </div>
    {/if}
    <div class="pane">
      {#if app.view === 'trash'}
        <TrashView />
      {:else if app.view === 'settings'}
        <SettingsView />
      {:else if app.openNoteId}
        <Editor noteId={app.openNoteId} />
      {:else if touch.current}
        <div class="empty muted"><p>Select a note, or tap the new-note button.</p></div>
      {:else}
        <div class="empty muted">
          <p>Select a note, or press <kbd>{keys('Ctrl+N')}</kbd> to create one.</p>
        </div>
      {/if}
    </div>
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
    height: 100dvh;
    /* iPhone notch and home indicator (zero elsewhere). */
    padding: env(safe-area-inset-top) env(safe-area-inset-right) env(safe-area-inset-bottom)
      env(safe-area-inset-left);
  }

  .layout.phone {
    grid-template-columns: minmax(0, 1fr);
  }

  /* The strips behind the status bar and home indicator match the list. */
  .layout.phone.list-shown {
    background: var(--bg-sidebar);
  }

  .layout.phone > .splitter,
  .layout.phone.list-shown > .content,
  .layout.phone:not(.list-shown) > :global(.sidebar) {
    display: none;
  }

  .phone-bar {
    flex: none;
    padding: 4px 8px;
    border-bottom: 1px solid var(--border);
  }

  .back {
    display: inline-flex;
    align-items: center;
    gap: 2px;
    min-height: 44px;
    padding: 0 8px 0 2px;
    border: none;
    background: none;
    color: var(--accent);
  }

  .back :global(svg) {
    transform: rotate(180deg);
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
    display: flex;
    flex-direction: column;
    min-width: 0;
    overflow: hidden;
  }

  .pane {
    position: relative;
    flex: 1;
    min-height: 0;
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
