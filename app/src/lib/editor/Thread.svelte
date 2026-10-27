<script lang="ts">
  import { onMount, tick, untrack } from 'svelte'
  import { api, type Id, type Reply } from '../api'
  import { app } from '../app.svelte'
  import Composer from './Composer.svelte'
  import ReplyCard from './ReplyCard.svelte'

  let { noteId }: { noteId: Id } = $props()

  let replies = $state.raw<Reply[]>([])
  let loaded = $state(false)
  let highlightId = $state<Id | null>(null)
  const cards: Record<Id, ReplyCard | undefined> = {}

  onMount(async () => {
    try {
      replies = await api.replies(noteId)
      loaded = true
    } catch (e) {
      app.fail(e)
    }
  })

  // Other devices may have added, changed or deleted replies.
  let seenSyncVersion = app.syncVersion
  $effect(() => {
    const version = app.syncVersion
    untrack(async () => {
      if (version === seenSyncVersion || !loaded) return
      seenSyncVersion = version
      try {
        replies = await api.replies(noteId)
      } catch {
        // The note itself is gone; the editor handles that.
      }
    })
  })

  // Scroll to a reply chosen in search (also when the note was already open).
  $effect(() => {
    const target = app.focusReplyId
    if (!loaded || !target || !replies.some((r) => r.id === target)) return
    app.focusReplyId = null
    void reveal(target)
  })

  async function reveal(id: Id) {
    highlightId = id
    await tick()
    document.getElementById(`reply-${id}`)?.scrollIntoView({ block: 'center', behavior: 'smooth' })
    setTimeout(() => (highlightId = null), 1800)
  }

  /** Saves edits still waiting in reply cards. */
  export async function flush() {
    await Promise.all(Object.values(cards).map((c) => c?.flush()))
  }

  async function add(body: string) {
    const id = await api.addReply(noteId, body)
    const now = Date.now()
    replies = [...replies, { id, body, created_at: now, updated_at: now }]
    app.replyCountChanged(noteId, replies.length)
    await tick()
    document.getElementById(`reply-${id}`)?.scrollIntoView({ block: 'nearest' })
  }

  async function remove(reply: Reply) {
    if (!(await app.confirm("Delete this reply? This can't be undone.", 'Delete reply'))) return
    await app.run(async () => {
      await api.deleteReply(reply.id)
      replies = replies.filter((r) => r.id !== reply.id)
      delete cards[reply.id]
      app.replyCountChanged(noteId, replies.length)
    })
  }
</script>

<section class="thread" aria-label="Replies">
  <header>
    <h2>
      Replies
      {#if replies.length}<span class="count">{replies.length}</span>{/if}
    </h2>
    <div class="layout" role="group" aria-label="Show replies">
      <button
        aria-pressed={app.threadLayout === 'below'}
        title="Show replies below the note"
        onclick={() => app.setThreadLayout('below')}
      >
        Below
      </button>
      <button
        aria-pressed={app.threadLayout === 'side'}
        title="Show replies in a column beside the note (on wide windows)"
        onclick={() => app.setThreadLayout('side')}
      >
        Side
      </button>
    </div>
  </header>

  <div class="list">
    {#each replies as reply (reply.id)}
      <ReplyCard
        {reply}
        highlighted={reply.id === highlightId}
        ondelete={() => remove(reply)}
        bind:this={cards[reply.id]}
      />
    {/each}
  </div>

  <Composer {noteId} onsubmit={add} />
</section>

<style>
  .thread {
    cursor: default;
  }

  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    margin-bottom: 6px;
  }

  h2 {
    display: flex;
    align-items: center;
    gap: 6px;
    margin: 0;
    font-size: 13px;
    font-weight: 600;
    color: var(--text-muted);
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }

  .count {
    padding: 0 6px;
    border-radius: 9px;
    background: var(--bg-hover);
    color: var(--text);
    font-size: 11px;
    line-height: 17px;
  }

  .layout {
    display: flex;
    padding: 2px;
    border-radius: 6px;
    background: var(--bg-hover);
  }

  /* Phones never have room for the side column. */
  @media (max-width: 700px) {
    .layout {
      display: none;
    }
  }

  .layout button {
    padding: 1px 9px;
    border: none;
    border-radius: 4px;
    background: none;
    color: var(--text-muted);
    font-size: 12px;
  }

  .layout button[aria-pressed='true'] {
    background: var(--bg-raised);
    color: var(--text);
  }

  .list {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
</style>
