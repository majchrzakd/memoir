<script lang="ts">
  import { api } from '../api'
  import { app, errorMessage } from '../app.svelte'
  import Icon from '../ui/Icon.svelte'
  import { focusOnMount } from '../ui/focus'

  let mode = $state<'passphrase' | 'recovery'>('passphrase')
  let passphrase = $state('')
  let recoveryKey = $state('')
  let newPassphrase = $state('')
  let repeat = $state('')
  let remember = $state(true)
  let busy = $state(false)
  let error = $state<string | null>(null)

  async function submit(e: SubmitEvent) {
    e.preventDefault()
    error = null
    if (mode === 'recovery') {
      if ([...newPassphrase].length < 8) return void (error = 'Use at least 8 characters.')
      if (newPassphrase !== repeat) return void (error = "The passphrases don't match.")
    }
    busy = true
    try {
      if (mode === 'passphrase') await api.unlock(passphrase, remember)
      else await api.unlockWithRecoveryKey(recoveryKey, newPassphrase, remember)
      passphrase = recoveryKey = newPassphrase = repeat = ''
      await app.enter()
    } catch (err) {
      error = errorMessage(err)
    } finally {
      busy = false
    }
  }
</script>

<main class="screen">
  <form class="card" onsubmit={submit}>
    <div class="title">
      <Icon name="lock" size={22} />
      <h1>{mode === 'passphrase' ? 'Unlock Memoir' : 'Use your recovery key'}</h1>
    </div>

    {#if mode === 'passphrase'}
      <label>
        Passphrase
        <input
          type="password"
          bind:value={passphrase}
          autocomplete="current-password"
          use:focusOnMount
        />
      </label>
    {:else}
      <label>
        Recovery key
        <input
          type="text"
          bind:value={recoveryKey}
          spellcheck="false"
          autocomplete="off"
          use:focusOnMount
        />
      </label>
      <label>
        New passphrase
        <input type="password" bind:value={newPassphrase} autocomplete="new-password" />
      </label>
      <label>
        Repeat new passphrase
        <input type="password" bind:value={repeat} autocomplete="new-password" />
      </label>
    {/if}

    <label class="check">
      <input type="checkbox" bind:checked={remember} />
      Remember on this device
    </label>

    {#if error}<p class="error">{error}</p>{/if}

    <div class="actions">
      <button class="primary" type="submit" disabled={busy}>{busy ? 'Unlocking…' : 'Unlock'}</button
      >
      <button
        type="button"
        class="link"
        onclick={() => {
          mode = mode === 'passphrase' ? 'recovery' : 'passphrase'
          error = null
        }}
      >
        {mode === 'passphrase' ? 'Forgot your passphrase?' : 'Use passphrase instead'}
      </button>
    </div>
  </form>
</main>

<style>
  .screen {
    display: grid;
    place-items: center;
    height: 100vh;
    padding: 16px;
    background: var(--bg-sidebar);
  }

  .card {
    display: flex;
    flex-direction: column;
    gap: 12px;
    width: min(400px, 100%);
    padding: 28px;
    background: var(--bg-raised);
    border: 1px solid var(--border);
    border-radius: 12px;
    box-shadow: var(--shadow);
  }

  .title {
    display: flex;
    align-items: center;
    gap: 10px;
  }

  h1 {
    margin: 0;
    font-size: 20px;
  }

  label {
    display: flex;
    flex-direction: column;
    gap: 4px;
    font-weight: 500;
  }

  .check {
    flex-direction: row;
    align-items: center;
    gap: 8px;
    font-weight: normal;
  }

  p {
    margin: 0;
  }

  .actions {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }

  .link {
    padding: 0;
    border: none;
    background: none;
    color: var(--accent);
  }

  .link:hover:not(:disabled) {
    background: none;
    text-decoration: underline;
  }
</style>
