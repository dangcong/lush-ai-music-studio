import { useEffect, useState } from 'react'
import type { ReactNode } from 'react'
import { ArrowLeft, Ban, TriangleAlert } from 'lucide-react'
import { Link, useParams } from 'react-router-dom'
import { TRAINING_POLL_MS, isActiveJob, useDatasetSnapshot, useModels, useTrainingRun } from '../api/hooks'
import { useCancelJob } from '../api/mutations'
import type { SnapshotSample, SnapshotSummary, TrainingMetric, TrainingResult, TrainingRunDetail } from '../api/types'
import { useCurrentUser } from '../auth/AuthContext'
import { ConfirmDialog } from '../components/Modal'
import { ErrorState, LoadingState, Notice, ProgressBar, StatusBadge } from '../components/ui'
import { ModelBar } from '../features/lora/ModelBar'
import { LossChart } from '../features/training/LossChart'
import { isStale, stageLabel } from '../features/training/trainingForm'
import { formatBytes, formatDateTime, formatDuration, progressPercent } from '../lib/format'
import { TrainingAdminOnly, trainingRunName } from './TrainingPage'

const NO_DATA = 'Chưa có dữ liệu'
const UNSUPPORTED = 'Không hỗ trợ'

const timeFormat = new Intl.DateTimeFormat('vi-VN', { hour: '2-digit', minute: '2-digit', hour12: false })

function formatClock(iso: string): string {
  return timeFormat.format(new Date(iso))
}

function isNumber(value: unknown): value is number {
  return typeof value === 'number' && Number.isFinite(value)
}

/** Re-renders on an interval so "time since the last update" is judged against a current clock. */
function useNow(intervalMs: number): number {
  const [now, setNow] = useState(() => Date.now())
  useEffect(() => {
    const id = window.setInterval(() => setNow(Date.now()), intervalMs)
    return () => window.clearInterval(id)
  }, [intervalMs])
  return now
}

function Item({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="meta-item">
      <dt>{label}</dt>
      <dd>{children}</dd>
    </div>
  )
}

/** The newest reported point: highest sequence, else the last one in the list. */
function latestMetric(metrics: TrainingMetric[]): TrainingMetric | undefined {
  let latest: TrainingMetric | undefined
  for (const m of metrics) {
    if (!latest || (isNumber(m.sequence) && isNumber(latest.sequence) ? m.sequence >= latest.sequence : true)) {
      latest = m
    }
  }
  return latest
}

const STOP_REASONS: Record<string, string> = {
  completed: 'Chạy hết số epoch',
  time_limit: 'Hết giới hạn thời gian',
}

function yesNo(value: boolean | null | undefined, yes: string, no: string): string {
  if (value === true) return yes
  if (value === false) return no
  return NO_DATA
}

function ResultSummary({ result }: { result: TrainingResult }) {
  return (
    <section className="panel" aria-label="Kết quả">
      <h2 className="section-title">Kết quả</h2>
      <dl className="meta-grid">
        <Item label="Lý do dừng">
          {result.stop_reason ? (STOP_REASONS[result.stop_reason] ?? result.stop_reason) : NO_DATA}
        </Item>
        <Item label="Epoch đã chạy">{isNumber(result.epochs_completed) ? result.epochs_completed : NO_DATA}</Item>
        <Item label="Optimizer steps">{isNumber(result.optimizer_steps) ? result.optimizer_steps : NO_DATA}</Item>
        <Item label="VRAM đỉnh">{isNumber(result.vram_peak_mb) ? `${Math.round(result.vram_peak_mb)} MB` : NO_DATA}</Item>
        <Item label="Thời gian tiền xử lý">
          {isNumber(result.preprocess_seconds) ? formatDuration(result.preprocess_seconds) : NO_DATA}
        </Item>
        <Item label="Thời gian train">
          {isNumber(result.train_seconds) ? formatDuration(result.train_seconds) : NO_DATA}
        </Item>
        <Item label="Engine tạo nhạc">
          {yesNo(result.inference_restored, 'Đã khôi phục', 'CHƯA khôi phục được')}
        </Item>
        <Item label="Nạp thử adapter">{yesNo(result.adapter_loadable, 'Nạp được', 'KHÔNG nạp được')}</Item>
        <Item label="Adapter">
          {result.lora?.key ? <span className="mono">{result.lora.key}</span> : NO_DATA}
          {isNumber(result.lora?.bytes) ? ` · ${formatBytes(result.lora.bytes)}` : ''}
        </Item>
      </dl>
      {result.inference_restored === false && (
        <Notice tone="danger">
          <span>Engine tạo nhạc chưa được khôi phục sau run này — cần kiểm tra Render Node trước khi tạo nhạc.</span>
        </Notice>
      )}
    </section>
  )
}

