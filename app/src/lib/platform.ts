import { MediaQuery } from 'svelte/reactivity'

/**
 * An Apple device (Mac, iPhone, iPad): shortcuts use ⌘ there (the editor's
 * `Mod-` keys do too).
 */
export const isApple =
  typeof navigator !== 'undefined' && /^(Mac|iPhone|iPad)/.test(navigator.platform)

/**
 * iPhone or iPad: no self-updates (the App Store or TestFlight updates the
 * app), and no folders to export into or to open. An iPad's web view says
 * it's a Mac ("MacIntel"); unlike a Mac, it has a touch screen.
 */
export const isIOS =
  typeof navigator !== 'undefined' &&
  (/^(iPhone|iPad)/.test(navigator.platform) ||
    (/^Mac/.test(navigator.platform) && navigator.maxTouchPoints > 1))

/** A touch screen is the main pointer: no keyboard hints, bigger targets. */
export const touch = new MediaQuery('(pointer: coarse)')

/** A phone-sized window: one pane at a time (the list, or what's open). */
export const phone = new MediaQuery('(max-width: 700px)')

const MODIFIERS = ['Ctrl', 'Alt', 'Shift'] as const
const MAC_SYMBOLS: Record<string, string> = { Ctrl: '⌘', Alt: '⌥', Shift: '⇧', Enter: '↩' }

/**
 * A shortcut written the Windows way ("Ctrl+Shift+N") as this platform
 * shows it: unchanged on Windows, "⇧⌘N" on Apple devices.
 */
export function keys(shortcut: string, mac = isApple): string {
  if (!mac) return shortcut
  // Redo is ⇧⌘Z on a Mac (the editor accepts both).
  if (shortcut === 'Ctrl+Y') return '⇧⌘Z'
  const parts = shortcut.split('+')
  // Mac order: ⌥ ⇧ ⌘, then the key.
  const mods = (['Alt', 'Shift', 'Ctrl'] as const).filter((m) => parts.includes(m))
  const key = parts.filter((p) => !(MODIFIERS as readonly string[]).includes(p))
  return [...mods, ...key].map((p) => MAC_SYMBOLS[p] ?? p).join('')
}
