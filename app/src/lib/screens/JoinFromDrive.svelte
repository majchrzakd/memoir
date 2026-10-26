<script lang="ts">
  import { api } from '../api'
  import { app, errorMessage } from '../app.svelte'
  import { focusOnMount } from '../ui/focus'

  let { onback }: { onback: () => void } = $props()

  /** idle → signing in (browser) → found (ask passphrase) → opening */
  let step = $state<'idle' | 'signing' | 'found' | 'opening'>('idle')
  let account = $state('')
  let passphrase = $state('')
  let remember = $state(true)
  let error = $state<string | null>(null)

  async function signIn() {
    error = null
    step = 'signing'
    try {
      account = await api.findDriveNotebook()
      step = 'found'
    } catch (e) {
      error = errorMessage(e)
      step = 'idle'
    }
  }

  async function open(e: SubmitEvent) {
    e.preventDefault()
    error = null
    step = 'opening'
    try {
      await api.openDriveNotebook(passphrase, remember)
      passphrase = ''
      await app.enter()
    } catch (err) {
      error = errorMessage(err)
      step = 'found'
    }
  }

  async function back() {
    if (step === 'signing') await api.cancelConnect()
    if (step === 'found') await api.forgetDriveNotebook()
    onback()
  }
</script>

{#if step === 'found' || step === 'opening'}
  <form class="card" onsubmit={open}>
    <h1>Open your notebook</h1>
    <p class="muted">
      Found your notebook in the Google Drive of <strong>{account}</strong>. Enter the passphrase
      you use for it; your notes then download in the background.
    </p>
    <label>
      Passphrase
      <input
        type="password"
        bind:value={passphrase}
        autocomplete="current-password"
        use:focusOnMount
      />
    </label>
    <label class="check">
      <input type="checkbox" bind:checked={remember} />
      Remember on this device
    </label>
    {#if error}<p class="error">{error}</p>{/if}
    <div class="actions">
      <button class="primary" type="submit" disabled={step === 'opening' || !passphrase}>
        {step === 'opening' ? 'Opening…' : 'Open notebook'}
      </button>
      <button type="button" class="link" disabled={step === 'opening'} onclick={back}>Back</button>
    </div>
  </form>
{:else}
  <section class="card">
    <h1>Open your notebook from Google Drive</h1>
    <p class="muted">
      Already use Memoir on another computer? Sign in to the Google account your notebook is in.
      Memoir finds it in the Memoir folder of that Drive; you then unlock it with your passphrase.
    </p>
    {#if step === 'signing'}
      <p>Waiting for you to sign in to Google in your browser…</p>
    {/if}
    {#if error}<p class="error">{error}</p>{/if}
    <div class="actions">
      <button class="primary" disabled={step === 'signing'} onclick={signIn}>
        {step === 'signing' ? 'Signing in…' : 'Sign in with Google'}
      </button>
      <button type="button" class="link" onclick={back}>
        {step === 'signing' ? 'Cancel' : 'Back'}
      </button>
    </div>
  </section>
{/if}

<style>
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

  .check {
    flex-direction: row;
    align-items: center;
    gap: 8px;
    font-weight: normal;
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
