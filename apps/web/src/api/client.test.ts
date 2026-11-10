import { afterEach, describe, expect, it, vi } from 'vitest'
import { api } from './api'
import { ApiError, artifactUrl, request, setUnauthorizedHandler } from './client'
import { jsonResponse, makeUser } from '../test/utils'

function stubFetch(response: Response | Error) {
  const fn = vi.fn<typeof fetch>(async () => {
    if (response instanceof Error) throw response
    return response
  })
  vi.stubGlobal('fetch', fn)
  return fn
}

afterEach(() => setUnauthorizedHandler(null))

describe('request', () => {
  it('sends JSON with the session cookie to /api/v1 and parses the response', async () => {
    const fetchMock = stubFetch(jsonResponse(makeUser(), 200))
    const user = await api.auth.login('linh', 'secret')

    expect(user.username).toBe('linh')
    const [url, init] = fetchMock.mock.calls[0]!
    expect(url).toBe('/api/v1/auth/login')
    expect(init?.method).toBe('POST')
    expect(init?.credentials).toBe('include')
    expect(JSON.parse(init?.body as string)).toEqual({ username: 'linh', password: 'secret' })
  })

  it('builds query strings, dropping empty filters and false flags', async () => {
    const fetchMock = stubFetch(jsonResponse([]))
    await api.tracks.list({ q: 'rain', genre_id: '', favorite: true, mine: false, limit: 50 })
    expect(fetchMock.mock.calls[0]![0]).toBe('/api/v1/tracks?q=rain&favorite=true&limit=50')
  })

  it('returns undefined for 204 responses', async () => {
    stubFetch(new Response(null, { status: 204 }))
    await expect(api.auth.logout()).resolves.toBeUndefined()
  })

  it('turns the {error:{code,message}} envelope into an ApiError', async () => {
    stubFetch(jsonResponse({ error: { code: 'validation', message: 'Model đang bị tắt' } }, 422))
    const error = await request('POST', '/generations', { body: {} }).catch((e: unknown) => e)

    expect(error).toBeInstanceOf(ApiError)
    expect(error).toMatchObject({ status: 422, code: 'validation', message: 'Model đang bị tắt' })
  })

  it('falls back to a readable message when the error body is not JSON', async () => {
    stubFetch(new Response('<html>Bad Gateway</html>', { status: 502 }))
    const error = await request('GET', '/tracks').catch((e: unknown) => e)
    expect(error).toMatchObject({ status: 502, code: 'http_error', message: 'Lỗi máy chủ (502).' })
  })

  it('reports network failures as ApiError', async () => {
    stubFetch(new TypeError('Failed to fetch'))
    const error = await request('GET', '/tracks').catch((e: unknown) => e)
    expect(error).toMatchObject({ status: 0, code: 'network_error' })
  })

  it('calls the unauthorized handler on any 401', async () => {
    const handler = vi.fn()
    setUnauthorizedHandler(handler)
    stubFetch(jsonResponse({ error: { code: 'unauthorized', message: 'Chưa đăng nhập' } }, 401))

    await expect(api.tracks.list()).rejects.toMatchObject({ status: 401, message: 'Chưa đăng nhập' })
    expect(handler).toHaveBeenCalledTimes(1)
  })

  it('does not treat a failed login as an expired session', async () => {
    const handler = vi.fn()
    setUnauthorizedHandler(handler)
    stubFetch(jsonResponse({ error: { code: 'unauthorized', message: 'Sai mật khẩu' } }, 401))

    await expect(api.auth.login('linh', 'wrong')).rejects.toMatchObject({ message: 'Sai mật khẩu' })
    expect(handler).not.toHaveBeenCalled()
  })
})

describe('artifactUrl', () => {
  it('points at the artifact content endpoint', () => {
    expect(artifactUrl('abc')).toBe('/api/v1/artifacts/abc/content')
    expect(artifactUrl('abc', true)).toBe('/api/v1/artifacts/abc/content?download=true')
  })
})
