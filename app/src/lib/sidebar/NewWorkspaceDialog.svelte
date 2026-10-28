<script lang="ts">
  import { app } from '../app.svelte'
  import { focusOnMount } from '../ui/focus'

  let name = $state('')
  const close = () => (app.newWorkspaceOpen = false)

  function submit(e: SubmitEvent) {
    e.preventDefault()
    if (!name.trim()) return
    close()
    void app.addWorkspace(name.trim())
  }
</script>

<div class="backdrop" role="presentation" onclick={close}>
  <div
    class="dialog"
    role="dialog"
    aria-modal="true"
    aria-label="New workspace"
    tabindex="-1"
    onclick={(e) => e.stopPropagation()}
    onkeydown={(e) => e.key === 'Escape' && close()}
  >
    <form onsubmit={submit}>
      <h2>New workspace</h2>
      <p class="muted">
        A workspace is a separate notebook with its own Google Drive account and passphrase — for
        example one for work and one for personal notes.
      </p>
      <label>
        Name
        <input type="text" placeholder="Work" maxlength="60" bind:value={name} use:focusOnMount />
      </label>
      <div class="actions">
        <button type="button" onclick={close}>Cancel</button>
        <button class="primary" type="submit" disabled={!name.trim()}>Continue</button>
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
    justify-content: flex-end;
    gap: 8px;
  }
</style>
