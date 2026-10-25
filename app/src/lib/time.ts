// Date and time display. Dates use the system's regional format; the few
// words ("Today", "min ago") follow the UI language (English).

const MINUTE = 60_000
const HOUR = 60 * MINUTE

const startOfDay = (t: number) => new Date(t).setHours(0, 0, 0, 0)

function format(ts: number, options: Intl.DateTimeFormatOptions, locale?: string): string {
  return new Intl.DateTimeFormat(locale, options).format(ts)
}

/** Compact time for lists: "14:32" today, "24 Oct" this year, "24/10/2025" before. */
export function shortTime(ts: number, now = Date.now(), locale?: string): string {
  if (startOfDay(ts) === startOfDay(now)) return format(ts, { timeStyle: 'short' }, locale)
  if (new Date(ts).getFullYear() === new Date(now).getFullYear())
    return format(ts, { day: 'numeric', month: 'short' }, locale)
  return format(ts, { dateStyle: 'short' }, locale)
}

/** "24 Oct 2026, 22:27" */
export function fullTime(ts: number, locale?: string): string {
  return format(ts, { dateStyle: 'medium', timeStyle: 'short' }, locale)
}

/** "just now", "5 min ago", "3 h ago" within 12 hours, otherwise the full time. */
export function sinceTime(ts: number, now = Date.now(), locale?: string): string {
  const elapsed = now - ts
  if (elapsed < MINUTE) return 'just now'
  if (elapsed < HOUR) return `${Math.floor(elapsed / MINUTE)} min ago`
  if (elapsed < 12 * HOUR) return `${Math.floor(elapsed / HOUR)} h ago`
  return fullTime(ts, locale)
}

/** Local calendar date as YYYY-MM-DD. */
export function localDate(ts = Date.now()): string {
  const d = new Date(ts)
  const pad = (n: number) => String(n).padStart(2, '0')
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`
}

/** How a due date (YYYY-MM-DD) reads relative to today. */
export function dueLabel(
  date: string,
  now = Date.now(),
  locale?: string,
): { text: string; overdue: boolean; today: boolean } {
  const [y, m, d] = date.split('-').map(Number)
  const due = new Date(y, m - 1, d).getTime()
  const days = Math.round((due - startOfDay(now)) / (24 * HOUR))
  const sameYear = y === new Date(now).getFullYear()
  const text =
    days === 0
      ? 'Today'
      : days === 1
        ? 'Tomorrow'
        : days === -1
          ? 'Yesterday'
          : format(
              due,
              sameYear ? { day: 'numeric', month: 'short' } : { dateStyle: 'medium' },
              locale,
            )
  return { text, overdue: days < 0, today: days === 0 }
}
