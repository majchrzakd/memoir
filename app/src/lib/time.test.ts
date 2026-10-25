import { describe, expect, it } from 'vitest'
import { dueLabel, localDate, shortTime, sinceTime } from './time'

const L = 'en-GB'
const now = new Date(2026, 9, 25, 15, 0).getTime() // 25 Oct 2026, 15:00 local

describe('shortTime', () => {
  it('shows the time today, the day this year, the date before', () => {
    expect(shortTime(new Date(2026, 9, 25, 9, 5).getTime(), now, L)).toBe('09:05')
    expect(shortTime(new Date(2026, 9, 24, 23, 0).getTime(), now, L)).toBe('24 Oct')
    expect(shortTime(new Date(2025, 0, 2).getTime(), now, L)).toBe('02/01/2025')
  })
})

describe('sinceTime', () => {
  it('is relative for recent edits', () => {
    expect(sinceTime(now - 20_000, now)).toBe('just now')
    expect(sinceTime(now - 5 * 60_000, now)).toBe('5 min ago')
    expect(sinceTime(now - 3 * 3600_000, now)).toBe('3 h ago')
    // (Not "now - 30 h": in Europe that crosses the end of daylight saving time.)
    expect(sinceTime(new Date(2026, 9, 24, 9, 0).getTime(), now, L)).toBe('24 Oct 2026, 09:00')
  })
})

describe('dueLabel', () => {
  it('names nearby days and flags overdue dates', () => {
    expect(dueLabel('2026-10-25', now, L)).toEqual({ text: 'Today', overdue: false, today: true })
    expect(dueLabel('2026-10-26', now, L).text).toBe('Tomorrow')
    expect(dueLabel('2026-10-24', now, L)).toEqual({
      text: 'Yesterday',
      overdue: true,
      today: false,
    })
    expect(dueLabel('2026-11-03', now, L)).toEqual({ text: '3 Nov', overdue: false, today: false })
    expect(dueLabel('2027-01-15', now, L).text).toBe('15 Jan 2027')
  })
})

it('formats local dates', () => {
  expect(localDate(now)).toBe('2026-10-25')
})
