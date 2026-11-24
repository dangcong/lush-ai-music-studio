import { keepPreviousData, useQuery } from '@tanstack/react-query'
import { api } from './api'
import type { CatalogTrack, Job, LoraBuildRequest, JobListParams, JobStatus, TrackListParams } from './types'

export const JOB_POLL_MS = 2000
/** Training runs last minutes to hours; the worker reports far less often than a generate job. */
export const TRAINING_POLL_MS = 5000
const IDLE_SUMMARY_POLL_MS = 15000

const ACTIVE_STATUSES: JobStatus[] = ['queued', 'preparing', 'running']

export function isActiveJob(job: Pick<Job, 'status'>): boolean {
  return ACTIVE_STATUSES.includes(job.status)
}

export const queryKeys = {
  genres: (includeInactive: boolean) => ['genres', { includeInactive }] as const,
  presets: (includeInactive: boolean) => ['presets', { includeInactive }] as const,
  models: ['models'] as const,
  loras: ['loras'] as const,
  datasets: ['datasets'] as const,
  catalogs: ['catalogs', 'list'] as const,
  catalog: (id: string) => ['catalogs', 'detail', id] as const,
  catalogTracks: (id: string) => ['catalogs', 'tracks', id] as const,
  trainingRuns: ['training-runs', 'list'] as const,
  trainingRun: (id: string) => ['training-runs', 'detail', id] as const,
  jobs: (params: JobListParams) => ['jobs', 'list', params] as const,
  job: (id: string) => ['jobs', 'detail', id] as const,
  queueSummary: ['queue-summary'] as const,
  tracks: (params: TrackListParams) => ['tracks', 'list', params] as const,
  track: (id: string) => ['tracks', 'detail', id] as const,
  nodes: ['nodes'] as const,
  users: ['users'] as const,
  auditLogs: ['audit-logs'] as const,
}

export function useGenres(includeInactive = false) {
  return useQuery({
    queryKey: queryKeys.genres(includeInactive),
    queryFn: () => api.genres.list(includeInactive),
  })
}

export function usePresets(includeInactive = false) {
  return useQuery({
    queryKey: queryKeys.presets(includeInactive),
    queryFn: () => api.presets.list({ include_inactive: includeInactive }),
  })
}

export function useModels() {
  return useQuery({ queryKey: queryKeys.models, queryFn: api.models.list })
}

/** Active captioner models (Catalog description models); installed or not, the UI decides what to grey out. */
export function useCaptioners() {
  return useQuery({
    queryKey: queryKeys.models,
    queryFn: api.models.list,
    select: (models) => models.filter((m) => m.kind === 'captioner' && m.status === 'active'),
  })
}

export function useLoras() {
  return useQuery({ queryKey: queryKeys.loras, queryFn: api.loras.list })
}

export function useDatasets(enabled = true) {
  return useQuery({ queryKey: queryKeys.datasets, queryFn: api.datasets.list, enabled })
}

/** Training runs (admin only); polls every 5s only while one is unfinished. */
export function useTrainingRuns(enabled = true) {
  return useQuery({
    queryKey: queryKeys.trainingRuns,
    queryFn: api.trainingRuns.list,
    enabled,
    refetchInterval: (query) => (query.state.data?.some(isActiveJob) ? TRAINING_POLL_MS : false),
  })
}

/** Faster poll while the after-train screen waits on an eval_lora or lora_files job. */
export const REVIEW_POLL_MS = 2000

export function useTrainingRun(id: string | undefined, enabled = true) {
  return useQuery({
    queryKey: queryKeys.trainingRun(id ?? ''),
    queryFn: () => api.trainingRuns.get(id as string),
    enabled: Boolean(id) && enabled,
    refetchInterval: (query) => {
      const run = query.state.data
      if (!run) return false
      const live = (status: string | undefined) => status !== undefined && ACTIVE_STATUSES.includes(status as JobStatus)
      if (live(run.eval?.status) || live(run.files_job?.status)) return REVIEW_POLL_MS
      return isActiveJob(run) ? TRAINING_POLL_MS : false
    },
  })
}

