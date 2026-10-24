<script lang="ts">
  import { app } from '../app.svelte'
  import Icon from '../ui/Icon.svelte'
  import Tree from './Tree.svelte'
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

  <Tree />

  <footer>
    <button class="trash" class:active={app.view === 'trash'} onclick={() => (app.view = 'trash')}>
      <Icon name="trash" />
      <span>Trash</span>
    </button>
  </footer>
</nav>

<style>
  .sidebar {
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
    padding: 6px;
    border-top: 1px solid var(--border);
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
