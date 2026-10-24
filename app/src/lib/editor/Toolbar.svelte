<script lang="ts">
  import type { ChainedCommands, Editor } from '@tiptap/core'

  let { editor, tick }: { editor: Editor; tick: number } = $props()

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
        title: 'Bold (Ctrl+B)',
        run: (c) => c.toggleBold(),
        active: ['bold'],
        style: 'font-weight:700',
      },
      {
        label: 'I',
        title: 'Italic (Ctrl+I)',
        run: (c) => c.toggleItalic(),
        active: ['italic'],
        style: 'font-style:italic',
      },
      {
        label: 'U',
        title: 'Underline (Ctrl+U)',
        run: (c) => c.toggleUnderline(),
        active: ['underline'],
        style: 'text-decoration:underline',
      },
      {
        label: 'S',
        title: 'Strikethrough (Ctrl+Shift+S)',
        run: (c) => c.toggleStrike(),
        active: ['strike'],
        style: 'text-decoration:line-through',
      },
      {
        label: '</>',
        title: 'Inline code (Ctrl+E)',
        run: (c) => c.toggleCode(),
        active: ['code'],
        style: 'font-family:var(--mono);font-size:12px',
      },
    ],
    [
      {
        label: 'H1',
        title: 'Heading 1 (Ctrl+Alt+1)',
        run: (c) => c.toggleHeading({ level: 1 }),
        active: ['heading', { level: 1 }],
      },
      {
        label: 'H2',
        title: 'Heading 2 (Ctrl+Alt+2)',
        run: (c) => c.toggleHeading({ level: 2 }),
        active: ['heading', { level: 2 }],
      },
      {
        label: 'H3',
        title: 'Heading 3 (Ctrl+Alt+3)',
        run: (c) => c.toggleHeading({ level: 3 }),
        active: ['heading', { level: 3 }],
      },
    ],
    [
      {
        label: '•',
        title: 'Bulleted list (Ctrl+Shift+8)',
        run: (c) => c.toggleBulletList(),
        active: ['bulletList'],
      },
      {
        label: '1.',
        title: 'Numbered list (Ctrl+Shift+7)',
        run: (c) => c.toggleOrderedList(),
        active: ['orderedList'],
      },
      {
        label: '☑',
        title: 'Checklist (Ctrl+Shift+9)',
        run: (c) => c.toggleTaskList(),
        active: ['taskList'],
      },
    ],
    [
      {
        label: '❝',
        title: 'Quote (Ctrl+Shift+B)',
        run: (c) => c.toggleBlockquote(),
        active: ['blockquote'],
      },
      {
        label: '{ }',
        title: 'Code block (Ctrl+Alt+C)',
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
      { label: '↶', title: 'Undo (Ctrl+Z)', run: (c) => c.undo() },
      { label: '↷', title: 'Redo (Ctrl+Y)', run: (c) => c.redo() },
    ],
  ]

  const state = $derived.by(() => {
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
          class:active={state[g][t].active}
          title={tool.title}
          aria-label={tool.title}
          aria-pressed={tool.active ? state[g][t].active : undefined}
          disabled={!state[g][t].enabled}
          style={tool.style}
          onmousedown={(e) => e.preventDefault()}
          onclick={() => tool.run(editor.chain().focus()).run()}
        >
          {tool.label}
        </button>
      {/each}
    </div>
  {/each}
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
</style>
