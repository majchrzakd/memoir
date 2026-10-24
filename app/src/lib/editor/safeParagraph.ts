import Paragraph from '@tiptap/extension-paragraph'

/**
 * Paragraph whose Markdown output can't be re-parsed as a different block.
 *
 * TipTap escapes inline syntax (`*`, `_`, `[`…) but not block syntax at the
 * start of a line, so a typed paragraph "# idea" or "1. thing" or "---" would
 * reload as a heading, a list or a rule. Every line is checked because a hard
 * break starts a new line that can still interrupt the paragraph.
 */
export const SafeParagraph = Paragraph.extend({
  renderMarkdown(node, helpers, ctx) {
    const md: string = Paragraph.config.renderMarkdown?.call(this, node, helpers, ctx) ?? ''
    return md.split('\n').map(escapeBlockStart).join('\n')
  },
})

export function escapeBlockStart(line: string): string {
  // Indentation would be stripped, or turn the line into a code block.
  const indent = /^ +/.exec(line)
  if (indent) return '&#32;' + line.slice(1)

  return (
    line
      // ATX headings, list bullets, setext underlines, table rows.
      .replace(/^([#+=|-])/, '\\$1')
      // Ordered list markers: "1." / "1)".
      .replace(/^(\d{1,9})([.)])/, '$1\\$2')
  )
}
