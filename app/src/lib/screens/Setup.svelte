<script lang="ts">
  import { api } from '../api'
  import { app, errorMessage } from '../app.svelte'
  import Icon from '../ui/Icon.svelte'
  import { focusOnMount } from '../ui/focus'

  let passphrase = $state('')
  let repeat = $state('')
  let busy = $state(false)
  let error = $state<string | null>(null)
  let recoveryKey = $state<string | null>(null)
  let saved = $state(false)
  let copied = $state(false)

  const length = $derived([...passphrase].length)
  const strength = $derived(
    length < 8 ? 'too short' : length < 12 ? 'okay' : length < 16 ? 'good' : 'strong',
  )

  async function create(e: SubmitEvent) {
    e.preventDefault()
    error = null
    if (length < 8) return void (error = 'Use at least 8 characters.')
    if (passphrase !== repeat) return void (error = "The passphrases don't match.")
    busy = true
    try {
      recoveryKey = await api.beginSetup(passphrase)
      passphrase = repeat = ''
    } catch (err) {
      error = errorMessage(err)
    } finally {
      busy = false
    }
  }

  async function copy() {
    if (!recoveryKey) return
    await navigator.clipboard.writeText(recoveryKey)
    copied = true
  }

  async function finish() {
    busy = true
    try {
      await api.finishSetup()
      recoveryKey = null
      await app.enter()
    } catch (err) {
      error = errorMessage(err)
    } finally {
      busy = false
    }
  }
</script>

<main class="screen">
  {#if !recoveryKey}
    <form class="card" onsubmit={create}>
      <h1>Create your notebook</h1>
      <p class="muted">
        Your notes are encrypted on this device with a key protected by your passphrase. Nobody else
        — not Google, not us — can read them.
      </p>

      <label>
        Passphrase
        <input
          type="password"
          bind:value={passphrase}
          autocomplete="new-password"
          use:focusOnMount
        />
      </label>
      <p class="hint" class:weak={length > 0 && length < 8}>
        {#if length > 0}Strength: {strength}.{/if} A few unrelated words make a strong, memorable passphrase.
      </p>
      <label>
        Repeat passphrase
        <input type="password" bind:value={repeat} autocomplete="new-password" />
      </label>

      {#if error}<p class="error">{error}</p>{/if}
      <button class="primary" type="submit" disabled={busy}>
        {busy ? 'Creating…' : 'Continue'}
      </button>
    </form>
  {:else}
    <section class="card">
      <h1>Save your recovery key</h1>
      <p>
        If you forget your passphrase, this key is the <strong>only</strong> way to get your notes back.
        Store it in a password manager or write it down and keep it somewhere safe. It won't be shown
        again.
      </p>
      <div class="key">
        <code>{recoveryKey}</code>
        <button class="icon" title="Copy" onclick={copy}>
          <Icon name={copied ? 'check' : 'copy'} size={18} />
        </button>
      </div>
      <label class="check">
        <input type="checkbox" bind:checked={saved} />
        I have saved my recovery key somewhere safe
      </label>
      {#if error}<p class="error">{error}</p>{/if}
      <button class="primary" disabled={!saved || busy} onclick={finish}>
        {busy ? 'Opening…' : 'Open my notebook'}
      </button>
    </section>
  {/if}
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
    width: min(460px, 100%);
    padding: 28px;
    background: var(--bg-raised);
    border: 1px solid var(--border);
    border-radius: 12px;
    box-shadow: var(--shadow);
  }

  h1 {
    margin: 0;
    font-size: 20px;
  }

  p {
    margin: 0;
  }

  label {
    display: flex;
    flex-direction: column;
    gap: 4px;
    font-weight: 500;
  }

  .hint {
    margin-top: -6px;
    font-size: 12px;
    color: var(--text-muted);
  }

  .hint.weak {
    color: var(--danger);
  }

  .key {
    display: flex;
    align-items: center;
    gap: 8px;
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

  .check {
    flex-direction: row;
    align-items: center;
    gap: 8px;
    font-weight: normal;
  }

  button.primary {
    align-self: flex-start;
  }
</style>
