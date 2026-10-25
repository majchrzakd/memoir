<script lang="ts">
  import { api, type LaterItem } from '../api'
  import { app } from '../app.svelte'
  import Icon from '../ui/Icon.svelte'
  import LaterList from './LaterList.svelte'
  import Tree from './Tree.svelte'

  let later = $state.raw<LaterItem[]>([])

  // Reload when notes change (titles, trash) or Later flags change.
  $effect(() => {
    void app.items
    void app.laterVersion
    api
      .laterNotes()
      .then((list) => (later = list))
      .catch((e) => app.fail(e))
  })
</script>

<nav class="sidebar" aria-label="Notebook">
  <header>
    <span class="brand">Memoir</span>
    <button class="icon" title="New note (Ctrl+N)" onclick={() => app.newNote()}>
      <Icon name="notePlus" size={18} />
    </button>
    <button class="icon" title="New folder (Ctrl+Shift+N)" onclick={() => app.newFolder()}>
      <Icon name="folderPlus" size={18} />
    </button>
  </header>

  <button class="search" onclick={() => (app.searchOpen = true)}>
    <Icon name="search" />
    <span>Search</span>
    <kbd>Ctrl+K</kbd>
  </button>

  <div class="tabs" role="tablist" aria-label="Sidebar">
    <button
      role="tab"
      aria-selected={app.sidebarTab === 'notes'}
      onclick={() => (app.sidebarTab = 'notes')}
    >
      Notes
    </button>
    <button
      role="tab"
      aria-selected={app.sidebarTab === 'later'}
      onclick={() => (app.sidebarTab = 'later')}
    >
      Later
      {#if later.length}<span class="count">{later.length}</span>{/if}
    </button>
  </div>

  {#if app.sidebarTab === 'notes'}
    <Tree />
  {:else}
    <LaterList items={later} />
  {/if}

  <footer>
    <button class="trash" class:active={app.view === 'trash'} onclick={() => (app.view = 'trash')}>
      <Icon name="trash" />
      <span>Trash</span>
    </button>
    <button
      class="icon"
      class:active={app.view === 'settings'}
      title="Settings"
      aria-label="Settings"
      onclick={() => (app.view = 'settings')}
    >
      <Icon name="settings" size={18} />
    </button>
  </footer>
</nav>

<style>
  .tabs {
    display: flex;
    gap: 2px;
    margin: 0 10px 6px;
    padding: 2px;
    border-radius: 7px;
    background: var(--bg-hover);
  }

  .tabs button {
    flex: 1;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
    padding: 3px 8px;
    border: none;
    border-radius: 5px;
    background: none;
    color: var(--text-muted);
    font-size: 13px;
  }

  .tabs button[aria-selected='true'] {
    background: var(--bg-raised);
    color: var(--text);
    box-shadow: 0 1px 2px rgb(0 0 0 / 0.15);
  }

  .count {
    min-width: 18px;
    padding: 0 5px;
    border-radius: 9px;
    background: var(--accent);
    color: var(--accent-text);
    font-size: 11px;
    font-weight: 600;
    line-height: 16px;
  }

  .sidebar {
    /* Lets the tree hide edit times when the sidebar is narrow. */
    container-type: inline-size;
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
    height: 100%;
    background: var(--bg-sidebar);
  }

  header {
    display: flex;
    align-items: center;
    gap: 2px;
    padding: 12px 10px 8px 16px;
  }

  .brand {
    flex: 1;
    font-weight: 600;
    font-size: 15px;
  }

  .search {
    display: flex;
    align-items: center;
    gap: 8px;
    margin: 0 10px 8px;
    padding: 6px 10px;
    color: var(--text-muted);
    background: var(--bg);
    text-align: left;
  }

  .search span {
    flex: 1;
  }

  kbd {
    font: inherit;
    font-size: 11px;
  }

  footer {
    display: flex;
    gap: 4px;
    padding: 6px;
    border-top: 1px solid var(--border);
  }

  footer .icon.active {
    background: var(--bg-selected);
    color: var(--text);
  }

  .trash {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    padding: 6px 10px;
    border: none;
    background: none;
    color: var(--text-muted);
  }

  .trash.active {
    background: var(--bg-selected);
    color: var(--text);
  }
</style>
