<script lang="ts">
  import { app } from '../app.svelte'
  import { sinceTime } from '../time'
  import Icon, { type IconName } from '../ui/Icon.svelte'

  /** Current time, refreshed so "synced 2 min ago" stays true. */
  let now = $state(Date.now())
  $effect(() => {
    const clock = setInterval(() => (now = Date.now()), 30_000)
    return () => clearInterval(clock)
  })

  const view = $derived.by((): { icon: IconName; label: string; tip: string } => {
    const s = app.sync
    const last = s.last_synced_at ? `Last synced ${sinceTime(s.last_synced_at, now)}.` : ''
    const waiting = s.pending
      ? `${s.pending} change${s.pending === 1 ? '' : 's'} waiting to be uploaded.`
      : ''
    switch (s.state) {
      case 'syncing':
        return { icon: 'sync', label: 'Syncing…', tip: 'Syncing with Google Drive.' }
      case 'offline':
        return {
          icon: 'cloudOff',
          label: 'Offline',
          tip: `Can't reach Google Drive; Memoir keeps trying. Your changes are safe on this device. ${waiting} ${last}`,
        }
      case 'error':
        return {
          icon: 'alert',
          label: 'Sync problem',
          tip: `${s.message ?? 'Syncing failed.'} ${waiting}`,
        }
      default:
        return s.pending
          ? { icon: 'cloud', label: `${s.pending} to sync`, tip: `${waiting} ${last}` }
          : { icon: 'check', label: 'Synced', tip: `Everything is in Google Drive. ${last}` }
    }
  })

  function onclick() {
    if (app.sync.state === 'error') app.view = 'settings'
    else app.syncNow()
  }
</script>

{#if app.sync.state !== 'off'}
  <button
    class="sync {app.sync.state}"
    title="{view.tip.trim()}{app.sync.state === 'error' ? '' : '\nClick to sync now.'}"
    {onclick}
  >
    <span class="glyph"><Icon name={view.icon} size={15} /></span>
    <span class="label">{view.label}</span>
  </button>
{/if}

<style>
  .sync {
    display: inline-flex;
    flex: none;
    align-items: center;
    gap: 6px;
    padding: 6px 8px;
    border: none;
    background: none;
    color: var(--text-muted);
    font-size: 12px;
    white-space: nowrap;
  }

  .sync:hover {
    background: var(--bg-hover);
    color: var(--text);
  }

  .glyph {
    display: inline-flex;
  }

  .syncing .glyph {
    animation: spin 1.2s linear infinite;
  }

  .error {
    color: var(--danger, #c0392b);
  }

  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .syncing .glyph {
      animation: none;
    }
  }

  /* Narrow sidebar: icon only (the tooltip says the rest). */
  @container (max-width: 230px) {
    .label {
      display: none;
    }
  }
</style>
