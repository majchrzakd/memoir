<script lang="ts" module>
  export interface MenuItem {
    label: string
    shortcut?: string
    danger?: boolean
    /** Shows a check mark (e.g. the current choice). */
    checked?: boolean
    /** Draws a line above the item. */
    separator?: boolean
    action: () => void
  }
</script>

<script lang="ts">
  import Icon from './Icon.svelte'

  let { x, y, items, onclose }: { x: number; y: number; items: MenuItem[]; onclose: () => void } =
    $props()

  let menu = $state<HTMLDivElement>()
  /** Leave room for check marks when any item can have one. */
  const checked = $derived(items.some((i) => i.checked !== undefined))
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
  {#each items as item, i (i)}
    <button
      class:danger={item.danger}
      class:separator={item.separator}
      role={item.checked === undefined ? 'menuitem' : 'menuitemradio'}
      aria-checked={item.checked}
      onclick={() => {
        onclose()
        item.action()
      }}
    >
      <span class="label">
        {#if checked}<span class="check"
            >{#if item.checked}<Icon name="check" size={14} />{/if}</span
          >{/if}
        {item.label}
      </span>
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

  button.separator {
    position: relative;
    margin-top: 9px;
  }

  button.separator::before {
    content: '';
    position: absolute;
    top: -5px;
    right: 4px;
    left: 4px;
    border-top: 1px solid var(--border);
  }

  .label {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
  }

  .check {
    display: inline-flex;
    width: 14px;
    color: var(--accent);
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
