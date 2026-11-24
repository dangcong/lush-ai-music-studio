import { useMutation, useQueryClient } from '@tanstack/react-query'
import { api } from './api'
import { queryKeys } from './hooks'
import type { PublishInput, QuickTryInput, CaptionInput, CaptionJobInput, CatalogTrackUpdateInput, LoraBuildRequest, Track, TrackUpdateInput } from './types'
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
    // Training runs are jobs too and are cancelled through the same endpoint.
    void queryClient.invalidateQueries({ queryKey: ['training-runs'] })
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

/** Create a catalog and refresh the list. */
export function useCreateCatalog() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: api.catalogs.create,
    onSuccess: () => void queryClient.invalidateQueries({ queryKey: ['catalogs'] }),
  })
}

function useInvalidateCatalog(catalogId: string) {
  const queryClient = useQueryClient()
  return () => {
    void queryClient.invalidateQueries({ queryKey: queryKeys.catalogTracks(catalogId) })
    void queryClient.invalidateQueries({ queryKey: queryKeys.catalog(catalogId) })
  }
}

/** Queue a caption job for ticked tracks; the tracks refetch (and poll) afterwards. */
export function useCreateCaptionJob(catalogId: string) {
  const queryClient = useQueryClient()
  const invalidate = useInvalidateCatalog(catalogId)
  const toast = useToast()
  return useMutation({
    mutationFn: (input: CaptionJobInput) => api.catalogs.createCaptionJob(catalogId, input),
    onSuccess: () => {
      invalidate()
      void queryClient.invalidateQueries({ queryKey: ['lora-preview', catalogId] })
    },
    onError: (error) => toast.error(error),
  })
}

/** Soft-delete tracks one request at a time, stopping at the first failure. */
export function useDeleteCatalogTracks(catalogId: string) {
  const invalidate = useInvalidateCatalog(catalogId)
  const toast = useToast()
  return useMutation({
    mutationFn: async (ids: string[]) => {
      for (const id of ids) await api.catalogTracks.remove(id)
    },
    onSettled: invalidate,
    onError: (error) => toast.error(error),
  })
}

export function useReanalyzeCatalogTrack(catalogId: string) {
  const invalidate = useInvalidateCatalog(catalogId)
  const toast = useToast()
  return useMutation({
    mutationFn: (id: string) => api.catalogTracks.reanalyze(id),
    onSuccess: invalidate,
    onError: (error) => toast.error(error),
  })
}

/** Refresh a catalog's tracks and counters after uploads. */
export function useRefreshCatalog(catalogId: string) {
  return useInvalidateCatalog(catalogId)
}

/**
 * Per-track edits in the catalog table. Errors are not toasted: the cell that
 * triggered the call shows them inline. Every success refetches the tracks.
 */
export function useCatalogTrackEdits(catalogId: string) {
  const invalidate = useInvalidateCatalog(catalogId)
  const onSuccess = () => invalidate()
  const update = useMutation({
    mutationFn: ({ id, input }: { id: string; input: CatalogTrackUpdateInput }) => api.catalogTracks.update(id, input),
    onSuccess,
  })
  const saveCaption = useMutation({
    mutationFn: ({ id, modelKey, body }: { id: string; modelKey: string; body: CaptionInput }) =>
      api.catalogTracks.saveCaption(id, modelKey, body),
    onSuccess,
  })
  const undo = useMutation({
    mutationFn: ({ id, modelKey }: { id: string; modelKey: string }) => api.catalogTracks.undo(id, modelKey),
    onSuccess,
  })
  const redo = useMutation({
    mutationFn: ({ id, modelKey }: { id: string; modelKey: string }) => api.catalogTracks.redo(id, modelKey),
    onSuccess,
  })
  const rerun = useMutation({
    mutationFn: (input: CaptionJobInput) => api.catalogs.createCaptionJob(catalogId, input),
    onSuccess,
  })
  return { update, saveCaption, undo, redo, rerun }
}

/** Create a LoRA (snapshot + training job) from a catalog selection. Errors are shown by the dialog. */
export function useCreateLora(catalogId: string) {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: (body: LoraBuildRequest) => api.catalogs.createLora(catalogId, body),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ['training-runs'] })
      void queryClient.invalidateQueries({ queryKey: ['jobs'] })
      void queryClient.invalidateQueries({ queryKey: ['queue-summary'] })
      void queryClient.invalidateQueries({ queryKey: ['loras'] })
    },
  })
}

/** Refresh the review screen of a training run (checkpoints, files job, style). */
function useInvalidateRun() {
  const queryClient = useQueryClient()
  return () => {
    void queryClient.invalidateQueries({ queryKey: ['training-runs'] })
    void queryClient.invalidateQueries({ queryKey: ['loras'] })
  }
}

/** Save the note of a checkpoint (on blur). */
export function useSaveCheckpointNote() {
  const invalidate = useInvalidateRun()
  return useMutation({
    mutationFn: ({ id, note }: { id: string; note: string }) => api.loraCheckpoints.saveNote(id, note),
    onSuccess: invalidate,
  })
}

/** Put a checkpoint into the Style Preset; the files job runs on the node, the run detail shows it. */
export function usePublishCheckpoint(loraId: string) {
  const invalidate = useInvalidateRun()
  return useMutation({
    mutationFn: (body: PublishInput) => api.loras.publish(loraId, body),
    onSuccess: invalidate,
  })
}

/** Delete the kept (unchosen) checkpoints now. */
export function useCleanupCheckpoints(loraId: string) {
  const invalidate = useInvalidateRun()
  return useMutation({
    mutationFn: () => api.loras.cleanup(loraId),
    onSuccess: invalidate,
  })
}

export function useDeleteLora(loraId: string) {
  const invalidate = useInvalidateRun()
  return useMutation({
    mutationFn: () => api.loras.remove(loraId),
    onSuccess: invalidate,
  })
}

/** Quick try renders a normal generate job, so the jobs lists refresh too. */
export function useQuickTry(loraId: string) {
  const invalidate = useInvalidateJobs()
  return useMutation({
    mutationFn: (body: QuickTryInput) => api.loras.tryCheckpoint(loraId, body),
    onSuccess: invalidate,
  })
}
