import { useState } from 'react'
import { GraduationCap, Plus } from 'lucide-react'
import { Link, useNavigate } from 'react-router-dom'
import { isActiveJob, useTrainingRuns } from '../api/hooks'
import type { TrainingRun } from '../api/types'
import { useCurrentUser } from '../auth/AuthContext'
import { EmptyState, ErrorState, LoadingState, Notice, PageHeader, ProgressBar, StatusBadge } from '../components/ui'
import { TrainingRunForm } from '../features/training/TrainingRunForm'
import { stageLabel } from '../features/training/trainingForm'
import { formatDateTime, formatRelative, progressPercent } from '../lib/format'

const TITLE = 'Train Style'

export function trainingRunName(run: Pick<TrainingRun, 'title' | 'request'>): string {
  return run.title || run.request?.name || 'Không có tên'
}

export function TrainingAdminOnly() {
  return (
    <div className="page">
      <PageHeader title={TITLE} />
      <Notice tone="info">
        <span>Chỉ admin và trainer mới xem được training run. Train Style đã duyệt có ở trang Train Style Library.</span>
      </Notice>
    </div>
  )
}

function RunRow({ run }: { run: TrainingRun }) {
  const name = trainingRunName(run)
  const running = run.status === 'preparing' || run.status === 'running'
  const percent = progressPercent(run.progress)
  return (
    <tr>
      <td className="cell-title">
        <Link to={`/training/${run.id}`}>{name}</Link>
      </td>
      <td>
        <span className="mono">{run.request?.dataset_key ?? '—'}</span>
      </td>
      <td>
        <StatusBadge status={run.status} />
        {run.cancel_requested && isActiveJob(run) && <small>Đang hủy…</small>}
      </td>
      <td>{running ? stageLabel(run.stage) : '—'}</td>
      <td className="cell-progress">
        {running ? (
          <>
            <ProgressBar percent={percent} label={`Tiến độ ${name}`} />
            <small>{percent}%</small>
          </>
        ) : run.status === 'failed' ? (
          <small className="text-danger">{run.error ?? run.message ?? '—'}</small>
        ) : (
          <small>{run.status === 'completed' ? '100%' : '—'}</small>
        )}
      </td>
      <td title={formatDateTime(run.created_at)}>{formatRelative(run.created_at)}</td>
    </tr>
  )
}

export function TrainingPage() {
  const user = useCurrentUser()
  const isAdmin = user.role === 'admin'
  const canView = isAdmin || user.role === 'trainer'
  const navigate = useNavigate()
  const runsQuery = useTrainingRuns(canView)
  const [creating, setCreating] = useState(false)

  if (!canView) return <TrainingAdminOnly />

  const hasActive = runsQuery.data?.some(isActiveJob) ?? false

  return (
    <div className="page">
      <PageHeader
        title={TITLE}
        description="Tạo một Train Style (về kỹ thuật là Train Style adapter phong cách, nhạc không lời) từ dataset trên Render Node. Base model không bị thay đổi."
        actions={
          <Link to="/datasets" className="btn btn-primary">
            <Plus size={16} aria-hidden />
            Tạo LoRA từ Catalog
          </Link>
        }
      />

      {isAdmin && (
        <details className="panel">
          <summary>Train từ dataset có sẵn trên máy (cách cũ)</summary>
          <p>
            <button type="button" className="btn" onClick={() => setCreating(true)}>
              Tạo training run
            </button>
          </p>
        </details>
      )}

      {hasActive && (
        <Notice tone="warn">
          <span>
            Đang có training run chưa kết thúc. Mỗi lúc chỉ chạy được một run; trong lúc run chạy, website không tạo
            nhạc được và các job tạo nhạc nằm chờ trong hàng đợi.
          </span>
        </Notice>
      )}

      {runsQuery.isPending ? (
        <LoadingState />
      ) : runsQuery.isError ? (
        <ErrorState error={runsQuery.error} onRetry={() => void runsQuery.refetch()} />
      ) : runsQuery.data.length === 0 ? (
        <EmptyState icon={<GraduationCap size={26} aria-hidden />} title="Chưa có training run nào">
          Bấm “Tạo LoRA từ Catalog” để tạo Train Style đầu tiên.
        </EmptyState>
      ) : (
        <div className="table-wrap">
          <table className="table">
            <thead>
              <tr>
                <th scope="col">Tên Train Style</th>
                <th scope="col">Dataset</th>
                <th scope="col">Trạng thái</th>
                <th scope="col">Giai đoạn</th>
                <th scope="col">Tiến độ</th>
                <th scope="col">Tạo lúc</th>
              </tr>
            </thead>
            <tbody>
              {runsQuery.data.map((run) => (
                <RunRow key={run.id} run={run} />
              ))}
            </tbody>
          </table>
        </div>
      )}

      {isAdmin && creating && (
        <TrainingRunForm
          onClose={() => setCreating(false)}
          onCreated={(run) => {
            setCreating(false)
            if (run?.id) navigate(`/training/${run.id}`)
          }}
        />
      )}
    </div>
  )
}
