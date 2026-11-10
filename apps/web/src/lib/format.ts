/** 125 -> "2:05" */
export function formatDuration(seconds: number | null | undefined): string {
  if (seconds === null || seconds === undefined || !Number.isFinite(seconds)) return '--:--'
  const total = Math.max(0, Math.round(seconds))
  const m = Math.floor(total / 60)
  const s = total % 60
  return `${m}:${String(s).padStart(2, '0')}`
}

const dateTimeFormat = new Intl.DateTimeFormat('vi-VN', {
  day: '2-digit',
  month: '2-digit',
  year: 'numeric',
  hour: '2-digit',
  minute: '2-digit',
})

const dateFormat = new Intl.DateTimeFormat('vi-VN', {
  day: '2-digit',
  month: '2-digit',
  year: 'numeric',
})

function parse(iso: string | null | undefined): Date | null {
  if (!iso) return null
  const d = new Date(iso)
  return Number.isNaN(d.getTime()) ? null : d
}

export function formatDateTime(iso: string | null | undefined): string {
  const d = parse(iso)
  return d ? dateTimeFormat.format(d) : '—'
}

export function formatDate(iso: string | null | undefined): string {
  const d = parse(iso)
  return d ? dateFormat.format(d) : '—'
}

/** "vừa xong", "5 phút trước", ... falls back to a date after a week. */
export function formatRelative(iso: string | null | undefined, now: Date = new Date()): string {
  const d = parse(iso)
  if (!d) return '—'
  const seconds = Math.round((now.getTime() - d.getTime()) / 1000)
  if (seconds < 45) return 'vừa xong'
  const minutes = Math.round(seconds / 60)
  if (minutes < 60) return `${minutes} phút trước`
  const hours = Math.round(minutes / 60)
  if (hours < 24) return `${hours} giờ trước`
  const days = Math.round(hours / 24)
  if (days < 7) return `${days} ngày trước`
  return dateFormat.format(d)
}

export function formatBytes(bytes: number | null | undefined): string {
  if (bytes === null || bytes === undefined || !Number.isFinite(bytes)) return '—'
  if (bytes < 1024) return `${bytes} B`
  const units = ['KB', 'MB', 'GB']
  let value = bytes / 1024
  let unit = 0
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024
    unit++
  }
  return `${value.toFixed(value >= 10 ? 0 : 1)} ${units[unit]}`
}

/** Elapsed time between two ISO timestamps as "1 phút 20 giây". */
export function formatElapsed(
  startIso: string | null | undefined,
  endIso: string | null | undefined,
): string {
  const start = parse(startIso)
  const end = parse(endIso)
  if (!start || !end) return '—'
  const total = Math.max(0, Math.round((end.getTime() - start.getTime()) / 1000))
  if (total < 60) return `${total} giây`
  return `${Math.floor(total / 60)} phút ${total % 60} giây`
}

/** Converts the API's 0..1 progress to a 0..100 integer. */
export function progressPercent(progress: number | null | undefined): number {
  if (progress === null || progress === undefined || !Number.isFinite(progress)) return 0
  return Math.round(Math.min(1, Math.max(0, progress)) * 100)
}
