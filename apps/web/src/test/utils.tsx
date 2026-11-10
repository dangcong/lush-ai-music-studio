import type { ReactElement } from 'react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { render } from '@testing-library/react'
import { MemoryRouter } from 'react-router-dom'
import { vi } from 'vitest'
import type { Genre, Job, Model, Preset, Track, User } from '../api/types'
import { AuthContext } from '../auth/AuthContext'
import { ToastProvider } from '../components/Toast'
import { PlayerProvider } from '../player/PlayerContext'

export function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'Content-Type': 'application/json' },
  })
}

type Handler = (init: RequestInit | undefined, url: URL) => Response | Promise<Response>

/**
 * Stubs global fetch with a route table keyed by "METHOD /path" (path without /api/v1
 * and without the query string). Unknown routes answer 404 in the API's error format.
 */
export function mockFetch(routes: Record<string, Handler | unknown>) {
  const fn = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = new URL(String(input), 'http://localhost')
    const key = `${init?.method ?? 'GET'} ${url.pathname.replace('/api/v1', '')}`
    const route = routes[key]
    if (route === undefined) {
      return jsonResponse({ error: { code: 'not_found', message: `No mock for ${key}` } }, 404)
    }
    return typeof route === 'function' ? (route as Handler)(init, url) : jsonResponse(route)
  })
  vi.stubGlobal('fetch', fn)
  return fn
}

export function renderWithProviders(
  ui: ReactElement,
  { user = makeUser(), route = '/' }: { user?: User | null; route?: string } = {},
) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  })
  return render(
    <QueryClientProvider client={queryClient}>
      <MemoryRouter initialEntries={[route]}>
        <AuthContext.Provider value={{ user, login: async () => {}, logout: async () => {} }}>
          <ToastProvider>
            <PlayerProvider>{ui}</PlayerProvider>
          </ToastProvider>
        </AuthContext.Provider>
      </MemoryRouter>
    </QueryClientProvider>,
  )
}

export function makeUser(overrides: Partial<User> = {}): User {
  return {
    id: 'user-1',
    username: 'linh',
    display_name: 'Linh',
    role: 'member',
    is_active: true,
    created_at: '2026-01-01T00:00:00.000Z',
    ...overrides,
  }
}

export function makeGenre(overrides: Partial<Genre> = {}): Genre {
  return {
    id: 'genre-1',
    parent_id: null,
    name: 'Genre',
    slug: 'genre',
    description: null,
    prompt_template: null,
    instruments: [],
    moods: [],
    bpm_min: null,
    bpm_max: null,
    default_duration: null,
    is_active: true,
    sort_order: 0,
    ...overrides,
  }
}

export function makeModel(overrides: Partial<Model> = {}): Model {
  return {
    id: 'model-1',
    engine: 'mock',
    key: 'mock-v1',
    name: 'Mock Engine v1',
    version: '1',
    description: null,
    capabilities: ['generate'],
    status: 'active',
    installed_nodes: [{ node_id: 'node-1', node_name: 'render-node-01', online: true }],
    ...overrides,
  }
}

export function makePreset(overrides: Partial<Preset> = {}): Preset {
  return {
    id: 'preset-1',
    genre_id: 'genre-1',
    name: 'Preset',
    description: null,
    kind: 'genre',
    prompt_template: null,
    instruments: [],
    moods: [],
    bpm: null,
    duration: null,
    model_id: null,
    lora_id: null,
    lora_scale: null,
    params: {},
    version: 1,
    is_active: true,
    ...overrides,
  }
}

export function makeJob(overrides: Partial<Job> = {}): Job {
  return {
    id: 'job-1',
    type: 'generate',
    status: 'queued',
    priority: 0,
    progress: 0,
    stage: null,
    message: null,
    error: null,
    attempt: 0,
    max_attempts: 3,
    cancel_requested: false,
    owner_id: 'user-1',
    owner_name: 'Linh',
    node_id: null,
    node_name: null,
    track_id: null,
    title: 'Rainy night',
    request: { prompt: 'Warm jazz piano' },
    result: null,
    created_at: '2026-01-01T00:00:00.000Z',
    started_at: null,
    finished_at: null,
    ...overrides,
  }
}

export function makeTrack(overrides: Partial<Track> = {}): Track {
  return {
    id: 'track-1',
    title: 'Rainy night',
    owner_id: 'user-1',
    owner_name: 'Linh',
    project_id: null,
    job_id: 'job-1',
    parent_track_id: null,
    version: 1,
    prompt: 'Warm jazz piano',
    prompt_final: 'Warm jazz piano, instrumental, no vocals',
    genre_id: 'genre-1',
    genre_name: 'Jazz',
    subgenre_id: null,
    preset_id: null,
    model_id: 'model-1',
    model_name: 'Mock Engine v1',
    lora_id: null,
    lora_scale: null,
    seed: 42,
    bpm: null,
    key: null,
    time_signature: null,
    duration: 120,
    favorite: false,
    rating: 0,
    params: {},
    artifacts: [
      {
        id: 'art-master',
        kind: 'audio_master',
        filename: 'rainy-night.wav',
        content_type: 'audio/wav',
        size_bytes: 1000,
        sha256: 'aa',
      },
      {
        id: 'art-preview',
        kind: 'audio_preview',
        filename: 'rainy-night.mp3',
        content_type: 'audio/mpeg',
        size_bytes: 100,
        sha256: 'bb',
      },
    ],
    created_at: '2026-01-01T00:00:00.000Z',
    ...overrides,
  }
}
