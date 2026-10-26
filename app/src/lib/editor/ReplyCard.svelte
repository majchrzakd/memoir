<script lang="ts">
  import { Editor } from '@tiptap/core'
  import { onMount, untrack } from 'svelte'
  import { api, type Reply } from '../api'
  import { app } from '../app.svelte'
  import { fullTime, shortTime } from '../time'
  import Icon from '../ui/Icon.svelte'
  import { attachmentActions } from './attachmentActions'
  import { editorExtensions } from './extensions'
  import { replaceMarkdown } from './replaceMarkdown'

  let {
    reply,
    highlighted = false,
    ondelete,
  }: { reply: Reply; highlighted?: boolean; ondelete: () => void } = $props()

  const AUTOSAVE_MS = 300

  let host = $state<HTMLDivElement>()
  let editor: Editor | null = null
  let editing = $state(false)
  // Starts from the loaded reply; afterwards this card tracks its own saves.
  // svelte-ignore state_referenced_locally
  let updatedAt = $state(reply.updated_at)

  /** The reply as stored when last loaded or saved (see Editor.svelte). */
  // svelte-ignore state_referenced_locally
  let base = reply.body
  let dirty = false
  let timer: ReturnType<typeof setTimeout> | undefined
  let queue: Promise<void> = Promise.resolve()

  onMount(() => {
    editor = new Editor({
      element: host,
      extensions: editorExtensions(app.editorOptions()),
      content: reply.body,
      contentType: 'markdown',
      editable: false,
      editorProps: { attributes: { class: 'prose', spellcheck: 'true' } },
      onUpdate: () => {
        if (!editing) return
        dirty = true
        clearTimeout(timer)
        timer = setTimeout(() => void flush(), AUTOSAVE_MS)
      },
    })
    return () => {
      const closing = editor
      void flush().then(() => closing?.destroy())
    }
  })

  /** Saves pending edits (in order, one at a time). */
  export function flush(): Promise<void> {
    clearTimeout(timer)
    queue = queue.then(async () => {
      if (!dirty || !editor) return
      dirty = false
      const body = editor.getMarkdown()
      try {
        const merged = await api.saveReply(reply.id, body, base)
        base = body
        updatedAt = Date.now()
        if (merged !== null) showNewer(merged)
      } catch (e) {
        dirty = true
        app.fail(e)
      }
    })
    return queue
  }

  /** Shows newer stored text unless there are unsaved edits. */
  function showNewer(body: string) {
    if (dirty || !editor || body === base) return
    replaceMarkdown(editor, body)
    base = body
  }

  // The thread reloaded (sync brought changes).
  $effect(() => {
    const body = reply.body
    untrack(() => showNewer(body))
  })

  function startEditing() {
    if (editing || !editor) return
    editing = true
    editor.setEditable(true, false)
    editor.commands.focus('end')
  }

  async function stopEditing() {
    await flush()
    editing = false
    editor?.setEditable(false, false)
  }
</script>

<article class="reply" class:editing class:highlighted id="reply-{reply.id}">
  <header>
    <time datetime={new Date(reply.created_at).toISOString()} title={fullTime(reply.created_at)}>
      {shortTime(reply.created_at)}
    </time>
    {#if updatedAt - reply.created_at > 60_000}
      <span class="edited" title="Edited {fullTime(updatedAt)}">edited</span>
    {/if}
    <span class="spacer"></span>
    {#if editing}
      <button class="done" onclick={stopEditing}>Done</button>
    {:else}
      <button class="icon" title="Edit reply" aria-label="Edit reply" onclick={startEditing}>
        <Icon name="edit" size={14} />
      </button>
      <button class="icon" title="Delete reply" aria-label="Delete reply" onclick={ondelete}>
        <Icon name="trash" size={14} />
      </button>
    {/if}
  </header>
  <!-- Double-click is a mouse shortcut for the Edit button. -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div
    class="text"
    bind:this={host}
    use:attachmentActions
    ondblclick={startEditing}
    onkeydown={(e) => {
      if (editing && e.key === 'Escape') {
        e.preventDefault()
        void stopEditing()
      }
    }}
  ></div>
</article>

<style>
  .reply {
    padding: 8px 12px 10px;
    border: 1px solid transparent;
    border-radius: 8px;
    transition: background-color 0.6s;
  }

  .reply:hover {
    background: var(--bg-sidebar);
  }

  .reply.editing {
    border-color: var(--accent);
    background: var(--bg);
  }

  .reply.highlighted {
    background: var(--bg-selected);
  }

  header {
    display: flex;
    align-items: center;
    gap: 6px;
    min-height: 24px;
    font-size: 12px;
    color: var(--text-muted);
  }

  .edited {
    font-style: italic;
  }

  .spacer {
    flex: 1;
  }

  header .icon {
    padding: 3px;
    opacity: 0;
  }

  .reply:hover header .icon,
  header .icon:focus-visible {
    opacity: 1;
  }

  .done {
    padding: 1px 10px;
    font-size: 12px;
  }

  .text :global(.prose) {
    font-size: 14px;
  }
</style>
