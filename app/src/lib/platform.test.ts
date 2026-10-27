import { describe, expect, it } from 'vitest'
import { keys } from './platform'

describe('keys', () => {
  it('leaves shortcuts unchanged on Windows', () => {
    expect(keys('Ctrl+Shift+N', false)).toBe('Ctrl+Shift+N')
    expect(keys('Ctrl+Y', false)).toBe('Ctrl+Y')
  })

  it('uses Mac symbols in Mac order', () => {
    expect(keys('Ctrl+N', true)).toBe('⌘N')
    expect(keys('Ctrl+Shift+N', true)).toBe('⇧⌘N')
    expect(keys('Ctrl+Alt+1', true)).toBe('⌥⌘1')
    expect(keys('Ctrl+Enter', true)).toBe('⌘↩')
  })

  it('shows the Mac redo shortcut', () => {
    expect(keys('Ctrl+Y', true)).toBe('⇧⌘Z')
  })
})
