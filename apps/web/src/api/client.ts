export const API_BASE = '/api/v1'

export class ApiError extends Error {
  readonly status: number
  readonly code: string

  constructor(status: number, code: string, message: string) {
    super(message)
    this.name = 'ApiError'
    this.status = status
    this.code = code
  }
}

type QueryValue = string | number | boolean | null | undefined

export interface RequestOptions {
  body?: unknown
  /** Multipart body (file upload). The browser sets the Content-Type with its boundary. */
  form?: FormData
  query?: Record<string, QueryValue>
  /** Do not trigger the global "session expired" handler on 401 (used by login). */
  skipUnauthorizedHandler?: boolean
}

let unauthorizedHandler: (() => void) | null = null

/** Registers the callback run whenever any API call answers 401. */
export function setUnauthorizedHandler(handler: (() => void) | null): void {
  unauthorizedHandler = handler
}

const FALLBACK_MESSAGES: Record<number, string> = {
  400: 'Yêu cầu không hợp lệ.',
  401: 'Phiên đăng nhập đã hết hạn. Vui lòng đăng nhập lại.',
  403: 'Bạn không có quyền thực hiện thao tác này.',
  404: 'Không tìm thấy dữ liệu.',
  409: 'Dữ liệu bị xung đột.',
  413: 'File quá lớn.',
  422: 'Dữ liệu không hợp lệ.',
}

export function buildUrl(path: string, query?: Record<string, QueryValue>): string {
  const params = new URLSearchParams()
  for (const [key, value] of Object.entries(query ?? {})) {
    // Omit empty filters; boolean flags are only sent when true.
    if (value === undefined || value === null || value === '' || value === false) continue
    params.set(key, String(value))
  }
  const qs = params.toString()
  return API_BASE + path + (qs ? '?' + qs : '')
}

async function parseError(response: Response): Promise<ApiError> {
  let code = 'http_error'
  let message = FALLBACK_MESSAGES[response.status] ?? `Lỗi máy chủ (${response.status}).`
  try {
    const data: unknown = await response.json()
    const err = (data as { error?: { code?: unknown; message?: unknown } } | null)?.error
    if (err) {
      if (typeof err.code === 'string') code = err.code
      if (typeof err.message === 'string' && err.message) message = err.message
    }
  } catch {
    // Body is not JSON; keep the fallback message.
  }
  return new ApiError(response.status, code, message)
}

export async function request<T>(
  method: string,
  path: string,
  options: RequestOptions = {},
): Promise<T> {
  const headers: Record<string, string> = { Accept: 'application/json' }
  const init: RequestInit = { method, credentials: 'include', headers }
  if (options.form !== undefined) {
    init.body = options.form
  } else if (options.body !== undefined) {
    headers['Content-Type'] = 'application/json'
    init.body = JSON.stringify(options.body)
  }

  let response: Response
  try {
    response = await fetch(buildUrl(path, options.query), init)
  } catch {
    throw new ApiError(0, 'network_error', 'Không kết nối được tới máy chủ.')
  }

  if (!response.ok) {
    const error = await parseError(response)
    if (response.status === 401 && !options.skipUnauthorizedHandler) unauthorizedHandler?.()
    throw error
  }
  if (response.status === 204) return undefined as T
  const text = await response.text()
  return (text ? JSON.parse(text) : undefined) as T
}

export function errorMessage(error: unknown): string {
  if (error instanceof Error && error.message) return error.message
  return 'Đã xảy ra lỗi không xác định.'
}

/** URL of an artifact's binary content; usable directly in <audio src> / <a href>. */
export function artifactUrl(artifactId: string, download = false): string {
  return buildUrl(`/artifacts/${artifactId}/content`, { download })
}

/** ZIP of every file the track has. */
export function trackArchiveUrl(trackId: string): string {
  return buildUrl(`/tracks/${trackId}/archive`)
}
