import { useState } from 'react'
import { errorMessage } from '../../api/client'
import { useCleanupCheckpoints, usePublishCheckpoint } from '../../api/mutations'
import type { Checkpoint, TrainingRunDetail } from '../../api/types'
import { ConfirmDialog } from '../../components/Modal'
import { Field } from '../../components/ui'
import { epochLabel } from './EvalTable'

const LIVE = ['queued', 'preparing', 'running']

/** 0.7, 0.55, 1 — no trailing zeros. */
export function formatScale(scale: number): string {
  return String(Number(scale.toFixed(2)))
}

interface PublishPanelProps {
  run: TrainingRunDetail
  /** Row the user selected; null when none (or a removed one). */
  selected: Checkpoint | null
  scale: number
  onScale: (scale: number) => void
}

/** "Đưa vào Style Preset": publish the selected epoch, and delete the kept ones now. */
export function PublishPanel({ run, selected, scale, onScale }: PublishPanelProps) {
  const loraId = run.lora?.id ?? ''
  const publish = usePublishCheckpoint(loraId)
  const cleanup = useCleanupCheckpoints(loraId)
  const [name, setName] = useState(run.style_preset?.name ?? run.lora?.name ?? '')
  const [confirmCleanup, setConfirmCleanup] = useState(false)

  const files = run.files_job
  const filesLive = files !== null && LIVE.includes(files.status)
  const publishFailed = files?.op === 'publish' && files.status === 'failed'
  const label = selected ? epochLabel(selected) : null
  const canPublish = selected !== null && !filesLive && !publish.isPending && name.trim().length > 0

  const onPublish = () => {
    if (!selected) return
    publish.mutate({ checkpoint_id: selected.id, style_name: name.trim(), lora_scale: scale })
  }

  return (
    <section className="panel review-side" aria-labelledby="publish-title">
      <h2 className="section-title" id="publish-title">
        Đưa vào Style Preset
      </h2>
      <p className="review-pick">
        Bản chọn: {label ? <strong>{label}</strong> : <span className="muted">Chưa chọn bản nào</span>}
      </p>
      <Field label="Tên hiện ở Style Preset">
        <input type="text" maxLength={80} value={name} onChange={(e) => setName(e.target.value)} />
      </Field>
      <Field label="Độ mạnh mặc định" aside={formatScale(scale)}>
        <input
          type="range"
          className="range"
          min={0}
          max={1}
          step={0.05}
          value={scale}
          style={{ ['--fill' as string]: `${scale * 100}%` }}
          onChange={(e) => onScale(Number(e.target.value))}
        />
      </Field>
      <div className="range-ends" aria-hidden>
        <span>Nhẹ</span>
        <span>Mạnh</span>
      </div>
      <button type="button" className="btn btn-primary review-publish" disabled={!canPublish} onClick={onPublish}>
        {label ? `Đưa ${label} vào Style Preset` : 'Đưa vào Style Preset'}
      </button>
      {filesLive && (
        <p className="field-hint" role="status">
          {files.op === 'delete' ? 'Đang xóa bản lưu trên máy…' : 'Đang cập nhật trên máy…'}
        </p>
      )}
      {publishFailed && (
        <p className="job-error" role="alert">
          {`Đưa lên máy lỗi: ${files.error ?? 'không rõ'}. Bấm lại để thử lần nữa.`}
        </p>
      )}
      {publish.isError && (
        <p className="job-error" role="alert">
          {errorMessage(publish.error)}
        </p>
      )}
      {publish.isSuccess && publish.data.job_id === null && (
        <p className="field-hint" role="status">
          Bản này đã có trên máy, đã cập nhật Style Preset.
        </p>
      )}

      <p className="field-hint review-cleanup">
        Các bản không chọn được giữ 30 ngày rồi tự xóa để đỡ tốn ổ.{' '}
        <button
          type="button"
          className="btn btn-sm btn-danger-ghost"
          disabled={cleanup.isPending}
          onClick={() => setConfirmCleanup(true)}
        >
          Xóa ngay
        </button>
      </p>
      {cleanup.isError && (
        <p className="job-error" role="alert">
          {errorMessage(cleanup.error)}
        </p>
      )}
      {cleanup.isSuccess && (
        <p className="field-hint" role="status">
          {cleanup.data.count === 0 ? 'Không còn bản nào để xóa.' : `Đang xóa ${cleanup.data.count} bản trên máy…`}
        </p>
      )}

      {confirmCleanup && (
        <ConfirmDialog
          title="Xóa ngay các bản không chọn?"
          message="Các bản lưu không được chọn sẽ bị xóa khỏi máy ngay, không lấy lại được. Bản đang chọn được giữ."
          confirmLabel="Xóa ngay"
          danger
          busy={cleanup.isPending}
          onConfirm={() => cleanup.mutate(undefined, { onSettled: () => setConfirmCleanup(false) })}
          onClose={() => setConfirmCleanup(false)}
        />
      )}
    </section>
  )
}
