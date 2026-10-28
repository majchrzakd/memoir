<script lang="ts">
  import { api } from '../api'
  import { app, errorMessage } from '../app.svelte'
  import Icon from '../ui/Icon.svelte'
  import { focusOnMount } from '../ui/focus'
  import JoinFromDrive from './JoinFromDrive.svelte'

  let passphrase = $state('')
  let repeat = $state('')
  let busy = $state(false)
  let error = $state<string | null>(null)
  let recoveryKey = $state<string | null>(null)
  let saved = $state(false)
  let copied = $state(false)
  /** First a choice: make a new notebook, or open one that's in Google
   * Drive already (only offered when this build can sign in to Google). */
  let mode = $state<'loading' | 'choose' | 'create' | 'join'>('loading')
  let canJoin = $state(false)
  /** Setting up a workspace added next to others (it can be cancelled). */
  const adding = $derived(app.previousWorkspace !== null)

  $effect(() => {
    api
      .cloudStatus()
      .then((c) => (canJoin = c.configured))
      .catch(() => {})
      .finally(() => (mode = canJoin ? 'choose' : 'create'))
  })

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
  {#if mode === 'loading'}
    <!-- Decided in a moment; nothing to show meanwhile. -->
  {:else if mode === 'choose'}
    <section class="card">
      {#if adding}
        <h1>New workspace: {app.workspaceName}</h1>
        <p class="muted">
          Create a new notebook for this workspace, or open one you already keep in another Google
          account's Drive — your work account, for example.
        </p>
      {:else}
        <h1>Welcome to Memoir</h1>
        <p class="muted">Is this your first notebook, or do you already use Memoir elsewhere?</p>
      {/if}
      <div class="choices">
        <button class="choice" onclick={() => (mode = 'create')} use:focusOnMount>
          <Icon name="notePlus" size={22} />
          <span>
            <strong>Create a new notebook</strong>
            <small>Start fresh on this computer. You can connect Google Drive later.</small>
          </span>
        </button>
        <button class="choice" onclick={() => (mode = 'join')}>
          <Icon name="cloud" size={22} />
          <span>
            <strong>Open my notebook from Google Drive</strong>
            <small>You already use Memoir on another computer.</small>
          </span>
        </button>
      </div>
      {#if adding}
        <button class="link cancel" onclick={() => app.cancelNewWorkspace()}>
          Cancel — don't add this workspace
        </button>
      {/if}
    </section>
  {:else if mode === 'join'}
    <JoinFromDrive onback={() => (mode = 'choose')} />
  {:else if !recoveryKey}
    <form class="card" onsubmit={create}>
      <h1>{adding ? `Create the notebook for ${app.workspaceName}` : 'Create your notebook'}</h1>
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
      <div class="actions">
        <button class="primary" type="submit" disabled={busy}>
          {busy ? 'Creating…' : 'Continue'}
        </button>
        {#if canJoin}
          <button type="button" class="link" disabled={busy} onclick={() => (mode = 'choose')}>
            Back
          </button>
        {:else if adding}
          <button
            type="button"
            class="link"
            disabled={busy}
            onclick={() => app.cancelNewWorkspace()}
          >
            Cancel
          </button>
        {/if}
      </div>
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

  .choices {
    display: flex;
    flex-direction: column;
    gap: 10px;
    margin-top: 4px;
  }

  .choice {
    display: flex;
    align-items: center;
    gap: 14px;
    padding: 14px 16px;
    border: 1px solid var(--border);
    border-radius: 10px;
    background: var(--bg);
    color: var(--text);
    text-align: left;
  }

  .choice:hover {
    border-color: var(--accent);
    background: var(--bg-hover);
  }

  .choice :global(svg) {
    flex: none;
    color: var(--accent);
  }

  .choice span {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .choice small {
    color: var(--text-muted);
    font-size: 13px;
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
    font-size: inherit;
  }

  .cancel {
    align-self: flex-start;
    margin-top: 4px;
  }

  .link:hover {
    background: none;
    text-decoration: underline;
  }
</style>
