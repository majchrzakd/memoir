<script lang="ts">
  import type { Workspace } from './api'
  import { app } from './app.svelte'
  import Icon from './ui/Icon.svelte'
  import { focusOnMount } from './ui/focus'

  let renaming = $state<string | null>(null)
  let name = $state('')

  function startRename(w: Workspace) {
    renaming = w.id
    name = w.name
  }

  async function rename(e: SubmitEvent) {
    e.preventDefault()
    const id = renaming
    renaming = null
    if (id && name.trim()) await app.renameWorkspace(id, name.trim())
  }

  async function remove(w: Workspace) {
    const ok = await app.confirm(
      `Remove “${w.name}” from this device? Its notes on this device are deleted; the copy in its ` +
        `Google Drive stays, and you can open it again from there. Changes made in it that ` +
        `weren't uploaded yet are lost.`,
      'Remove',
    )
    if (ok) await app.removeWorkspace(w.id)
  }
</script>

<article class="card">
  <header>
    <h2>Workspaces</h2>
  </header>
  <p class="muted">
    Each workspace is a separate notebook with its own Google Drive account, passphrase and notes —
    for example one for work and one for personal notes. Switch between them at the top of the
    sidebar. The settings below are for <strong>{app.workspaceName}</strong>.
  </p>
  <ul>
    {#each app.workspaces.workspaces as w (w.id)}
      <li>
        {#if renaming === w.id}
          <form onsubmit={rename}>
            <input
              type="text"
              maxlength="60"
              aria-label="Workspace name"
              bind:value={name}
              use:focusOnMount
              onkeydown={(e) => e.key === 'Escape' && (renaming = null)}
            />
            <button class="primary" type="submit" disabled={!name.trim()}>Save</button>
            <button type="button" onclick={() => (renaming = null)}>Cancel</button>
          </form>
        {:else}
          <span class="name">
            {w.name}
            {#if w.id === app.workspaces.active}<span class="badge">Open</span>{/if}
          </span>
          {#if w.id !== app.workspaces.active}
            <button onclick={() => app.switchWorkspace(w.id)}>Open</button>
          {/if}
          <button class="icon" title="Rename" aria-label="Rename" onclick={() => startRename(w)}>
            <Icon name="edit" />
          </button>
          {#if w.id !== app.workspaces.active}
            <button
              class="icon danger"
              title="Remove from this device"
              aria-label="Remove from this device"
              onclick={() => remove(w)}
            >
              <Icon name="trash" />
            </button>
          {/if}
        {/if}
      </li>
    {/each}
  </ul>
  <div class="actions">
    <button onclick={() => (app.newWorkspaceOpen = true)}>New workspace…</button>
  </div>
</article>

<style>
  .card {
    margin-bottom: 16px;
    padding: 18px 20px;
    border: 1px solid var(--border);
    border-radius: 10px;
  }

  h2 {
    margin: 0;
    font-size: 16px;
  }

  p {
    margin: 10px 0 0;
  }

  ul {
    margin: 12px 0 0;
    padding: 0;
    list-style: none;
  }

  li {
    display: flex;
    align-items: center;
    gap: 6px;
    min-height: 40px;
    border-top: 1px solid var(--border);
  }

  li form {
    display: flex;
    flex: 1;
    gap: 6px;
  }

  li input {
    flex: 1;
    min-width: 0;
  }

  .name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .badge {
    margin-left: 6px;
    padding: 1px 8px;
    border-radius: 999px;
    background: var(--bg-selected);
    font-size: 12px;
  }

  .danger {
    color: var(--danger);
  }

  .actions {
    margin-top: 14px;
  }
</style>
