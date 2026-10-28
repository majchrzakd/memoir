<script lang="ts">
  import { api, type Backlink, type Id } from '../api'
  import { app } from '../app.svelte'
  import { displayTitle } from '../tree'
  import Icon from '../ui/Icon.svelte'

  let { noteId }: { noteId: Id } = $props()

  let links = $state.raw<Backlink[]>([])

  // Reloaded when another note is opened (links are added from other notes,
  // so they're current by the time you come back) or sync brought changes.
  $effect(() => {
    const id = noteId
    void app.syncVersion
    let current = true
    api
      .backlinks(id)
      .then((found) => {
        if (current) links = found
      })
      .catch(() => {
        if (current) links = []
      })
    return () => (current = false)
  })

  // Another note's list never shows while this one's loads.
  $effect.pre(() => {
    void noteId
    links = []
  })
</script>

{#if links.length}
  <section class="backlinks" aria-label="Linked from">
    <h3><Icon name="link" size={14} /> Linked from</h3>
    <ul>
      {#each links as link (link.id)}
        <li>
          <button onclick={() => app.openLinkedNote(link.id)}>
            <Icon name="note" size={14} />
            {displayTitle({ title: link.title, kind: 'note' })}
          </button>
        </li>
      {/each}
    </ul>
  </section>
{/if}

<style>
  .backlinks {
    margin-top: 32px;
    cursor: default;
  }

  h3 {
    display: flex;
    align-items: center;
    gap: 6px;
    margin: 0 0 8px;
    color: var(--text-muted);
    font-size: 12px;
    font-weight: 600;
    letter-spacing: 0.02em;
    text-transform: uppercase;
  }

  ul {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    margin: 0;
    padding: 0;
    list-style: none;
  }

  button {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    max-width: 280px;
    padding: 3px 10px;
    border-radius: 999px;
    color: var(--text);
    font-size: 13px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  button :global(svg) {
    flex: none;
    color: var(--text-muted);
  }
</style>
