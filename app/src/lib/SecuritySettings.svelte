<script lang="ts">
  import { api } from './api'
  import { app, errorMessage } from './app.svelte'
  import { keys } from './platform'
  import Icon from './ui/Icon.svelte'

  const AUTO_LOCK_CHOICES = [
    [0, 'Never'],
    [5, 'After 5 minutes'],
    [15, 'After 15 minutes'],
    [30, 'After 30 minutes'],
    [60, 'After 1 hour'],
  ] as const

  // Change passphrase
  let current = $state('')
  let next = $state('')
  let repeat = $state('')
  let changing = $state(false)
  let changeError = $state<string | null>(null)
  let changed = $state(false)

  const length = $derived([...next].length)
  const strength = $derived(
    length < 8 ? 'too short' : length < 12 ? 'okay' : length < 16 ? 'good' : 'strong',
  )

  async function changePassphrase(e: SubmitEvent) {
    e.preventDefault()
    changeError = null
    changed = false
    if (length < 8) return void (changeError = 'Use at least 8 characters.')
    if (next !== repeat) return void (changeError = "The new passphrases don't match.")
    changing = true
    try {
      await api.changePassphrase(current, next)
      current = next = repeat = ''
      changed = true
    } catch (err) {
      changeError = errorMessage(err)
    } finally {
      changing = false
    }
  }

  // New recovery key
  let keyPassphrase = $state('')
  let creating = $state(false)
  let keyError = $state<string | null>(null)
  let recoveryKey = $state<string | null>(null)
  let saved = $state(false)
  let copied = $state(false)

  async function newRecoveryKey(e: SubmitEvent) {
    e.preventDefault()
    keyError = null
    creating = true
    try {
      recoveryKey = await api.newRecoveryKey(keyPassphrase)
      keyPassphrase = ''
      saved = copied = false
    } catch (err) {
      keyError = errorMessage(err)
    } finally {
      creating = false
    }
  }

  async function copy() {
    if (!recoveryKey) return
    await navigator.clipboard.writeText(recoveryKey)
    copied = true
  }
</script>

<article class="card">
  <header>
    <h2>Security</h2>
  </header>

  <h3>Lock</h3>
  <p class="muted">
    Locking closes your notebook until you enter your passphrase, even on this computer. Lock any
    time with the lock button in the sidebar or {keys('Ctrl+L')}.
  </p>
  <label class="row">
    Lock automatically when not used
    <select
      value={app.autoLockMinutes}
      onchange={(e) => app.setAutoLock(Number(e.currentTarget.value))}
    >
      {#each AUTO_LOCK_CHOICES as [minutes, label] (minutes)}
        <option value={minutes}>{label}</option>
      {/each}
    </select>
  </label>

  <h3>Passphrase</h3>
  <form onsubmit={changePassphrase}>
    <label>
      Current passphrase
      <input type="password" bind:value={current} autocomplete="current-password" />
    </label>
    <label>
      New passphrase
      <input type="password" bind:value={next} autocomplete="new-password" />
    </label>
    <p class="hint" class:weak={length > 0 && length < 8}>
      {#if length > 0}Strength: {strength}.{/if} A few unrelated words make a strong, memorable passphrase.
    </p>
    <label>
      Repeat new passphrase
      <input type="password" bind:value={repeat} autocomplete="new-password" />
    </label>
    {#if changeError}<p class="error">{changeError}</p>{/if}
    {#if changed}
      <p class="ok">
        Passphrase changed. Your other devices switch to it the next time they open Memoir and sync;
        until then they still use the old one. Your recovery key keeps working.
      </p>
    {/if}
    <div class="actions">
      <button class="primary" type="submit" disabled={changing || !current || !next}>
        {changing ? 'Changing…' : 'Change passphrase'}
      </button>
    </div>
  </form>

  <h3>Recovery key</h3>
  {#if recoveryKey}
    <p>
      Your new recovery key. Store it in a password manager or write it down; it won't be shown
      again. The old key no longer works (on your other devices once they've synced).
    </p>
    <div class="key">
      <code>{recoveryKey}</code>
      <button class="icon" title="Copy" aria-label="Copy" onclick={copy}>
        <Icon name={copied ? 'check' : 'copy'} size={18} />
      </button>
    </div>
    <label class="check">
      <input type="checkbox" bind:checked={saved} />
      I have saved my new recovery key somewhere safe
    </label>
    <div class="actions">
      <button class="primary" disabled={!saved} onclick={() => (recoveryKey = null)}>Done</button>
    </div>
  {:else}
    <p class="muted">
      Lost your recovery key, or someone may have seen it? Create a new one; the old one stops
      working.
    </p>
    <form onsubmit={newRecoveryKey}>
      <label>
        Passphrase
        <input type="password" bind:value={keyPassphrase} autocomplete="current-password" />
      </label>
      {#if keyError}<p class="error">{keyError}</p>{/if}
      <div class="actions">
        <button type="submit" disabled={creating || !keyPassphrase}>
          {creating ? 'Creating…' : 'Create a new recovery key'}
        </button>
      </div>
    </form>
  {/if}
</article>

<style>
  .card {
    margin: 16px 0;
    padding: 18px 20px;
    border: 1px solid var(--border);
    border-radius: 10px;
  }

  h2 {
    margin: 0;
    font-size: 16px;
  }

  h3 {
    margin: 22px 0 0;
    font-size: 14px;
  }

  p {
    margin: 8px 0 0;
  }

  form {
    display: flex;
    flex-direction: column;
    gap: 10px;
    max-width: 420px;
    margin-top: 10px;
  }

  label {
    display: flex;
    flex-direction: column;
    gap: 4px;
    font-weight: 500;
  }

  label.row {
    flex-direction: row;
    align-items: center;
    gap: 10px;
    margin-top: 10px;
    font-weight: normal;
  }

  .check {
    flex-direction: row;
    align-items: center;
    gap: 8px;
    margin-top: 10px;
    font-weight: normal;
  }

  .hint {
    margin: -6px 0 0;
    font-size: 12px;
    color: var(--text-muted);
  }

  .hint.weak {
    color: var(--danger);
  }

  .ok {
    color: var(--text-muted);
  }

  .key {
    display: flex;
    align-items: center;
    gap: 8px;
    max-width: 520px;
    margin-top: 10px;
    padding: 12px;
    background: var(--bg-code);
    border-radius: 8px;
  }

  .key code {
    flex: 1;
    font: 15px/1.6 var(--mono);
    word-spacing: 4px;
    letter-spacing: 0.5px;
    user-select: all;
  }

  .actions {
    display: flex;
    gap: 8px;
    margin-top: 6px;
  }
</style>
