<script lang="ts">
  import { app } from '../app.svelte'
  import ContextMenu from '../ui/ContextMenu.svelte'
  import Icon from '../ui/Icon.svelte'

  let menu = $state<{ x: number; y: number } | null>(null)

  function open(e: MouseEvent) {
    const rect = (e.currentTarget as HTMLElement).getBoundingClientRect()
    menu = { x: rect.left, y: rect.bottom + 4 }
  }
</script>

<button
  class="switcher"
  title="Switch workspace"
  aria-haspopup="menu"
  aria-expanded={menu !== null}
  onclick={open}
>
  <span class="name">{app.workspaceName}</span>
  <Icon name="chevronDown" size={14} />
</button>

{#if menu}
  <ContextMenu
    x={menu.x}
    y={menu.y}
    items={[
      ...app.workspaces.workspaces.map((w) => ({
        label: w.name,
        checked: w.id === app.workspaces.active,
        action: () => void app.switchWorkspace(w.id),
      })),
      { label: 'New workspace…', separator: true, action: () => (app.newWorkspaceOpen = true) },
    ]}
    onclose={() => (menu = null)}
  />
{/if}

<style>
  .switcher {
    display: inline-flex;
    flex: 0 1 auto;
    align-items: center;
    margin-right: auto;
    gap: 4px;
    min-width: 0;
    margin-left: -8px;
    padding: 4px 8px;
    border: none;
    background: none;
    color: var(--text);
    font-weight: 600;
    font-size: 15px;
    text-align: left;
  }

  .switcher:hover {
    background: var(--bg-hover);
  }

  .switcher :global(svg) {
    flex: none;
    color: var(--text-muted);
  }

  .name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