const DASH = '—'

function SnapshotTable({ snapshotId }: { snapshotId: string }) {
  const query = useDatasetSnapshot(snapshotId)
  if (query.isPending) return <LoadingState />
  if (query.isError) return <ErrorState error={query.error} onRetry={() => void query.refetch()} />
  const { samples, held_out } = query.data.content
  const rows: { sample: SnapshotSample; heldOut: boolean }[] = [
    ...samples.map((sample) => ({ sample, heldOut: false })),
    ...held_out.map((sample) => ({ sample, heldOut: true })),
  ]
  return (
    <div className="table-wrap">
      <table className="table" aria-label="Danh sách bài và mô tả">
        <thead>
          <tr>
            <th scope="col">Bài</th>
            <th scope="col">Model</th>
            <th scope="col">Mô tả khi train</th>
            <th scope="col">Tempo</th>
            <th scope="col">Key</th>
          </tr>
        </thead>
        <tbody>
          {rows.map(({ sample, heldOut }) => (
            <tr key={`${heldOut ? 'h' : 't'}-${sample.catalog_track_id}-${sample.filename}`}>
              <td>
                {sample.filename}
                {heldOut && <span className="badge"> Giữ lại</span>}
              </td>
              <td>{sample.model_key || DASH}</td>
              <td>{sample.train_caption || DASH}</td>
              <td>{isNumber(sample.bpm) ? sample.bpm : DASH}</td>
              <td>{sample.keyscale || DASH}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  )
}

function SnapshotPanel({ snapshot }: { snapshot: SnapshotSummary }) {
  const [open, setOpen] = useState(false)
  return (
    <section className="panel" aria-labelledby="snapshot-title">
      <h2 className="section-title" id="snapshot-title">
        LoRA này được train từ
      </h2>
      <dl className="meta-grid">
        <Item label="Catalog">
          <Link to={`/catalogs/${snapshot.catalog_id}`}>{snapshot.catalog_name}</Link>
        </Item>
        <Item label="Bài train">{`${snapshot.samples} bài train`}</Item>
        <Item label="Giữ lại">
          {snapshot.held_out.length === 0 ? 'Không có' : snapshot.held_out.join(', ')}
        </Item>
        <Item label="Từ khóa">{snapshot.keyword ? <span className="mono">{snapshot.keyword}</span> : 'Không có'}</Item>
        <Item label="Câu lệnh nghe thử">{snapshot.test_prompt || DASH}</Item>
      </dl>
      {snapshot.models.length > 0 && <ModelBar models={snapshot.models} />}
      <p>
        <button type="button" className="btn btn-sm" aria-expanded={open} onClick={() => setOpen((v) => !v)}>
          Xem danh sách bài và mô tả
        </button>
      </p>
      {open && <SnapshotTable snapshotId={snapshot.id} />}
    </section>
  )
}

function formatMb(bytes: number | null): string {
  if (!isNumber(bytes)) return DASH
  return String(Number((bytes / (1024 * 1024)).toFixed(1)))
}

type CheckpointRow = { id: string; epoch: number | null; step: number | null; bytes: number | null }

/** Snapshot runs have checkpoint rows; legacy runs only have `result.checkpoints` (epoch, bytes). */
function checkpointRows(run: TrainingRunDetail): CheckpointRow[] {
  if (run.checkpoints.length > 0) return run.checkpoints
  return (run.result?.checkpoints ?? []).map((c, i) => ({
    id: `legacy-${i}`,
    epoch: isNumber(c.epoch) ? c.epoch : null,
    step: null,
    bytes: isNumber(c.bytes) ? c.bytes : null,
  }))
}

function CheckpointsPanel({ checkpoints }: { checkpoints: CheckpointRow[] }) {
  return (
    <section className="panel" aria-labelledby="checkpoints-title">
      <h2 className="section-title" id="checkpoints-title">
        Bản lưu
      </h2>
      <ul>
        {checkpoints.map((c) => (
          <li key={c.id}>{`Epoch ${c.epoch ?? '?'} · bước ${c.step ?? DASH} · ${formatMb(c.bytes)} MB`}</li>
        ))}
      </ul>
      <p className="field-hint">Nghe thử và chọn bản sẽ có ở bước sau.</p>
    </section>
  )
}

function RunDetail({ run }: { run: TrainingRunDetail }) {
  const now = useNow(TRAINING_POLL_MS)
  const cancel = useCancelJob()
  const models = useModels().data
  const [confirming, setConfirming] = useState(false)

  const name = trainingRunName(run)
  const request = run.request ?? {}
  const metrics = run.metrics ?? []
  const latest = latestMetric(metrics)
  const active = isActiveJob(run)
  const working = run.status === 'preparing' || run.status === 'running'
  const stale = isStale(run.status, run.updated_at, now)
  const percent = progressPercent(run.progress)
  const checkpoints = checkpointRows(run)
  const baseModel = models?.find((m) => m.id === request.base_model_id)
  const lossCount = metrics.filter((m) => isNumber(m.train_loss) && isNumber(m.global_step)).length

  return (
    <>
      <header className="detail-head">
        <div className="detail-head-main">
          <h1>{name}</h1>
          <p className="page-desc">Train Style · training run</p>
        </div>
        <StatusBadge status={run.status} />
        {active &&
          (run.cancel_requested ? (
            <span className="muted">Đang hủy…</span>
          ) : (
            <button type="button" className="btn btn-danger-ghost" onClick={() => setConfirming(true)}>
              <Ban size={16} aria-hidden />
              Hủy run
            </button>
          ))}
      </header>

      {stale && run.updated_at && (
        <Notice tone="warn">
          <TriangleAlert size={16} aria-hidden />
          <span>
            <strong>Mất cập nhật từ {formatClock(run.updated_at)}</strong> — Render Node không báo gì thêm cho run này.
            Số liệu dưới đây là lần báo cuối, không phải trạng thái hiện tại.
          </span>
        </Notice>
      )}
      {run.status === 'failed' && (
        <p className="job-error" role="alert">
          {run.error || run.message || 'Run thất bại nhưng máy chủ không trả về chi tiết lỗi.'}
        </p>
      )}
      {run.status === 'queued' && (
        <Notice tone="info">
          <span>
            {run.message ||
              'Đang chờ Render Node nhận run. Các job tạo nhạc đang chờ luôn được chạy trước một training run mới.'}
          </span>
        </Notice>
      )}
      {active && (
        <p className="field-hint">
          Hủy run sẽ mất toàn bộ phần đã train sau checkpoint cuối cùng đã lưu. Trong lúc run chạy, website không tạo
          nhạc được.
        </p>
      )}

      <section className={stale ? 'panel is-stale' : 'panel'} aria-label="Tiến độ">
        <h2 className="section-title">Tiến độ</h2>
        {working && (
          <div className="run-progress">
            <ProgressBar percent={percent} label={`Tiến độ ${name}`} />
            <p className="job-text">
              <span className="job-percent">{percent}%</span>
              <span className="job-stage">{stageLabel(run.stage)}</span>
              {stale ? <span>(lần báo cuối — không còn cập nhật)</span> : run.message && <span>{run.message}</span>}
            </p>
          </div>
        )}
        <dl className="meta-grid">
          <Item label="Giai đoạn">{working ? stageLabel(run.stage) : '—'}</Item>
          <Item label="Node">{run.node_name ?? '—'}</Item>
          <Item label="Tạo lúc">{formatDateTime(run.created_at)}</Item>
          <Item label="Bắt đầu">{formatDateTime(run.started_at)}</Item>
          <Item label="Kết thúc">{formatDateTime(run.finished_at)}</Item>
          <Item label="Cập nhật cuối">{formatDateTime(run.updated_at)}</Item>
        </dl>
      </section>

      <section className="panel" aria-label="Số liệu training">
        <h2 className="section-title">Số liệu training</h2>
        <dl className="meta-grid">
          <Item label="Epoch mới nhất">{isNumber(latest?.epoch) ? latest.epoch : NO_DATA}</Item>
          <Item label="Optimizer step mới nhất">{isNumber(latest?.global_step) ? latest.global_step : NO_DATA}</Item>
          <Item label="Train loss mới nhất">
            {isNumber(latest?.train_loss) ? latest.train_loss.toFixed(4) : NO_DATA}
          </Item>
          <Item label="Learning rate">
            {isNumber(latest?.learning_rate) ? latest.learning_rate.toExponential(2) : NO_DATA}
          </Item>
          <Item label="Thời gian đã train">
            {isNumber(latest?.elapsed_seconds) ? formatDuration(latest.elapsed_seconds) : NO_DATA}
          </Item>
          <Item label="GPU memory">
            {isNumber(latest?.gpu_memory_mb) ? `${Math.round(latest.gpu_memory_mb)} MB` : NO_DATA}
          </Item>
          <Item label="Validation loss">{UNSUPPORTED}</Item>
          <Item label="Early stopping">{UNSUPPORTED}</Item>
        </dl>
        <h3 className="chart-title">Train loss</h3>
        <LossChart metrics={metrics} />
        {lossCount > 0 && (
          <p className="field-hint">“Step” là optimizer step do trainer báo về, không phải micro-batch.</p>
        )}
      </section>

      <section className="panel" aria-label="Cấu hình">
        <h2 className="section-title">Cấu hình</h2>
        <div className="table-wrap">
          <table className="table config-table">
            <tbody>
              {(
                [
                  ['Tên Train Style', request.name],
                  ['Dataset', request.dataset_key],
                  ['Base model', baseModel?.name ?? request.base_model_id],
                  ['Rank', request.rank],
                  ['Alpha', request.alpha],
                  ['Learning rate', request.learning_rate],
                  ['Epochs', request.epochs],
                  ['Gradient accumulation', request.gradient_accumulation],
                  ['Warmup steps', request.warmup_steps],
                  ['Seed', request.seed ?? 'Ngẫu nhiên'],
                ] as [string, unknown][]
              ).map(([label, value]) => (
                <tr key={label}>
                  <th scope="row">{label}</th>
                  <td className="mono">{value === undefined || value === null ? '—' : String(value)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
        <h3 className="chart-title">Giới hạn</h3>
        <dl className="meta-grid">
          <Item label="Giới hạn thời gian">
            {isNumber(request.max_minutes) ? `${request.max_minutes} phút` : '—'}
          </Item>
          <Item label="Lưu checkpoint">
            {isNumber(request.save_every) ? `Mỗi ${request.save_every} epoch (chỉ theo epoch)` : '—'}
          </Item>
          <Item label="Số epoch tối đa">{isNumber(request.epochs) ? request.epochs : '—'}</Item>
          <Item label="Dừng sớm theo validation">{UNSUPPORTED}</Item>
        </dl>
      </section>

      {run.snapshot && <SnapshotPanel snapshot={run.snapshot} />}
      {checkpoints.length > 0 && <CheckpointsPanel checkpoints={checkpoints} />}

      {run.status === 'completed' && run.result && <ResultSummary result={run.result} />}

      {run.lora && (
        <section className="panel" aria-label="Train Style">
          <h2 className="section-title">Train Style</h2>
          <p>
            <Link to="/lora-library" className="btn btn-sm">
              Mở “{run.lora.name}” trong Train Style Library
            </Link>{' '}
            <span className="mono muted">{run.lora.key}</span>
          </p>
        </section>
      )}

      <section className="panel" aria-label="Log">
        <h2 className="section-title">Log</h2>
        {run.log ? (
          <pre className="code-block log" tabIndex={0}>
            {run.log}
          </pre>
        ) : (
          <p className="muted">Run này chưa có log.</p>
        )}
      </section>

      {confirming && (
        <ConfirmDialog
          title="Hủy training run?"
          message={
            <>
              Hủy <strong>{name}</strong> sẽ mất toàn bộ phần đã train sau checkpoint cuối cùng đã lưu. Run đã hủy
              không chạy tiếp được.
            </>
          }
          confirmLabel="Hủy run"
          danger
          busy={cancel.isPending}
          onConfirm={() =>
            cancel.mutate(run.id, {
              onSettled: () => setConfirming(false),
            })
          }
          onClose={() => setConfirming(false)}
        />
      )}
    </>
  )
}

export function TrainingRunPage() {
  const { id } = useParams()
  const user = useCurrentUser()
  const canView = user.role === 'admin' || user.role === 'trainer'
  const runQuery = useTrainingRun(id, canView)

  if (!canView) return <TrainingAdminOnly />

  return (
    <div className="page training-run">
      <Link to="/training" className="back-link">
        <ArrowLeft size={16} aria-hidden />
        Train Style
      </Link>
      {runQuery.isPending ? (
        <LoadingState />
      ) : runQuery.isError ? (
        <ErrorState error={runQuery.error} onRetry={() => void runQuery.refetch()} />
      ) : (
        <RunDetail run={runQuery.data} />
      )}
    </div>
  )
}
