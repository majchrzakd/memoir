<script lang="ts">
  import { app } from '../app.svelte'

  let confirmButton = $state<HTMLButtonElement>()

  $effect(() => {
    if (app.confirmRequest) confirmButton?.focus()
  })

  function onkeydown(e: KeyboardEvent) {
    if (e.key === 'Escape') app.confirmRequest?.resolve(false)
  }
</script>

{#if app.confirmRequest}
  {@const request = app.confirmRequest}
  <div class="backdrop" role="presentation" onclick={() => request.resolve(false)}>
    <div
      class="dialog"
      role="alertdialog"
      aria-modal="true"
      aria-label="Confirm"
      tabindex="-1"
      onclick={(e) => e.stopPropagation()}
      {onkeydown}
    >
      <p>{request.message}</p>
      <div class="actions">
        <button onclick={() => request.resolve(false)}>Cancel</button>
        <button class="danger" bind:this={confirmButton} onclick={() => request.resolve(true)}>
          {request.confirmLabel}
        </button>
      </div>
    </div>
  </div>
{/if}

<style>
  .dialog {
    width: min(420px, calc(100vw - 32px));
    padding: 20px;
  }

  p {
    margin: 0 0 20px;
  }

  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }
</style>
