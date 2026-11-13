import { request } from './client'
import type {
  AudioInput,
  AuditLog,
  GenerationRequest,
  Genre,
  GenreInput,
  Job,
  JobListParams,
  Lora,
  Model,
  ModelUpdateInput,
  NodeCreated,
  NodeUpdateInput,
  Preset,
  PresetInput,
  QueueSummary,
  RenderNode,
  Track,
  TrackListParams,
  TrackUpdateInput,
  User,
  UserCreateInput,
  UserUpdateInput,
} from './types'

export const api = {
  auth: {
    login: (username: string, password: string) =>
      request<User>('POST', '/auth/login', {
        body: { username, password },
        skipUnauthorizedHandler: true,
      }),
    logout: () => request<void>('POST', '/auth/logout'),
    me: () => request<User>('GET', '/auth/me'),
    changePassword: (current_password: string, new_password: string) =>
      // A wrong current password must not be mistaken for an expired session.
      request<void>('POST', '/auth/password', {
        body: { current_password, new_password },
        skipUnauthorizedHandler: true,
      }),
  },
  users: {
    list: () => request<User[]>('GET', '/users'),
    create: (input: UserCreateInput) => request<User>('POST', '/users', { body: input }),
    update: (id: string, input: UserUpdateInput) =>
      request<User>('PATCH', `/users/${id}`, { body: input }),
  },
  genres: {
    list: (includeInactive = false) =>
      request<Genre[]>('GET', '/genres', { query: { include_inactive: includeInactive } }),
    create: (input: GenreInput) => request<Genre>('POST', '/genres', { body: input }),
    update: (id: string, input: GenreInput) =>
      request<Genre>('PATCH', `/genres/${id}`, { body: input }),
  },
  presets: {
    list: (params: { genre_id?: string; include_inactive?: boolean } = {}) =>
      request<Preset[]>('GET', '/presets', { query: params }),
    create: (input: PresetInput) => request<Preset>('POST', '/presets', { body: input }),
    update: (id: string, input: PresetInput) =>
      request<Preset>('PATCH', `/presets/${id}`, { body: input }),
  },
  models: {
    list: () => request<Model[]>('GET', '/models'),
    update: (id: string, input: ModelUpdateInput) =>
      request<Model>('PATCH', `/models/${id}`, { body: input }),
  },
  loras: {
    list: () => request<Lora[]>('GET', '/loras'),
  },
  generations: {
    create: (body: GenerationRequest) => request<Job>('POST', '/generations', { body }),
  },
  audioInputs: {
    upload: (file: File) => {
      const form = new FormData()
      form.append('file', file)
      return request<AudioInput>('POST', '/audio-inputs', { form })
    },
    get: (id: string) => request<AudioInput>('GET', `/audio-inputs/${id}`),
  },
  jobs: {
    list: (params: JobListParams = {}) => request<Job[]>('GET', '/jobs', { query: { ...params } }),
    get: (id: string) => request<Job>('GET', `/jobs/${id}`),
    cancel: (id: string) => request<unknown>('POST', `/jobs/${id}/cancel`),
    retry: (id: string) => request<Job>('POST', `/jobs/${id}/retry`),
  },
  queue: {
    summary: () => request<QueueSummary>('GET', '/queue/summary'),
  },
  tracks: {
    list: (params: TrackListParams = {}) =>
      request<Track[]>('GET', '/tracks', { query: { ...params } }),
    get: (id: string) => request<Track>('GET', `/tracks/${id}`),
    update: (id: string, input: TrackUpdateInput) =>
      request<Track>('PATCH', `/tracks/${id}`, { body: input }),
    remove: (id: string) => request<void>('DELETE', `/tracks/${id}`),
  },
  nodes: {
    list: () => request<RenderNode[]>('GET', '/nodes'),
    create: (name: string) => request<NodeCreated>('POST', '/nodes', { body: { name } }),
    update: (id: string, input: NodeUpdateInput) =>
      request<RenderNode>('PATCH', `/nodes/${id}`, { body: input }),
    rotateToken: (id: string) => request<{ token: string }>('POST', `/nodes/${id}/rotate-token`),
  },
  auditLogs: {
    list: (limit?: number) => request<AuditLog[]>('GET', '/audit-logs', { query: { limit } }),
  },
}
