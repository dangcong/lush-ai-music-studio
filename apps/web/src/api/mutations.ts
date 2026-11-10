import { useMutation, useQueryClient } from '@tanstack/react-query'
import { api } from './api'
import type { Track, TrackUpdateInput } from './types'
import { useToast } from '../components/Toast'

/** PATCH a track and refresh every cached copy of it (lists and detail). */
export function useUpdateTrack() {
  const queryClient = useQueryClient()
  const toast = useToast()
  return useMutation({
    mutationFn: ({ id, input }: { id: string; input: TrackUpdateInput }) =>
      api.tracks.update(id, input),
    onSuccess: (updated: Track | undefined) => {
      // Tolerate a backend that answers PATCH without a body.
      if (updated) {
        queryClient.setQueriesData<Track[] | Track>({ queryKey: ['tracks'] }, (data) => {
          if (!data) return data
          if (Array.isArray(data)) return data.map((t) => (t.id === updated.id ? updated : t))
          return data.id === updated.id ? updated : data
        })
      }
      void queryClient.invalidateQueries({ queryKey: ['tracks'] })
    },
    onError: (error) => toast.error(error),
  })
}

function useInvalidateJobs() {
  const queryClient = useQueryClient()
  return () => {
    void queryClient.invalidateQueries({ queryKey: ['jobs'] })
    void queryClient.invalidateQueries({ queryKey: ['queue-summary'] })
  }
}

export function useCancelJob() {
  const invalidate = useInvalidateJobs()
  const toast = useToast()
  return useMutation({
    mutationFn: (id: string) => api.jobs.cancel(id),
    onSuccess: invalidate,
    onError: (error) => toast.error(error),
  })
}

export function useRetryJob() {
  const invalidate = useInvalidateJobs()
  const toast = useToast()
  return useMutation({
    mutationFn: (id: string) => api.jobs.retry(id),
    onSuccess: () => {
      invalidate()
      toast.success('Đã tạo job mới trong hàng đợi.')
    },
    onError: (error) => toast.error(error),
  })
}
