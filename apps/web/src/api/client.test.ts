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

describe('lora build endpoints', () => {
  it('createLora posts to the catalog loras endpoint with the id encoded', async () => {
    const fetchMock = stubFetch(jsonResponse({ job_id: 'j', snapshot_id: 's', lora_key: 'k' }, 201))
    const body = { name: 'A' } as never
    await api.catalogs.createLora('a/b', body)
    const [url, init] = fetchMock.mock.calls[0]!
    expect(url).toBe('/api/v1/catalogs/a%2Fb/loras')
    expect(init?.method).toBe('POST')
  })

  it('previewLora and snapshot use their endpoints', async () => {
    let fetchMock = stubFetch(jsonResponse({}))
    await api.catalogs.previewLora('c1', {} as never)
    expect(fetchMock.mock.calls[0]![0]).toBe('/api/v1/catalogs/c1/lora-preview')
    fetchMock = stubFetch(jsonResponse({}))
    await api.datasetSnapshots.get('s1')
    expect(fetchMock.mock.calls[0]![0]).toBe('/api/v1/dataset-snapshots/s1')
  })
})

describe('review endpoints', () => {
  it('publishCheckpoint POSTs to /loras/{id}/publish', async () => {
    const fetchMock = stubFetch(jsonResponse({ job_id: 'j', style_preset_id: 's' }, 202))
    const out = await api.loras.publish('l/1', { checkpoint_id: 'c', style_name: 'S', lora_scale: 0.7 })
    expect(out.job_id).toBe('j')
    const [url, init] = fetchMock.mock.calls[0]!
    expect(url).toBe('/api/v1/loras/l%2F1/publish')
    expect(init?.method).toBe('POST')
    expect(JSON.parse(init?.body as string)).toEqual({ checkpoint_id: 'c', style_name: 'S', lora_scale: 0.7 })
  })
  it('note, cleanup, remove and try use their routes', async () => {
    const fetchMock = vi.fn<typeof fetch>(async () => jsonResponse({}, 200))
    vi.stubGlobal('fetch', fetchMock)
    await api.loraCheckpoints.saveNote('c1', 'hay')
    await api.loras.cleanup('l1')
    await api.loras.tryCheckpoint('l1', { checkpoint_id: 'c', prompt: 'p', lora_scale: 0.7 })
    await api.loras.remove('l1')
    const calls = fetchMock.mock.calls.map(([u, i]) => `${i?.method} ${u}`)
    expect(calls).toEqual(['PATCH /api/v1/lora-checkpoints/c1', 'POST /api/v1/loras/l1/cleanup', 'POST /api/v1/loras/l1/try', 'DELETE /api/v1/loras/l1'])
  })
})
