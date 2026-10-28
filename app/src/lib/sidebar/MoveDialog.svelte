<script lang="ts">
  import { app } from '../app.svelte'
  import { touch } from '../platform'
  import { displayTitle, moveTargets } from '../tree'
  import Icon from '../ui/Icon.svelte'

  let { id }: { id: string } = $props()

  const item = $derived(app.byId.get(id))
  const targets = $derived(moveTargets(app.items, id))

  let list = $state<HTMLDivElement>()

  // Start on the first folder it can move to.
  $effect(() => {
    const frame = requestAnimationFrame(() =>
      list?.querySelector<HTMLButtonElement>('button:not(:disabled)')?.focus(),
    )
    return () => cancelAnimationFrame(frame)
  })

  const close = () => (app.movingId = null)

  function onkeydown(e: KeyboardEvent) {
    if (e.key === 'Escape') close()
    else if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
      const buttons = [
        ...(list?.querySelectorAll<HTMLButtonElement>('button:not(:disabled)') ?? []),
      ]
      const at = buttons.indexOf(document.activeElement as HTMLButtonElement)
      const next = e.key === 'ArrowDown' ? at + 1 : at < 0 ? buttons.length - 1 : at - 1
      buttons[Math.max(0, Math.min(buttons.length - 1, next))]?.focus()
      e.preventDefault()
    }
  }
</script>

{#if item}
  <div class="backdrop" role="presentation" onclick={close}>
    <div
      class="dialog"
      role="dialog"
      aria-modal="true"
      aria-label="Move to"
      tabindex="-1"
      onclick={(e) => e.stopPropagation()}
      {onkeydown}
    >
      <h2>Move “{displayTitle(item)}” to</h2>
      <div class="list" style:--row-height="{touch.current ? 44 : 30}px" bind:this={list}>
        {#each targets as target (target.id)}
          <button
            class="target"
            style:padding-left="{10 + target.depth * 16}px"
            disabled={target.current}
            onclick={() => app.moveInto(id, target.id)}
          >
            <Icon name={target.id ? 'folder' : 'folderOpen'} />
            <span class="title">{target.title}</span>
            {#if target.current}<span class="muted">(here now)</span>{/if}
          </button>
        {/each}
      </div>
      <div class="actions">
        <button onclick={close}>Cancel</button>
      </div>
    </div>
  </div>
{/if}

<style>
  .dialog {
    display: flex;
    flex-direction: column;
    width: min(420px, calc(100vw - 32px));
    max-height: 70vh;
    padding: 16px 12px 12px;
  }

  h2 {
    margin: 0 8px 10px;
    overflow: hidden;
    font-size: 15px;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .list {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
  }

  .target {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    height: var(--row-height);
    padding-right: 10px;
    border: none;
    border-radius: 6px;
    background: none;
    color: var(--text);
    text-align: left;
  }

  .target:hover:not(:disabled),
  .target:focus-visible {
    background: var(--bg-hover);
  }

  .target:disabled {
    opacity: 1;
    color: var(--text-muted);
  }

  .target :global(svg) {
    flex: none;
    color: var(--text-muted);
  }

  .title {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .actions {
    display: flex;
    justify-content: flex-end;
    margin-top: 10px;
  }
</style>