/** Job list that polls every 2s only while it contains an unfinished job. */
export function useJobs(params: JobListParams) {
  return useQuery({
    queryKey: queryKeys.jobs(params),
    queryFn: () => api.jobs.list(params),
    refetchInterval: (query) => (query.state.data?.some(isActiveJob) ? JOB_POLL_MS : false),
  })
}

/**
 * One job, polled every 2s while it is unfinished. `pollWithoutData`: also keep polling while no answer has
 * arrived yet (a failed first fetch), for a job the page is waiting on and knows exists.
 */
export function useJob(id: string | null, { pollWithoutData = false }: { pollWithoutData?: boolean } = {}) {
  return useQuery({
    queryKey: queryKeys.job(id ?? ''),
    queryFn: () => api.jobs.get(id as string),
    enabled: id !== null,
    refetchInterval: (query) => {
      const job = query.state.data
      return job ? (isActiveJob(job) ? JOB_POLL_MS : false) : pollWithoutData ? JOB_POLL_MS : false
    },
  })
}

/** Polls fast while the queue is busy, slowly when idle (to notice others' jobs and node status). */
export function useQueueSummary() {
  return useQuery({
    queryKey: queryKeys.queueSummary,
    queryFn: api.queue.summary,
    refetchInterval: (query) => {
      const s = query.state.data
      const busy = s ? s.queued + s.preparing + s.running > 0 : false
      return busy ? JOB_POLL_MS : IDLE_SUMMARY_POLL_MS
    },
  })
}

export function useTracks(params: TrackListParams) {
  return useQuery({
    queryKey: queryKeys.tracks(params),
    queryFn: () => api.tracks.list(params),
  })
}

export function useTrack(id: string | null | undefined, enabled = true) {
  return useQuery({
    queryKey: queryKeys.track(id ?? ''),
    queryFn: () => api.tracks.get(id as string),
    enabled: Boolean(id) && enabled,
  })
}

export function useCatalogs(enabled = true) {
  return useQuery({ queryKey: queryKeys.catalogs, queryFn: api.catalogs.list, enabled })
}

export function useCatalog(id: string | undefined) {
  return useQuery({
    queryKey: queryKeys.catalog(id ?? ''),
    queryFn: () => api.catalogs.get(id as string),
    enabled: Boolean(id),
  })
}

function isCatalogTrackBusy(track: Pick<CatalogTrack, 'analysis_status' | 'captioning_model_key'>): boolean {
  return track.analysis_status === 'pending' || track.captioning_model_key !== null
}

/** Polls every 2s only while a track is being analysed or captioned. */
export function useCatalogTracks(id: string | undefined) {
  return useQuery({
    queryKey: queryKeys.catalogTracks(id ?? ''),
    queryFn: () => api.catalogs.tracks(id as string),
    enabled: Boolean(id),
    refetchInterval: (query) => (query.state.data?.some(isCatalogTrackBusy) ? JOB_POLL_MS : false),
  })
}

/** Dry-run of a LoRA build; keeps the previous plan visible while the next one loads. */
export function useLoraPreview(catalogId: string, body: LoraBuildRequest | null) {
  return useQuery({
    queryKey: ['lora-preview', catalogId, body],
    queryFn: () => api.catalogs.previewLora(catalogId, body as LoraBuildRequest),
    placeholderData: keepPreviousData,
    enabled: body !== null,
  })
}

/** The immutable dataset snapshot a LoRA was trained from. */
export function useDatasetSnapshot(id: string | undefined) {
  return useQuery({
    queryKey: ['dataset-snapshots', id ?? ''],
    queryFn: () => api.datasetSnapshots.get(id as string),
    enabled: !!id,
  })
}
