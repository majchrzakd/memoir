<script lang="ts">
  import { app, errorMessage } from '../app.svelte'
  import Icon from '../ui/Icon.svelte'

  let installing = $state(false)
  let error = $state<string | null>(null)

  async function install() {
    error = null
    installing = true
    try {
      await app.installUpdate()
    } catch (e) {
      error = errorMessage(e)
      installing = false
    }
  }
</script>

{#if app.update && !app.updateDismissed}
  <aside class="update" aria-label="Update">
    <div class="head">
      <Icon name="sync" size={15} />
      <strong>Memoir {app.update.version} is available</strong>
      <button
        class="close"
        title="Not now"
        aria-label="Not now"
        disabled={installing}
        onclick={() => (app.updateDismissed = true)}>×</button
      >
    </div>
    {#if app.update.notes}<p>{app.update.notes}</p>{/if}
    {#if error}<p class="error">{error}</p>{/if}
    <button class="primary" disabled={installing} onclick={install}>
      {installing ? 'Installing…' : 'Restart to update'}
    </button>
  </aside>
{/if}

<style>
  .update {
    display: flex;
    flex-direction: column;
    gap: 6px;
    margin: 6px 8px;
    padding: 10px 12px;
    border: 1px solid var(--border);
    border-radius: 8px;
    background: var(--bg-raised);
    font-size: 13px;
  }

  .head {
    display: flex;
    align-items: center;
    gap: 6px;
    color: var(--accent);
  }

  strong {
    flex: 1;
    color: var(--text);
    font-weight: 600;
  }

  p {
    margin: 0;
    color: var(--text-muted);
  }

  .close {
    padding: 0 4px;
    border: none;
    background: none;
    color: var(--text-muted);
    font-size: 16px;
    line-height: 1;
  }

  .primary {
    align-self: flex-start;
    padding: 4px 10px;
    font-size: 13px;
  }
</style>
