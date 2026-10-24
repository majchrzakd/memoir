<script lang="ts">
  import { Editor } from '@tiptap/core'
  import { onMount } from 'svelte'
  import { api, type Id } from '../api'
  import { app } from '../app.svelte'
  import { editorExtensions } from './extensions'
  import Toolbar from './Toolbar.svelte'
  import './prose.css'

  let { noteId }: { noteId: Id } = $props()

  const AUTOSAVE_MS = 300

  let host = $state<HTMLDivElement>()
  let titleInput = $state<HTMLInputElement>()
  let editor = $state.raw<Editor | null>(null)
  let title = $state('')
  let status = $state<'saved' | 'unsaved' | 'saving' | 'error'>('saved')
  /** Bumped on every editor transaction so the toolbar re-reads its state. */
  let tick = $state(0)
  let loadError = $state<string | null>(null)

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
        app.titleSaved(id, savedTitle)
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

  $effect(() => {
    void load(noteId)
  })

  onMount(() => {
    app.flushEditor = flush
    return () => {
      if (app.flushEditor === flush) app.flushEditor = async () => {}
      const closing = editor
      void flush().then(() => closing?.destroy())
    }
  })

  const statusText = $derived(
    { saved: 'Saved', unsaved: 'Editing…', saving: 'Saving…', error: "Couldn't save" }[status],
  )
</script>

<section class="editor">
  {#if editor}
    <Toolbar {editor} {tick} />
  {/if}
  <div class="scroll">
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
      <div class="body" bind:this={host}></div>
    </div>
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

  .scroll {
    flex: 1;
    overflow-y: auto;
  }

  .page {
    max-width: 760px;
    margin: 0 auto;
    padding: 28px 40px 120px;
  }

  .title {
    width: 100%;
    padding: 0 0 12px;
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

  .body {
    min-height: 60vh;
    cursor: text;
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
