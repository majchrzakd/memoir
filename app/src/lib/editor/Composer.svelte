<script lang="ts">
  import { Editor, Extension } from '@tiptap/core'
  import { Placeholder } from '@tiptap/extensions'
  import { onMount } from 'svelte'
  import type { Id } from '../api'
  import { app } from '../app.svelte'
  import { attachmentActions } from './attachmentActions'
  import { editorExtensions } from './extensions'

  let { noteId, onsubmit }: { noteId: Id; onsubmit: (body: string) => Promise<void> } = $props()

  let host = $state<HTMLDivElement>()
  let editor: Editor | null = null
  let empty = $state(true)
  let busy = $state(false)

  onMount(() => {
    editor = new Editor({
      element: host,
      extensions: [
        ...editorExtensions(app.editorOptions()),
        Placeholder.configure({ placeholder: 'Reply…' }),
        Extension.create({
          name: 'sendShortcut',
          addKeyboardShortcuts: () => ({
            'Mod-Enter': () => {
              void submit()
              return true
            },
          }),
        }),
      ],
      content: app.replyDrafts.get(noteId) ?? '',
      contentType: 'markdown',
      editorProps: { attributes: { class: 'prose', spellcheck: 'true' } },
      onUpdate: ({ editor }) => {
        empty = editor.isEmpty
        if (editor.isEmpty) app.replyDrafts.delete(noteId)
        else app.replyDrafts.set(noteId, editor.getMarkdown())
      },
    })
    empty = editor.isEmpty
    return () => editor?.destroy()
  })

  async function submit() {
    if (!editor || editor.isEmpty || busy) return
    busy = true
    try {
      await onsubmit(editor.getMarkdown())
      editor.commands.clearContent(true)
      app.replyDrafts.delete(noteId)
    } catch (e) {
      app.fail(e)
    } finally {
      busy = false
    }
  }
</script>

<div class="composer">
  <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
  <div
    class="input"
    bind:this={host}
    use:attachmentActions
    onclick={() => editor?.commands.focus()}
  ></div>
  <div class="actions">
    <span class="hint">Ctrl+Enter to send</span>
    <button class="primary" disabled={empty || busy} onclick={submit}>Reply</button>
  </div>
</div>

<style>
  .composer {
    margin-top: 8px;
    padding: 8px 10px;
    border: 1px solid var(--border);
    border-radius: 8px;
    background: var(--bg);
  }

  .composer:focus-within {
    border-color: var(--accent);
  }

  .input {
    min-height: 44px;
    max-height: 40vh;
    overflow-y: auto;
    cursor: text;
  }

  .input :global(.prose) {
    font-size: 14px;
  }

  .input :global(.is-editor-empty:first-child::before) {
    content: attr(data-placeholder);
    float: left;
    height: 0;
    color: var(--text-muted);
    pointer-events: none;
  }

  .actions {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: 10px;
    margin-top: 6px;
  }

  .hint {
    font-size: 11px;
    color: var(--text-muted);
  }

  button.primary {
    padding: 3px 14px;
    font-size: 13px;
  }
</style>
