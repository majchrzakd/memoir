<script lang="ts">
  import { onMount } from 'svelte'
  import { api } from './lib/api'
  import { app } from './lib/app.svelte'
  import Main from './lib/screens/Main.svelte'
  import Setup from './lib/screens/Setup.svelte'
  import Unlock from './lib/screens/Unlock.svelte'
  import ConfirmDialog from './lib/ui/ConfirmDialog.svelte'

  onMount(() => {
    void app.start()
    // When the window closes, save what the editor hasn't saved yet.
    const unlisten = api.onFlushRequest(() => app.flushEditor())
    return () => void unlisten.then((stop) => stop())
  })
</script>

{#if app.screen === 'first_run' || app.screen === 'confirm_recovery_key'}
  <Setup />
{:else if app.screen === 'locked'}
  <Unlock />
{:else if app.screen === 'unlocked'}
  <Main />
{/if}

<ConfirmDialog />

{#if app.error}
  <div class="toast" role="alert">
    <span>{app.error}</span>
    <button class="icon" aria-label="Dismiss" onclick={() => (app.error = null)}>✕</button>
  </div>
{/if}

<style>
  .toast {
    position: fixed;
    left: 50%;
    bottom: 24px;
    z-index: 200;
    display: flex;
    align-items: center;
    gap: 12px;
    max-width: min(560px, calc(100vw - 32px));
    padding: 10px 12px 10px 16px;
    transform: translateX(-50%);
    background: var(--bg-raised);
    border: 1px solid var(--danger);
    border-radius: 8px;
    box-shadow: var(--shadow);
  }
</style>
