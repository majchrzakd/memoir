<script lang="ts" module>
  export interface MenuItem {
    label: string
    shortcut?: string
    danger?: boolean
    action: () => void
  }
</script>

<script lang="ts">
  let { x, y, items, onclose }: { x: number; y: number; items: MenuItem[]; onclose: () => void } =
    $props()

  let menu = $state<HTMLDivElement>()
  let left = $state(0)
  let top = $state(0)

  // Keep the menu inside the window.
  $effect(() => {
    if (!menu) return
    const { width, height } = menu.getBoundingClientRect()
    left = Math.min(x, window.innerWidth - width - 4)
    top = Math.min(y, window.innerHeight - height - 4)
    menu.querySelector('button')?.focus()
  })

  function onkeydown(e: KeyboardEvent) {
    const buttons = [...(menu?.querySelectorAll('button') ?? [])]
    const i = buttons.indexOf(document.activeElement as HTMLButtonElement)
    if (e.key === 'Escape') onclose()
    else if (e.key === 'ArrowDown') buttons[(i + 1) % buttons.length]?.focus()
    else if (e.key === 'ArrowUp') buttons[(i - 1 + buttons.length) % buttons.length]?.focus()
    else return
    e.preventDefault()
  }
</script>

<svelte:window onblur={onclose} />

<div
  class="backdrop transparent"
  role="presentation"
  onclick={onclose}
  oncontextmenu={(e) => {
    e.preventDefault()
    onclose()
  }}
></div>
<div
  class="menu"
  role="menu"
  tabindex="-1"
  bind:this={menu}
  style:left="{left}px"
  style:top="{top}px"
  {onkeydown}
>
  {#each items as item (item.label)}
    <button
      role="menuitem"
      class:danger={item.danger}
      onclick={() => {
        onclose()
        item.action()
      }}
    >
      <span>{item.label}</span>
      {#if item.shortcut}<kbd>{item.shortcut}</kbd>{/if}
    </button>
  {/each}
</div>

<style>
  .transparent {
    background: transparent;
  }

  .menu {
    position: fixed;
    z-index: 101;
    min-width: 200px;
    padding: 4px;
    background: var(--bg-raised);
    border: 1px solid var(--border);
    border-radius: 8px;
    box-shadow: var(--shadow);
  }

  button {
    display: flex;
    width: 100%;
    justify-content: space-between;
    gap: 24px;
    padding: 6px 10px;
    border: none;
    background: none;
    border-radius: 5px;
    text-align: left;
  }

  button:hover,
  button:focus-visible {
    background: var(--bg-hover);
    outline: none;
  }

  button.danger {
    color: var(--danger);
  }

  kbd {
    color: var(--text-muted);
    font: inherit;
    font-size: 12px;
  }
</style>
