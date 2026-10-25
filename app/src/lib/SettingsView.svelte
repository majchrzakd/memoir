<script lang="ts">
  import { api, type CloudStatus } from './api'
  import { app, errorMessage } from './app.svelte'
  import Icon from './ui/Icon.svelte'

  let cloud = $state<CloudStatus | null>(null)
  let busy = $state(false)
  let error = $state<string | null>(null)

  // Show the saved state at once, then verify it with Google.
  $effect(() => {
    void (async () => {
      try {
        cloud = await api.cloudStatus()
        if (cloud.connected) cloud = await api.checkDrive()
      } catch (e) {
        error = errorMessage(e)
      }
    })()
  })

  async function connect() {
    error = null
    busy = true
    if (cloud) cloud = { ...cloud, connecting: true }
    try {
      cloud = await api.connectDrive()
    } catch (e) {
      error = errorMessage(e)
      cloud = await api.cloudStatus()
    } finally {
      busy = false
    }
  }

  async function disconnect() {
    const ok = await app.confirm(
      'Disconnect Google Drive? Your notes stay on this device, and files already in Drive stay there.',
      'Disconnect',
    )
    if (!ok) return
    busy = true
    try {
      cloud = await api.disconnectDrive()
    } catch (e) {
      error = errorMessage(e)
    } finally {
      busy = false
    }
  }
</script>

<section class="settings">
  <h1>Settings</h1>

  <article class="card">
    <header>
      <h2>Google Drive</h2>
      {#if cloud?.connected && !cloud.problem}
        <span class="badge ok"><Icon name="check" size={14} /> Connected</span>
      {/if}
    </header>

    {#if !cloud}
      <p class="muted">Loading…</p>
    {:else if !cloud.configured}
      <p>
        This build of Memoir has no Google sign-in configured. Set
        <code>MEMOIR_GOOGLE_CLIENT_ID</code> and <code>MEMOIR_GOOGLE_CLIENT_SECRET</code> when
        building (see <code>docs/spikes/google-oauth.md</code>).
      </p>
    {:else if cloud.connecting || (busy && !cloud.connected)}
      <p>Waiting for you to sign in to Google in your browser…</p>
      <div class="actions">
        <button onclick={() => api.cancelConnect()}>Cancel</button>
      </div>
    {:else if cloud.connected}
      <p>
        Connected as <strong>{cloud.account}</strong>. Your notebook lives in the
        <strong>Memoir</strong> folder of that Drive — encrypted, so Google can't read it.
      </p>
      <p class="muted">Syncing notes is coming next; for now the folder is set up.</p>
      <div class="actions">
        <button class="danger" disabled={busy} onclick={disconnect}>Disconnect</button>
      </div>
    {:else}
      <p>
        Keep your notebook in your own Google Drive, encrypted before it leaves this device. Memoir
        can only see the files it creates there, nothing else in your Drive.
      </p>
      <div class="actions">
        <button class="primary" disabled={busy} onclick={connect}>Connect Google Drive</button>
      </div>
    {/if}

    {#if cloud?.problem}
      <p class="error">{cloud.problem}</p>
      {#if cloud.configured && !busy}
        <div class="actions">
          <button class="primary" onclick={connect}>Connect again</button>
        </div>
      {/if}
    {/if}
    {#if error}<p class="error">{error}</p>{/if}
  </article>
</section>

<style>
  .settings {
    height: 100%;
    overflow-y: auto;
    padding: 28px max(40px, calc((100% - 760px) / 2));
  }

  h1 {
    margin: 0 0 20px;
    font-size: 24px;
  }

  .card {
    padding: 18px 20px;
    border: 1px solid var(--border);
    border-radius: 10px;
  }

  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }

  h2 {
    margin: 0;
    font-size: 16px;
  }

  p {
    margin: 10px 0 0;
  }

  .badge {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 1px 8px;
    border-radius: 999px;
    font-size: 12px;
  }

  .badge.ok {
    background: var(--bg-selected);
  }

  code {
    padding: 0 4px;
    border-radius: 4px;
    background: var(--bg-code);
    font: 0.9em var(--mono);
  }

  .actions {
    margin-top: 14px;
  }
</style>
