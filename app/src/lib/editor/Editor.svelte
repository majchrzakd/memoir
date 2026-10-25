<script lang="ts">
  import { Editor } from '@tiptap/core'
  import { onMount } from 'svelte'
  import { api, type Id, type Note } from '../api'
  import { app } from '../app.svelte'
  import { fullTime, sinceTime } from '../time'
  import Icon from '../ui/Icon.svelte'
  import { editorExtensions } from './extensions'
  import Thread from './Thread.svelte'
  import Toolbar from './Toolbar.svelte'
  import './prose.css'

  let { noteId }: { noteId: Id } = $props()

  const AUTOSAVE_MS = 300

  let host = $state<HTMLDivElement>()
  let titleInput = $state<HTMLInputElement>()
  let editor = $state.raw<Editor | null>(null)
  let title = $state('')
  /** Dates and Later state of the open note. */
  let meta = $state<Pick<Note, 'created_at' | 'updated_at' | 'later_at' | 'due_date'> | null>(null)
  let status = $state<'saved' | 'unsaved' | 'saving' | 'error'>('saved')
  /** Bumped on every editor transaction so the toolbar re-reads its state. */
  let tick = $state(0)
  /** Current time, refreshed so "Edited 5 min ago" stays true. */
  let now = $state(Date.now())
  let loadError = $state<string | null>(null)

  /** The thread goes in a side column only when there is room for both. */
  const SIDE_MIN_WIDTH = 900
  let width = $state(0)
  const side = $derived(app.threadLayout === 'side' && width >= SIDE_MIN_WIDTH)
  let thread = $state<Thread>()

  let loadedId: Id | null = null
  let dirty = false
  let timer: ReturnType<typeof setTimeout> | undefined
  let queue: Promise<void> = Promise.resolve()

  function scheduleSave() {
    dirty = true
    status = 'unsaved'
    clearTimeout(timer)
    timer = setTimeout(() => void flush(), AUTOSAVE_MS)
  }

  /** Saves pending edits now. Saves run one at a time, in order, so an older
   * save can never overwrite a newer one. */
  function flush(): Promise<void> {
    clearTimeout(timer)
    queue = queue.then(async () => {
      if (!dirty || !loadedId || !editor) return
      dirty = false
      const id = loadedId
      const savedTitle = title
      const body = editor.getMarkdown()
      status = 'saving'
      try {
        await api.saveNote(id, savedTitle, body)
        const savedAt = Date.now()
        if (meta && id === loadedId) meta.updated_at = savedAt
        app.noteSaved(id, savedTitle, savedAt)
        if (!dirty) status = 'saved'
      } catch (e) {
        dirty = true
        status = 'error'
        app.fail(e)
      }
    })
    return queue
  }

  async function load(id: Id) {
    await flush()
    if (id !== noteId) return // another note was chosen meanwhile
    try {
      const note = await api.note(id)
      if (id !== noteId || !host) return
      // A fresh editor per note: undo history never crosses notes.
      editor?.destroy()
      loadError = null
      title = note.title
      meta = {
        created_at: note.created_at,
        updated_at: note.updated_at,
        later_at: note.later_at,
        due_date: note.due_date,
      }
      loadedId = id
      status = 'saved'
      editor = new Editor({
        element: host,
        extensions: editorExtensions(),
        content: note.body,
        contentType: 'markdown',
        editorProps: { attributes: { class: 'prose', spellcheck: 'true' } },
        onUpdate: scheduleSave,
        onTransaction: () => tick++,
      })
      if (!note.title && !note.body) titleInput?.focus()
    } catch (e) {
      loadError = String(e)
    }
  }

  async function setLater(later: boolean, dueDate: string | null) {
    if (!loadedId || !meta) return
    const id = loadedId
    try {
      await api.setLater(id, later, later ? dueDate : null)
      if (id !== loadedId) return
      meta.later_at = later ? (meta.later_at ?? Date.now()) : null
      meta.due_date = later ? dueDate : null
      app.laterChanged()
    } catch (e) {
      app.fail(e)
    }
  }

  /** Clicking empty space in the page puts the cursor in the text: on the
   * line level with the click, or at the end when clicking below the text. */
  function focusFromClick(e: MouseEvent) {
    const target = e.target as HTMLElement
    if (!editor || target.closest('.ProseMirror, input, button, .meta, .thread')) return
    e.preventDefault()
    const view = editor.view
    const rect = view.dom.getBoundingClientRect()
    let pos = view.state.doc.content.size
    if (e.clientY <= rect.bottom) {
      const left = Math.min(Math.max(e.clientX, rect.left + 1), rect.right - 1)
      pos = view.posAtCoords({ left, top: Math.max(e.clientY, rect.top + 1) })?.pos ?? pos
    }
    editor.commands.setTextSelection(pos)
    view.focus()
  }

  $effect(() => {
    void load(noteId)
  })

  /** Saves the note and any reply being edited. */
  async function flushAll() {
    await flush()
    await thread?.flush()
  }

  onMount(() => {
    app.flushEditor = flushAll
    const clock = setInterval(() => (now = Date.now()), 30_000)
    return () => {
      clearInterval(clock)
      if (app.flushEditor === flushAll) app.flushEditor = async () => {}
      const closing = editor
      void flush().then(() => closing?.destroy())
    }
  })

  const statusText = $derived(
    { saved: 'Saved', unsaved: 'Editing…', saving: 'Saving…', error: "Couldn't save" }[status],
  )
