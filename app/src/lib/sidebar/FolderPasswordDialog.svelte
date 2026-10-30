<script lang="ts">
  import type { Id } from '../api'
  import { app, errorMessage, type FolderPasswordMode } from '../app.svelte'
  import { focusOnMount } from '../ui/focus'

  let { id, mode: initialMode }: { id: Id; mode: FolderPasswordMode } = $props()

  // svelte-ignore state_referenced_locally
  let mode = $state(initialMode)
  let current = $state('')
  let password = $state('')
  let repeat = $state('')
  let busy = $state(false)
  let error = $state<string | null>(null)

  const name = $derived(app.byId.get(id)?.title ?? 'this folder')
  const close = () => (app.folderPassword = null)

  const titles: Record<FolderPasswordMode, string> = {
    unlock: 'Unlock',
    set: 'Set a password for',
    change: 'Change the password of',
    remove: 'Remove the password of',
    forgot: 'Forgot the password of',
  }
  const asksCurrent = $derived(mode === 'unlock' || mode === 'change' || mode === 'remove')
  const asksNew = $derived(mode === 'set' || mode === 'change')
  const ready = $derived(
    (!asksCurrent || current !== '') &&
      (!asksNew || (password !== '' && password === repeat)) &&
      (mode !== 'forgot' || password !== ''),
  )

  function forgot() {
    mode = 'forgot'
    password = ''
    error = null
  }

  async function submit(e: SubmitEvent) {
    e.preventDefault()
    if (!ready || busy) return
    busy = true
    error = null
    try {
      if (mode === 'unlock') await app.unlockFolder(id, current)
      else if (mode === 'set') await app.setFolderPassword(id, null, password)
      else if (mode === 'change') await app.setFolderPassword(id, current, password)
      else if (mode === 'remove') await app.setFolderPassword(id, current, null)
      else await app.forgetFolderPassword(id, password)
    } catch (e) {
      error = errorMessage(e)
    } finally {
      busy = false
    }
  }
</script>

<div class="backdrop" role="presentation" onclick={close}>
  <div
    class="dialog"
    role="dialog"
    aria-modal="true"
    aria-label="Folder password"
    tabindex="-1"
    onclick={(e) => e.stopPropagation()}
    onkeydown={(e) => e.key === 'Escape' && close()}
  >
    <form onsubmit={submit}>
      <h2>{titles[mode]} “{name}”</h2>
      {#if mode === 'set'}
        <p class="muted">
          Its notes are hidden — in the list, search and Later — until you enter this password, and
          again each time Memoir locks. Your other devices ask for it too. It's a lock on top of
          your notebook's encryption, not a second one: your notebook passphrase can remove it.
        </p>
      {:else if mode === 'forgot'}
        <p class="muted">
          Enter your notebook passphrase to remove this folder's password. Its notes stay as they
          are.
        </p>
      {/if}

      {#key mode}
        {#if asksCurrent}
          <label>
            {mode === 'unlock' ? 'Password' : 'Current password'}
            <input type="password" autocomplete="off" bind:value={current} use:focusOnMount />
          </label>
        {/if}
        {#if asksNew}
          <label>
            {mode === 'change' ? 'New password' : 'Password'}
            {#if mode === 'set'}
              <input
                type="password"
                autocomplete="new-password"
                bind:value={password}
                use:focusOnMount
              />
            {:else}
              <input type="password" autocomplete="new-password" bind:value={password} />
            {/if}
          </label>
          <label>
            Repeat it
            <input type="password" autocomplete="new-password" bind:value={repeat} />
          </label>
          {#if repeat && password !== repeat}
            <p class="muted">The passwords don't match yet.</p>
          {/if}
        {/if}
        {#if mode === 'forgot'}
          <label>
            Notebook passphrase
            <input
              type="password"
              autocomplete="current-password"
              bind:value={password}
              use:focusOnMount
            />
          </label>
        {/if}
      {/key}

      {#if error}<p class="error">{error}</p>{/if}

      <div class="actions">
        {#if asksCurrent}
          <button type="button" class="link" onclick={forgot}>Forgot the password?</button>
        {/if}
        <span class="spacer"></span>
        <button type="button" onclick={close}>Cancel</button>
        <button
          class={mode === 'remove' || mode === 'forgot' ? 'danger' : 'primary'}
          type="submit"
          disabled={busy || !ready}
        >
          {mode === 'unlock'
            ? 'Unlock'
            : mode === 'set'
              ? 'Set password'
              : mode === 'change'
                ? 'Change password'
                : 'Remove password'}
        </button>
      </div>
    </form>
  </div>
</div>

<style>
  .dialog {
    width: min(420px, calc(100vw - 32px));
    padding: 20px;
  }

  form {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }

  h2 {
    margin: 0;
    font-size: 17px;
    overflow-wrap: anywhere;
  }

  p {
    margin: 0;
    font-size: 13px;
  }

  label {
    display: flex;
    flex-direction: column;
    gap: 4px;
    font-weight: 500;
  }

  .actions {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .spacer {
    flex: 1;
  }

  .link {
    padding: 0;
    border: none;
    background: none;
    color: var(--text-muted);
    font-size: 13px;
    text-decoration: underline;
  }
</style>
