<script lang="ts">
  import { api, type TreeItem } from './api'
  import { app } from './app.svelte'
  import { displayTitle } from './tree'
  import Icon from './ui/Icon.svelte'

  let items = $state.raw<TreeItem[]>([])

  async function refresh() {
    items = await api.trashedItems()
  }

  $effect(() => {
    void refresh().catch((e) => app.fail(e))
  })

  async function restore(item: TreeItem) {
    await app.run(async () => {
      await api.restore(item.id)
      await Promise.all([refresh(), app.refreshTree()])
      app.reveal(item.id)
    })
  }

  async function purge(item: TreeItem) {
    const what =
      item.kind === 'folder'
        ? `the folder "${displayTitle(item)}" and everything in it`
        : `"${displayTitle(item)}"`
    if (!(await app.confirm(`Delete ${what} forever? This can't be undone.`, 'Delete forever')))
      return
    await app.run(async () => {
      await api.purge(item.id)
      await refresh()
    })
  }

  async function emptyTrash() {
    if (
      !(await app.confirm(
        `Delete all ${items.length} items in the trash forever? This can't be undone.`,
        'Empty trash',
      ))
    )
      return
    await app.run(async () => {
      await api.emptyTrash()
      await refresh()
    })
  }

  const dateFormat = new Intl.DateTimeFormat(undefined, { dateStyle: 'medium', timeStyle: 'short' })
</script>

<section class="trash">
  <header>
    <h1>Trash</h1>
    <button class="danger" disabled={!items.length} onclick={emptyTrash}>Empty trash</button>
  </header>
  <p class="muted hint">Items in the trash are deleted for good after 30 days.</p>

  {#if items.length}
    <ul>
      {#each items as item (item.id)}
        <li>
          <Icon name={item.kind === 'folder' ? 'folder' : 'note'} />
          <span class="title">{displayTitle(item)}</span>
          <span class="muted date">{item.trashed_at ? dateFormat.format(item.trashed_at) : ''}</span
          >
          <button class="icon" title="Restore" onclick={() => restore(item)}
            ><Icon name="restore" /></button
          >
          <button class="icon" title="Delete forever" onclick={() => purge(item)}
            ><Icon name="trash" /></button
          >
        </li>
      {/each}
    </ul>
  {:else}
    <p class="muted">The trash is empty.</p>
  {/if}
</section>

<style>
  .trash {
    height: 100%;
    overflow-y: auto;
    padding: 28px max(40px, calc((100% - 760px) / 2));
  }

  @media (max-width: 700px) {
    .trash {
      padding: 16px;
    }
  }

  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }

  h1 {
    margin: 0;
    font-size: 24px;
  }

  .hint {
    margin: 4px 0 20px;
  }

  ul {
    margin: 0;
    padding: 0;
    list-style: none;
  }

  li {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 6px 8px;
    border-bottom: 1px solid var(--border);
  }

  li :global(svg) {
    flex: none;
  }

  .title {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .date {
    font-size: 12px;
  }
</style>