</script>

<section class="editor" bind:clientWidth={width}>
  {#if editor}
    <Toolbar {editor} {tick} />
  {/if}
  <div class="columns">
    <!-- Mouse-only convenience; keyboard users reach the text with Tab / Enter. -->
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="scroll" onmousedown={focusFromClick}>
      <div class="page">
        {#if loadError}
          <p class="error">{loadError}</p>
        {/if}
        <input
          class="title"
          type="text"
          placeholder="Untitled"
          bind:this={titleInput}
          bind:value={title}
          oninput={scheduleSave}
          onkeydown={(e) => {
            if ((e.key === 'Enter' || e.key === 'ArrowDown') && editor) {
              e.preventDefault()
              // Focus synchronously (commands.focus waits a frame), so the next
              // keystroke already lands in the body.
              editor.commands.setTextSelection(0)
              editor.view.focus()
            }
          }}
        />
        {#if meta}
          <div class="meta">
            <span title={fullTime(meta.created_at)}>Created {fullTime(meta.created_at)}</span>
            <span aria-hidden="true">·</span>
            <span title={fullTime(meta.updated_at)}>Edited {sinceTime(meta.updated_at, now)}</span>
            <span class="spacer"></span>
            <button
              class="later"
              class:on={meta.later_at !== null}
              aria-pressed={meta.later_at !== null}
              title={meta.later_at !== null ? 'Remove from Later' : 'Add to Later'}
              onclick={() => setLater(meta?.later_at === null, meta?.due_date ?? null)}
            >
              <Icon name="clock" size={14} />
              Later
            </button>
            {#if meta.later_at !== null}
              <input
                class="due"
                type="date"
                aria-label="Due date"
                title="Due date (optional)"
                value={meta.due_date ?? ''}
                onchange={(e) => setLater(true, e.currentTarget.value || null)}
              />
            {/if}
          </div>
        {/if}
        <div class="body" bind:this={host}></div>
        {#if !side}
          <div class="below">
            {#key noteId}<Thread {noteId} bind:this={thread} />{/key}
          </div>
        {/if}
      </div>
    </div>
    {#if side}
      <aside class="side">
        {#key noteId}<Thread {noteId} bind:this={thread} />{/key}
      </aside>
    {/if}
  </div>
  <footer class:error={status === 'error'}>{statusText}</footer>
</section>

<style>
  .editor {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-width: 0;
  }

  .columns {
    display: flex;
    flex: 1;
    min-height: 0;
  }

  .scroll {
    flex: 1;
    min-width: 0;
    overflow-y: auto;
    cursor: text;
  }

  .body {
    min-height: 120px;
  }

  .below {
    margin-top: 40px;
    padding-top: 16px;
    border-top: 1px solid var(--border);
  }

  .side {
    flex: none;
    width: 360px;
    overflow-y: auto;
    padding: 20px 16px 120px;
    border-left: 1px solid var(--border);
    background: var(--bg-sidebar);
  }

  .page {
    max-width: 760px;
    min-height: 100%;
    margin: 0 auto;
    padding: 28px 40px 120px;
  }

  .title {
    width: 100%;
    padding: 0 0 6px;
    border: none;
    background: none;
    font-size: 28px;
    font-weight: 700;
    outline: none;
  }

  .title::placeholder {
    color: var(--text-muted);
    opacity: 0.6;
  }

  .meta {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 4px 8px;
    margin-bottom: 18px;
    font-size: 12px;
    color: var(--text-muted);
    cursor: default;
  }

  .spacer {
    flex: 1;
  }

  .later {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    padding: 2px 9px;
    border-radius: 999px;
    color: var(--text-muted);
    font-size: 12px;
  }

  .later.on {
    border-color: var(--accent);
    background: var(--bg-selected);
    color: var(--text);
  }

  .due {
    padding: 1px 6px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--bg);
    color: var(--text);
    font: inherit;
  }

  footer {
    position: absolute;
    right: 16px;
    bottom: 8px;
    font-size: 12px;
    color: var(--text-muted);
    pointer-events: none;
  }
</style>
