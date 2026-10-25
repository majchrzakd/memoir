<script lang="ts">
  import { api, type SearchHit } from './api'
  import { app } from './app.svelte'
  import { displayTitle, folderPath, snippetParts } from './tree'
  import Icon from './ui/Icon.svelte'
  import { focusOnMount } from './ui/focus'

  const LIMIT = 50
  const DEBOUNCE_MS = 60

  let query = $state('')
  let hits = $state.raw<SearchHit[]>([])
  let active = $state(0)
  let list = $state<HTMLUListElement>()
  let timer: ReturnType<typeof setTimeout> | undefined
  let latest = 0

  function close() {
    app.searchOpen = false
  }

  function oninput() {
    clearTimeout(timer)
    timer = setTimeout(run, DEBOUNCE_MS)
  }

  async function run() {
    const request = ++latest
    try {
      const result = await api.search(query, LIMIT)
      if (request !== latest) return // a newer query is on its way
      hits = result
      active = 0
    } catch (e) {
      app.fail(e)
    }
  }

  /** Several hits can share a note (matches in its replies). */
  function hitKey(hit: SearchHit): string {
    return hit.reply_id ?? hit.id
  }

  async function open(hit: SearchHit | undefined) {
    if (!hit) return
    close()
    app.focusReplyId = hit.reply_id
    await app.openNote(hit.id)
  }

  function onkeydown(e: KeyboardEvent) {
    if (e.key === 'Escape') close()
    else if (e.key === 'ArrowDown') active = Math.min(active + 1, hits.length - 1)
    else if (e.key === 'ArrowUp') active = Math.max(active - 1, 0)
    else if (e.key === 'Enter') void open(hits[active])
    else return
    e.preventDefault()
  }

  $effect(() => {
    list?.querySelector('[aria-selected="true"]')?.scrollIntoView({ block: 'nearest' })
  })
</script>

<div class="backdrop" role="presentation" onclick={close}>
  <div
    class="dialog palette"
    role="dialog"
    aria-modal="true"
    aria-label="Search notes"
    tabindex="-1"
    onclick={(e) => e.stopPropagation()}
    {onkeydown}
  >
    <div class="field">
      <Icon name="search" size={18} />
      <input
        type="search"
        placeholder="Search notes…"
        bind:value={query}
        {oninput}
        use:focusOnMount
        role="combobox"
        aria-expanded={hits.length > 0}
        aria-controls="search-results"
        aria-activedescendant={hits[active] ? `hit-${hitKey(hits[active])}` : undefined}
      />
    </div>

    {#if hits.length}
      <ul id="search-results" role="listbox" bind:this={list}>
        {#each hits as hit, i (hitKey(hit))}
          {@const path = folderPath(app.byId, hit.id)}
          <li
            id="hit-{hitKey(hit)}"
            role="option"
            aria-selected={i === active}
            onmousemove={() => (active = i)}
            onclick={() => open(hit)}
            onkeydown={() => {}}
          >
            <div class="title">
              {displayTitle({ title: hit.title, kind: 'note' })}
              {#if hit.reply_id}<span class="in-reply">reply</span>{/if}
            </div>
            {#if path}<div class="path">{path}</div>{/if}
            <div class="snippet">
              {#each snippetParts(hit.snippet) as part, p (p)}
                {#if part.match}<mark>{part.text}</mark>{:else}{part.text}{/if}
              {/each}
            </div>
          </li>
        {/each}
      </ul>
    {:else if query.trim()}
      <p class="none muted">No notes match "{query.trim()}".</p>
    {/if}
  </div>
</div>

<style>
  .palette {
    display: flex;
    flex-direction: column;
    width: min(640px, calc(100vw - 32px));
    max-height: 70vh;
    overflow: hidden;
  }

  .field {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 12px 16px;
    border-bottom: 1px solid var(--border);
    color: var(--text-muted);
  }

  .field input {
    padding: 2px 0;
    border: none;
    background: none;
    font-size: 16px;
    outline: none;
  }

  ul {
    margin: 0;
    padding: 6px;
    overflow-y: auto;
    list-style: none;
  }

  li {
    padding: 8px 10px;
    border-radius: 6px;
    cursor: pointer;
  }

  li[aria-selected='true'] {
    background: var(--bg-selected);
  }

  .title {
    font-weight: 600;
  }

  .in-reply {
    margin-left: 6px;
    padding: 0 6px;
    border-radius: 4px;
    background: var(--bg-hover);
    color: var(--text-muted);
    font-size: 11px;
    font-weight: normal;
  }

  .path {
    font-size: 12px;
    color: var(--text-muted);
  }

  .snippet {
    margin-top: 2px;
    font-size: 13px;
    color: var(--text-muted);
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }

  mark {
    background: var(--mark);
    color: var(--text);
    border-radius: 2px;
  }

  .none {
    padding: 16px;
    margin: 0;
  }
</style>
