import { useQuery } from '@tanstack/react-query'
import { api } from './api'
import type { Job, JobListParams, JobStatus, TrackListParams } from './types'

export const JOB_POLL_MS = 2000
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

export function useLoras() {
  return useQuery({ queryKey: queryKeys.loras, queryFn: api.loras.list })
}

/** Job list that polls every 2s only while it contains an unfinished job. */
export function useJobs(params: JobListParams) {
  return useQuery({
    queryKey: queryKeys.jobs(params),
    queryFn: () => api.jobs.list(params),
    refetchInterval: (query) => (query.state.data?.some(isActiveJob) ? JOB_POLL_MS : false),
  })
}

export function useJob(id: string | null) {
  return useQuery({
    queryKey: queryKeys.job(id ?? ''),
    queryFn: () => api.jobs.get(id as string),
    enabled: id !== null,
    refetchInterval: (query) =>
      query.state.data && isActiveJob(query.state.data) ? JOB_POLL_MS : false,
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
