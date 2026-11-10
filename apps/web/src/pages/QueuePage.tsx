import { useEffect, useState } from 'react'
import { useQueryClient } from '@tanstack/react-query'
import { Ban, FileText, ListMusic, RefreshCw, RotateCcw } from 'lucide-react'
import { Link } from 'react-router-dom'
import { isActiveJob, useJob, useJobs, useQueueSummary } from '../api/hooks'
import { useCancelJob, useRetryJob } from '../api/mutations'
import { JOB_STATUSES } from '../api/types'
import type { Job, JobListParams, JobStatus } from '../api/types'
import { useCurrentUser } from '../auth/AuthContext'
import { jobTitle } from '../components/JobCard'
import { Modal } from '../components/Modal'
import {
  EmptyState,
  ErrorState,
  JOB_STATUS_LABELS,
  LoadingState,
  PageHeader,
  ProgressBar,
  StatusBadge,
  Switch,
} from '../components/ui'
import { formatDateTime, formatElapsed, formatRelative, progressPercent } from '../lib/format'

const QUEUE_LIMIT = 100

function logText(log: Job['log']): string {
  if (!log) return ''
  return Array.isArray(log) ? log.join('\n') : log
}

function JobLogDrawer({ jobId, onClose }: { jobId: string; onClose: () => void }) {
  const jobQuery = useJob(jobId)
  const job = jobQuery.data

  return (
    <Modal title="Chi tiết job" variant="drawer" onClose={onClose}>
      {jobQuery.isPending ? (
        <LoadingState />
      ) : jobQuery.isError ? (
        <ErrorState error={jobQuery.error} onRetry={() => void jobQuery.refetch()} />
      ) : job ? (
        <div className="job-detail">
          <div className="job-detail-head">
            <h3>{jobTitle(job)}</h3>
            <StatusBadge status={job.status} />
          </div>
          <dl className="meta-grid compact">
            <div className="meta-item">
              <dt>Job ID</dt>
              <dd className="mono">{job.id}</dd>
            </div>
            <div className="meta-item">
              <dt>Loại</dt>
              <dd>{job.type}</dd>
            </div>
            <div className="meta-item">
              <dt>Chủ job</dt>
              <dd>{job.owner_name ?? '—'}</dd>
            </div>
            <div className="meta-item">
              <dt>Node</dt>
              <dd>{job.node_name ?? '—'}</dd>
            </div>
            <div className="meta-item">
              <dt>Lần thử</dt>
              <dd>
                {job.attempt}/{job.max_attempts}
              </dd>
            </div>
            <div className="meta-item">
              <dt>Tiến độ</dt>
              <dd>
                {progressPercent(job.progress)}%{job.stage ? ` · ${job.stage}` : ''}
              </dd>
            </div>
            <div className="meta-item">
              <dt>Tạo lúc</dt>
              <dd>{formatDateTime(job.created_at)}</dd>
            </div>
            <div className="meta-item">
              <dt>Bắt đầu</dt>
              <dd>{formatDateTime(job.started_at)}</dd>
            </div>
            <div className="meta-item">
              <dt>Kết thúc</dt>
              <dd>{formatDateTime(job.finished_at)}</dd>
            </div>
          </dl>
          {job.error && (
            <p className="job-error" role="alert">
              {job.error}
            </p>
          )}
          {job.track_id && (
            <Link to={`/tracks/${job.track_id}`} className="btn btn-sm" onClick={onClose}>
              Mở track kết quả
            </Link>
          )}
          <h4>Log</h4>
          {logText(job.log) ? (
            <pre className="code-block log">{logText(job.log)}</pre>
          ) : (
            <p className="muted">Job này chưa có log.</p>
          )}
          <h4>Request</h4>
          <pre className="code-block">{JSON.stringify(job.request, null, 2)}</pre>
          {job.result && (
            <>
              <h4>Result</h4>
              <pre className="code-block">{JSON.stringify(job.result, null, 2)}</pre>
            </>
          )}
        </div>
      ) : null}
    </Modal>
  )
}

