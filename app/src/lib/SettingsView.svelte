<script lang="ts">
  import { api, type CloudStatus, type Exported, type Imported } from './api'
  import { app, errorMessage } from './app.svelte'
  import { localDate, sinceTime } from './time'
  import SecuritySettings from './SecuritySettings.svelte'
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

  const syncText = $derived.by(() => {
    const s = app.sync
    const waiting = s.pending
      ? `${s.pending} change${s.pending === 1 ? '' : 's'} waiting to be uploaded.`
      : 'Everything is synced.'
    const last = s.last_synced_at ? ` Last synced ${sinceTime(s.last_synced_at)}.` : ''
    switch (s.state) {
      case 'syncing':
        return 'Syncing…'
      case 'offline':
        return "Can't reach Google Drive right now. Your changes are safe on this device and upload when the connection is back."
      case 'error':
        return s.message ?? 'Syncing failed.'
      default:
        return waiting + last
    }
  })

  let version = $state('')
  $effect(() => {
    api
      .appInfo()
      .then((info) => (version = info.version))
      .catch(() => {})
  })

  let checking = $state(false)
  let checkResult = $state<string | null>(null)

  async function checkForUpdate() {
    checking = true
    checkResult = null
    try {
      const found = await api.checkForUpdate()
      app.update = found
      app.updateDismissed = false
      checkResult = found ? null : 'You have the latest version.'
    } catch (e) {
      checkResult = `Couldn't check for updates: ${errorMessage(e)}`
    } finally {
      checking = false
    }
  }

  let importing = $state(false)
  let importProgress = $state(0)
  let imported = $state<Imported | null>(null)
  let importError = $state<string | null>(null)

  async function importEvernote() {
    importError = null
    imported = null
    importProgress = 0
    importing = true
    const stop = await api.onImportProgress((p) => (importProgress = p.notes))
    try {
      const result = await api.importEvernote()
      if (result) imported = result
    } catch (e) {
      importError = errorMessage(e)
    } finally {
      importing = false
      stop()
      // Imported notes appear in the tree (also after a partial failure).
      await app.refreshTree().catch(() => {})
      app.laterChanged()
    }
  }

  let exporting = $state(false)
  let exported = $state<Exported | null>(null)
  let exportError = $state<string | null>(null)

  async function exportNotebook() {
    exportError = null
    exporting = true
    try {
      await app.flushEditor()
      const result = await api.exportNotebook(`Memoir export ${localDate()}`)
      if (result) exported = result
    } catch (e) {
      exportError = errorMessage(e)
    } finally {
      exporting = false
    }
  }

  function plural(n: number, word: string) {
    return `${n} ${word}${n === 1 ? '' : 's'}`
  }

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
      <p class={app.sync.state === 'error' ? 'error' : 'muted'}>{syncText}</p>
      <div class="actions">
        <button disabled={app.sync.state === 'syncing'} onclick={() => app.syncNow()}>
          Sync now
        </button>
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

  <SecuritySettings />

  <article class="card">
    <header>
      <h2>Import</h2>
    </header>
    <p>
      Bring your notes over from Evernote. In Evernote, export each notebook as an
      <strong>ENEX</strong> file (right-click the notebook → Export notebook…). Each file becomes a folder
      here; notes keep their dates, tags, checklists, tables, images and attachments.
    </p>
    {#if importing}
      <p class="muted">Importing… {plural(importProgress, 'note')} so far.</p>
    {:else if imported}
      {@const s = imported.summary}
      <p>
        Imported {plural(s.notes, 'note')} with {plural(s.attachments, 'attachment')} into
        {imported.folders.map((f) => `“${f}”`).join(', ')}.
      </p>
      {#if s.encrypted_sections || s.skipped_attachments}
        <p class="muted">
          {#if s.encrypted_sections}
            {plural(s.encrypted_sections, 'encrypted section')} couldn't be imported (Evernote's own encryption).
          {/if}
          {#if s.skipped_attachments}
            {plural(s.skipped_attachments, 'attachment')} over 100 MB were left out.
          {/if}
          The notes say where.
        </p>
      {/if}
    {/if}
    <div class="actions">
      <button class="primary" disabled={importing} onclick={importEvernote}>
        {importing ? 'Importing…' : 'Import from Evernote…'}
      </button>
    </div>
    {#if importError}<p class="error">{importError}</p>{/if}
  </article>

  <article class="card">
    <header>
      <h2>Export</h2>
    </header>
    <p>
      Save the whole notebook as Markdown files that any editor can open: a folder per folder, a
      file per note (with its replies), attachments and the trash.
    </p>
    <p class="muted">
      The export is not encrypted. Keep it somewhere safe, or delete it when you're done.
    </p>
    {#if exported}
      {@const s = exported.summary}
      <p>
        Exported {plural(s.notes, 'note')}, {plural(s.folders, 'folder')} and
        {plural(s.attachments, 'attachment')} to <code>{exported.path}</code>.
      </p>
      {#if s.missing_attachments}
        <p class="muted">
          {plural(s.missing_attachments, 'attachment')} weren't on this device yet, so their links point
          nowhere in the export.
        </p>
      {/if}
    {/if}
    <div class="actions">
      <button class="primary" disabled={exporting} onclick={exportNotebook}>
        {exporting ? 'Exporting…' : 'Export to Markdown…'}
      </button>
      {#if exported}
        <button onclick={() => exported && api.showExport(exported.path)}>Show in folder</button>
      {/if}
    </div>
    {#if exportError}<p class="error">{exportError}</p>{/if}
  </article>

  <article class="card">
    <header>
      <h2>About</h2>
    </header>
    <p>Memoir {version}</p>
    <p class="muted">
      If something goes wrong, the log helps find out why. It records what Memoir did (like sync
      rounds and errors), never the content of your notes.
    </p>
    {#if app.update}
      <p>Memoir {app.update.version} is available.</p>
    {:else if checkResult}
      <p class="muted">{checkResult}</p>
    {/if}
    <div class="actions">
      {#if app.update}
        <button class="primary" onclick={() => app.installUpdate().catch((e) => app.fail(e))}>
          Restart to update
        </button>
      {:else}
        <button disabled={checking} onclick={checkForUpdate}>
          {checking ? 'Checking…' : 'Check for updates'}
        </button>
      {/if}
      <button onclick={() => api.showLogs().catch((e) => app.fail(e))}>Open log folder</button>
    </div>
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

  .card + .card {
    margin-top: 16px;
  }

  code {
    word-break: break-all;
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
