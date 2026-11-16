import { useEffect, useRef } from 'react'
import { useQueryClient } from '@tanstack/react-query'
import { AudioLines } from 'lucide-react'
import { useJobs, useQueueSummary, useTrack, useTracks } from '../../api/hooks'
import { useCancelJob, useRetryJob } from '../../api/mutations'
import type { Job, Track } from '../../api/types'
import { JobCard } from '../../components/JobCard'
import { EmptyState, ErrorState, LoadingState } from '../../components/ui'

const RECENT_LIMIT = 20
// Only generate jobs: training runs (`train_lora`) have their own page.
const JOB_PARAMS = { mine: true, type: 'generate', limit: RECENT_LIMIT } as const
const TRACK_PARAMS = { mine: true, limit: 50 } as const

function RecentJobItem({
  job,
  knownTrack,
  trainingTitle,
}: {
  job: Job
  knownTrack: Track | undefined
  trainingTitle: string | undefined
}) {
  const cancel = useCancelJob()
  const retry = useRetryJob()
  // Fall back to a single fetch when the finished track is not in the recent list.
  const needsFetch = job.status === 'completed' && !knownTrack && Boolean(job.track_id)
  const trackQuery = useTrack(job.track_id, needsFetch)
  return (
    <JobCard
      job={job}
      track={knownTrack ?? trackQuery.data}
      trackLoading={needsFetch && trackQuery.isPending}
      busy={cancel.isPending || retry.isPending}
      trainingTitle={trainingTitle}
      onCancel={(j) => cancel.mutate(j.id)}
      onRetry={(j) => retry.mutate(j.id)}
    />
  )
}

/** The signed-in user's latest jobs and results; polls only while a job is unfinished. */
export function RecentJobs() {
  const queryClient = useQueryClient()
  const jobsQuery = useJobs(JOB_PARAMS)
  const tracksQuery = useTracks(TRACK_PARAMS)
  const training = useQueueSummary().data?.training

  // When another job completes, its track becomes available: refresh track lists.
  const completedCount = jobsQuery.data?.filter((j) => j.status === 'completed').length
  const previousCompleted = useRef<number | undefined>(undefined)
  useEffect(() => {
    if (completedCount === undefined) return
    if (previousCompleted.current !== undefined && completedCount !== previousCompleted.current) {
      void queryClient.invalidateQueries({ queryKey: ['tracks'] })
    }
    previousCompleted.current = completedCount
  }, [completedCount, queryClient])

  if (jobsQuery.isPending) return <LoadingState />
  if (jobsQuery.isError) return <ErrorState error={jobsQuery.error} onRetry={() => void jobsQuery.refetch()} />

  // Defensive: should the API ever return other job types here, do not render them as music jobs.
  const jobs = jobsQuery.data.filter((j) => j.type === 'generate')
  if (jobs.length === 0) {
    return (
      <EmptyState icon={<AudioLines size={26} aria-hidden />} title="Chưa có bản nhạc nào">
        Nhập Prompt ở bên trái rồi bấm Generate. Kết quả sẽ xuất hiện tại đây.
      </EmptyState>
    )
  }

  const tracksByJob = new Map<string, Track>()
  for (const t of tracksQuery.data ?? []) {
    if (t.job_id) tracksByJob.set(t.job_id, t)
  }

  return (
    <ul className="job-list">
      {jobs.map((job) => (
        <li key={job.id}>
          <RecentJobItem
            job={job}
            knownTrack={tracksByJob.get(job.id)}
            trainingTitle={training ? (training.title ?? '') : undefined}
          />
        </li>
      ))}
    </ul>
  )
}