export function QueuePage() {
  const user = useCurrentUser()
  const queryClient = useQueryClient()
  const [status, setStatus] = useState<JobStatus | ''>('')
  const [mine, setMine] = useState(false)
  const [logJobId, setLogJobId] = useState<string | null>(null)

  const params: JobListParams = { status: status || undefined, mine, limit: QUEUE_LIMIT }
  const jobsQuery = useJobs(params)
  const cancel = useCancelJob()
  const retry = useRetryJob()
  const busy = cancel.isPending || retry.isPending

  // The list stops polling once nothing is active; the (always polled) queue summary
  // tells us when someone else's job has appeared so the table can refresh.
  const summary = useQueueSummary().data
  const summaryKey = summary ? `${summary.queued}/${summary.preparing}/${summary.running}` : ''
  useEffect(() => {
    if (summaryKey) void queryClient.invalidateQueries({ queryKey: ['jobs', 'list'] })
  }, [summaryKey, queryClient])

  return (
    <div className="page">
      <PageHeader
        title="Hàng đợi"
        description={
          user.role === 'admin' ? 'Mọi job trong hệ thống, mới nhất trước.' : 'Các job của bạn, mới nhất trước.'
        }
        actions={
          <button type="button" className="btn" onClick={() => void jobsQuery.refetch()}>
            <RefreshCw size={16} aria-hidden className={jobsQuery.isFetching ? 'spin' : undefined} />
            Làm mới
          </button>
        }
      />

      <div className="toolbar">
        <select
          aria-label="Lọc theo trạng thái"
          value={status}
          onChange={(e) => setStatus(e.target.value as JobStatus | '')}
        >
          <option value="">Mọi trạng thái</option>
          {JOB_STATUSES.map((s) => (
            <option key={s} value={s}>
              {JOB_STATUS_LABELS[s]}
            </option>
          ))}
        </select>
        {user.role === 'admin' && <Switch label="Chỉ của tôi" checked={mine} onChange={setMine} />}
      </div>

      {jobsQuery.isPending ? (
        <LoadingState />
      ) : jobsQuery.isError ? (
        <ErrorState error={jobsQuery.error} onRetry={() => void jobsQuery.refetch()} />
      ) : jobsQuery.data.length === 0 ? (
        <EmptyState icon={<ListMusic size={26} aria-hidden />} title="Không có job nào">
          {status ? 'Không có job ở trạng thái này.' : 'Hàng đợi đang trống.'}
        </EmptyState>
      ) : (
        <div className="table-wrap">
          <table className="table">
            <thead>
              <tr>
                <th scope="col">Job</th>
                <th scope="col">Trạng thái</th>
                <th scope="col">Chủ job</th>
                <th scope="col">Node</th>
                <th scope="col">Tiến độ</th>
                <th scope="col">Tạo lúc</th>
                <th scope="col">Thời gian chạy</th>
                <th scope="col">
                  <span className="sr-only">Thao tác</span>
                </th>
              </tr>
            </thead>
            <tbody>
              {jobsQuery.data.map((job) => {
                const title = jobTitle(job)
                const percent = progressPercent(job.progress)
                const cancellable = isActiveJob(job) && !job.cancel_requested
                const retryable = job.status === 'failed' || job.status === 'cancelled'
                return (
                  <tr key={job.id}>
                    <td className="cell-title">
                      {job.track_id ? <Link to={`/tracks/${job.track_id}`}>{title}</Link> : <span>{title}</span>}
                      <small>{job.type}</small>
                    </td>
                    <td>
                      <StatusBadge status={job.status} />
                      {job.cancel_requested && isActiveJob(job) && <small>Đang hủy…</small>}
                    </td>
                    <td>{job.owner_name ?? '—'}</td>
                    <td>{job.node_name ?? '—'}</td>
                    <td className="cell-progress">
                      {job.status === 'preparing' || job.status === 'running' ? (
                        <>
                          <ProgressBar percent={percent} label={`Tiến độ ${title}`} />
                          <small>
                            {percent}%{job.stage ? ` · ${job.stage}` : ''}
                            {job.message ? ` · ${job.message}` : ''}
                          </small>
                        </>
                      ) : job.status === 'failed' ? (
                        <small className="text-danger">{job.error ?? job.message ?? '—'}</small>
                      ) : (
                        <small>{job.status === 'completed' ? '100%' : '—'}</small>
                      )}
                    </td>
                    <td title={formatDateTime(job.created_at)}>{formatRelative(job.created_at)}</td>
                    <td>
                      {job.started_at
                        ? formatElapsed(job.started_at, job.finished_at ?? new Date().toISOString())
                        : '—'}
                    </td>
                    <td className="cell-actions">
                      <button
                        type="button"
                        className="icon-btn"
                        aria-label={`Xem log ${title}`}
                        title="Xem log"
                        onClick={() => setLogJobId(job.id)}
                      >
                        <FileText size={16} aria-hidden />
                      </button>
                      {cancellable && (
                        <button
                          type="button"
                          className="icon-btn"
                          aria-label={`Hủy ${title}`}
                          title="Hủy"
                          disabled={busy}
                          onClick={() => cancel.mutate(job.id)}
                        >
                          <Ban size={16} aria-hidden />
                        </button>
                      )}
                      {retryable && (
                        <button
                          type="button"
                          className="icon-btn"
                          aria-label={`Thử lại ${title}`}
                          title="Thử lại"
                          disabled={busy}
                          onClick={() => retry.mutate(job.id)}
                        >
                          <RotateCcw size={16} aria-hidden />
                        </button>
                      )}
                    </td>
                  </tr>
                )
              })}
            </tbody>
          </table>
        </div>
      )}

      {logJobId && <JobLogDrawer jobId={logJobId} onClose={() => setLogJobId(null)} />}
    </div>
  )
}
