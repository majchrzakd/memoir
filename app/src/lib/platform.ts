/** Running on macOS: shortcuts use ⌘ there (the editor's `Mod-` keys do too). */
export const isMac = typeof navigator !== 'undefined' && /^Mac/.test(navigator.platform)

const MODIFIERS = ['Ctrl', 'Alt', 'Shift'] as const
const MAC_SYMBOLS: Record<string, string> = { Ctrl: '⌘', Alt: '⌥', Shift: '⇧', Enter: '↩' }

/**
 * A shortcut written the Windows way ("Ctrl+Shift+N") as this platform
 * shows it: unchanged on Windows, "⇧⌘N" on macOS.
 */
export function keys(shortcut: string, mac = isMac): string {
  if (!mac) return shortcut
  // Redo is ⇧⌘Z on a Mac (the editor accepts both).
  if (shortcut === 'Ctrl+Y') return '⇧⌘Z'
  const parts = shortcut.split('+')
  // Mac order: ⌥ ⇧ ⌘, then the key.
  const mods = (['Alt', 'Shift', 'Ctrl'] as const).filter((m) => parts.includes(m))
  const key = parts.filter((p) => !(MODIFIERS as readonly string[]).includes(p))
  return [...mods, ...key].map((p) => MAC_SYMBOLS[p] ?? p).join('')
}
