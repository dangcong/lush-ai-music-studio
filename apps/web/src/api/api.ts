import { request } from './client'
import type {
  AudioInput,
  BuildPlan,
  CreateLoraResult,
  DatasetSnapshot,
  LoraBuildRequest,
  AuditLog,
  CaptionInput,
  CaptionJobInput,
  CaptionJobResult,
  CaptionView,
  Catalog,
  CatalogCreateInput,
  CatalogTrack,
  CatalogTrackUpdateInput,
  CatalogUpdateInput,
  Dataset,
  GenerationRequest,
  Genre,
  GenreInput,
  Job,
  JobListParams,
  Lora,
  LoraUpdateInput,
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
  TrainingRun,
  TrainingRunDetail,
  TrainingRunInput,
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
    update: (id: string, input: LoraUpdateInput) =>
      request<Lora>('PATCH', `/loras/${id}`, { body: input }),
  },
  datasets: {
    list: () => request<Dataset[]>('GET', '/datasets'),
  },
  trainingRuns: {
    list: () => request<TrainingRun[]>('GET', '/training-runs'),
    get: (id: string) => request<TrainingRunDetail>('GET', `/training-runs/${id}`),
    create: (body: TrainingRunInput) => request<TrainingRun>('POST', '/training-runs', { body }),
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
  catalogs: {
    list: () => request<Catalog[]>('GET', '/catalogs'),
    create: (input: CatalogCreateInput) => request<Catalog>('POST', '/catalogs', { body: input }),
    get: (id: string) => request<Catalog>('GET', `/catalogs/${id}`),
    update: (id: string, input: CatalogUpdateInput) =>
      request<Catalog>('PATCH', `/catalogs/${id}`, { body: input }),
    remove: (id: string) => request<void>('DELETE', `/catalogs/${id}`),
    tracks: (id: string) => request<CatalogTrack[]>('GET', `/catalogs/${id}/tracks`),
    upload: (catalogId: string, file: File) => {
      const form = new FormData()
      form.append('file', file)
      return request<CatalogTrack>('POST', `/catalogs/${catalogId}/tracks`, { form })
    },
    createCaptionJob: (catalogId: string, body: CaptionJobInput) =>
      request<CaptionJobResult>('POST', `/catalogs/${catalogId}/caption-jobs`, { body }),
    previewLora: (catalogId: string, body: LoraBuildRequest) =>
      request<BuildPlan>('POST', `/catalogs/${encodeURIComponent(catalogId)}/lora-preview`, { body }),
    createLora: (catalogId: string, body: LoraBuildRequest) =>
      request<CreateLoraResult>('POST', `/catalogs/${encodeURIComponent(catalogId)}/loras`, { body }),
  },
  datasetSnapshots: {
    get: (id: string) => request<DatasetSnapshot>('GET', `/dataset-snapshots/${encodeURIComponent(id)}`),
  },
  catalogTracks: {
    update: (id: string, input: CatalogTrackUpdateInput) =>
      request<CatalogTrack>('PATCH', `/catalog-tracks/${id}`, { body: input }),
    remove: (id: string) => request<void>('DELETE', `/catalog-tracks/${id}`),
    reanalyze: (id: string) => request<CatalogTrack>('POST', `/catalog-tracks/${id}/reanalyze`),
    saveCaption: (id: string, modelKey: string, body: CaptionInput) =>
      request<CaptionView>('PUT', `/catalog-tracks/${id}/captions/${encodeURIComponent(modelKey)}`, { body }),
    undo: (id: string, modelKey: string) =>
      request<CaptionView>('POST', `/catalog-tracks/${id}/captions/${encodeURIComponent(modelKey)}/undo`),
    redo: (id: string, modelKey: string) =>
      request<CaptionView>('POST', `/catalog-tracks/${id}/captions/${encodeURIComponent(modelKey)}/redo`),
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
