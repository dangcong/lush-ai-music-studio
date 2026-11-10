import { Ban, RotateCcw } from 'lucide-react'
import type { Job, Track } from '../api/types'
import { formatRelative, progressPercent } from '../lib/format'
import { ProgressBar, Spinner, StatusBadge } from './ui'
import { TrackCard } from './TrackCard'

interface JobCardProps {
  job: Job
  /** Resulting track of a completed job, once loaded. */
  track?: Track
  trackLoading?: boolean
  busy?: boolean
  onCancel: (job: Job) => void
  onRetry: (job: Job) => void
}

export function jobTitle(job: Job): string {
  return job.title || job.request.title || job.request.prompt || 'Không có tiêu đề'
}

/** One generation job, rendered from the API's real status/progress/stage/message. */
export function JobCard({ job, track, trackLoading, busy, onCancel, onRetry }: JobCardProps) {
  if (job.status === 'completed' && track) {
    return <TrackCard track={track} />
  }

  const title = jobTitle(job)
  const active = job.status === 'preparing' || job.status === 'running'
  const cancellable = job.status === 'queued' || active
  const retryable = job.status === 'failed' || job.status === 'cancelled'
  const percent = progressPercent(job.progress)

  return (
    <article className={`job-card job-${job.status}`} aria-label={title}>
      <header className="job-head">
        <div className="job-title-wrap">
          <h3 className="job-title">{title}</h3>
          <p className="job-sub">
            {formatRelative(job.created_at)}
            {job.node_name && active ? ` · ${job.node_name}` : ''}
            {job.attempt > 1 ? ` · lần thử ${job.attempt}/${job.max_attempts}` : ''}
          </p>
        </div>
        <StatusBadge status={job.status} />
      </header>

      {job.status === 'queued' && (
        <p className="job-text">{job.message || 'Đang chờ máy render nhận job.'}</p>
      )}

      {active && (
        <div className="job-progress">
          <ProgressBar percent={percent} label={`Tiến độ ${title}`} />
          <p className="job-text">
            <span className="job-percent">{percent}%</span>
            {job.stage && <span className="job-stage">{job.stage}</span>}
            {job.message && <span>{job.message}</span>}
          </p>
        </div>
      )}

      {job.status === 'completed' &&
        (trackLoading ? (
          <p className="job-text">
            <Spinner size={14} /> Đang tải kết quả…
          </p>
        ) : (
          <p className="job-text">Job đã hoàn tất nhưng không tìm thấy track kết quả.</p>
        ))}

      {job.status === 'failed' && (
        <p className="job-error" role="alert">
          {job.error || job.message || 'Job thất bại nhưng máy chủ không trả về chi tiết lỗi.'}
        </p>
      )}

      {job.status === 'cancelled' && <p className="job-text">Job đã bị hủy.</p>}

      {(cancellable || retryable) && (
        <footer className="job-actions">
          {cancellable &&
            (job.cancel_requested ? (
              <span className="job-text">Đang hủy…</span>
            ) : (
              <button type="button" className="btn btn-sm" disabled={busy} onClick={() => onCancel(job)}>
                <Ban size={14} aria-hidden />
                Hủy
              </button>
            ))}
          {retryable && (
            <button type="button" className="btn btn-sm" disabled={busy} onClick={() => onRetry(job)}>
              <RotateCcw size={14} aria-hidden />
              Thử lại
            </button>
          )}
        </footer>
      )}
    </article>
  )
}
