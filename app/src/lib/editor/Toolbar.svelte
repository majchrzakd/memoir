<script lang="ts">
  import type { ChainedCommands, Editor } from '@tiptap/core'
  import { app } from '../app.svelte'
  import { keys } from '../platform'
  import Icon from '../ui/Icon.svelte'
  import { insertAttachments } from './attachments'

  let { editor, tick }: { editor: Editor; tick: number } = $props()

  let fileInput = $state<HTMLInputElement>()

  function attach(input: HTMLInputElement) {
    const files = [...(input.files ?? [])]
    input.value = ''
    const upload = app.editorOptions().upload
    if (files.length && upload) void insertAttachments(editor, files, upload)
  }

  interface Tool {
    label: string
    title: string
    run: (c: ChainedCommands) => ChainedCommands
    /** Node or mark name (and attributes) that marks the tool as active. */
    active?: [string, Record<string, unknown>?]
    style?: string
  }

  const groups: Tool[][] = [
    [
      {
        label: 'B',
        title: `Bold (${keys('Ctrl+B')})`,
        run: (c) => c.toggleBold(),
        active: ['bold'],
        style: 'font-weight:700',
      },
      {
        label: 'I',
        title: `Italic (${keys('Ctrl+I')})`,
        run: (c) => c.toggleItalic(),
        active: ['italic'],
        style: 'font-style:italic',
      },
      {
        label: 'U',
        title: `Underline (${keys('Ctrl+U')})`,
        run: (c) => c.toggleUnderline(),
        active: ['underline'],
        style: 'text-decoration:underline',
      },
      {
        label: 'S',
        title: `Strikethrough (${keys('Ctrl+Shift+S')})`,
        run: (c) => c.toggleStrike(),
        active: ['strike'],
        style: 'text-decoration:line-through',
      },
      {
        label: '</>',
        title: `Inline code (${keys('Ctrl+E')})`,
        run: (c) => c.toggleCode(),
        active: ['code'],
        style: 'font-family:var(--mono);font-size:12px',
      },
    ],
    [
      {
        label: 'H1',
        title: `Heading 1 (${keys('Ctrl+Alt+1')})`,
        run: (c) => c.toggleHeading({ level: 1 }),
        active: ['heading', { level: 1 }],
      },
      {
        label: 'H2',
        title: `Heading 2 (${keys('Ctrl+Alt+2')})`,
        run: (c) => c.toggleHeading({ level: 2 }),
        active: ['heading', { level: 2 }],
      },
      {
        label: 'H3',
        title: `Heading 3 (${keys('Ctrl+Alt+3')})`,
        run: (c) => c.toggleHeading({ level: 3 }),
        active: ['heading', { level: 3 }],
      },
    ],
    [
      {
        label: '•',
        title: `Bulleted list (${keys('Ctrl+Shift+8')})`,
        run: (c) => c.toggleBulletList(),
        active: ['bulletList'],
      },
      {
        label: '1.',
        title: `Numbered list (${keys('Ctrl+Shift+7')})`,
        run: (c) => c.toggleOrderedList(),
        active: ['orderedList'],
      },
      {
        label: '☑',
        title: `Checklist (${keys('Ctrl+Shift+9')})`,
        run: (c) => c.toggleTaskList(),
        active: ['taskList'],
      },
    ],
    [
      {
        label: '❝',
        title: `Quote (${keys('Ctrl+Shift+B')})`,
        run: (c) => c.toggleBlockquote(),
        active: ['blockquote'],
      },
      {
        label: '{ }',
        title: `Code block (${keys('Ctrl+Alt+C')})`,
        run: (c) => c.toggleCodeBlock(),
        active: ['codeBlock'],
        style: 'font-family:var(--mono);font-size:12px',
      },
      {
        label: '▦',
        title: 'Insert table',
        run: (c) => c.insertTable({ rows: 3, cols: 3, withHeaderRow: true }),
      },
      { label: '―', title: 'Divider', run: (c) => c.setHorizontalRule() },
    ],
    [
      { label: '↶', title: `Undo (${keys('Ctrl+Z')})`, run: (c) => c.undo() },
      { label: '↷', title: `Redo (${keys('Ctrl+Y')})`, run: (c) => c.redo() },
    ],
  ]

  const toolState = $derived.by(() => {
    void tick
    return groups.map((g) =>
      g.map((tool) => ({
        active: tool.active ? editor.isActive(tool.active[0], tool.active[1] ?? {}) : false,
        enabled: tool.run(editor.can().chain()).run(),
      })),
    )
  })
</script>

<div class="toolbar" role="toolbar" aria-label="Formatting">
  {#each groups as group, g (g)}
    <div class="group">
      {#each group as tool, t (tool.title)}
        <button
          class="tool"
          class:active={toolState[g][t].active}
          title={tool.title}
          aria-label={tool.title}
          aria-pressed={tool.active ? toolState[g][t].active : undefined}
          disabled={!toolState[g][t].enabled}
          style={tool.style}
          onmousedown={(e) => e.preventDefault()}
          onclick={() => tool.run(editor.chain().focus()).run()}
        >
          {tool.label}
        </button>
      {/each}
    </div>
  {/each}
  <div class="group">
    <button
      class="tool"
      title="Link to another note (or type [[)"
      aria-label="Link to another note"
      onmousedown={(e) => e.preventDefault()}
      onclick={() => (app.linkingEditor = editor)}
    >
      <Icon name="link" />
    </button>
    <button
      class="tool"
      title="Attach a file or image (or paste / drop it into the note)"
      aria-label="Attach a file or image"
      onmousedown={(e) => e.preventDefault()}
      onclick={() => fileInput?.click()}
    >
      <Icon name="paperclip" />
    </button>
    <input
      type="file"
      multiple
      hidden
      bind:this={fileInput}
      onchange={(e) => attach(e.currentTarget)}
    />
  </div>
</div>

<style>
  .toolbar {
    display: flex;
    flex-wrap: wrap;
    gap: 2px 10px;
    padding: 6px 12px;
    border-bottom: 1px solid var(--border);
  }

  .group {
    display: flex;
    gap: 2px;
  }

  .tool {
    min-width: 30px;
    height: 28px;
    padding: 0 6px;
    border: none;
    background: none;
    color: var(--text-muted);
    font-size: 14px;
  }

  .tool:hover:not(:disabled) {
    color: var(--text);
  }

  .tool.active {
    background: var(--bg-selected);
    color: var(--text);
  }

  /* Phone: one row that scrolls sideways, with finger-sized buttons. */
  @media (max-width: 700px) {
    .toolbar {
      flex-wrap: nowrap;
      overflow-x: auto;
      padding: 4px 8px;
    }

    .group {
      flex: none;
    }

    .tool {
      min-width: 40px;
      height: 40px;
    }
  }
</style>
