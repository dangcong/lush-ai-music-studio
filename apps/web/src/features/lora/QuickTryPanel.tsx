import { useState } from 'react'
import { Link } from 'react-router-dom'
import { errorMessage } from '../../api/client'
import { useQuickTry } from '../../api/mutations'
import type { Checkpoint, TrainingRunDetail } from '../../api/types'
import { epochLabel } from './EvalTable'
import { formatScale } from './PublishPanel'

interface QuickTryPanelProps {
  run: TrainingRunDetail
  selected: Checkpoint | null
  scale: number
}

/** "Thử nhanh": a normal generate job with the selected epoch, at the strength set in the publish panel. */
export function QuickTryPanel({ run, selected, scale }: QuickTryPanelProps) {
  const quickTry = useQuickTry(run.lora?.id ?? '')
  const [prompt, setPrompt] = useState(run.snapshot?.test_prompt ?? '')
  const canTry = selected !== null && prompt.trim().length > 0 && !quickTry.isPending

  return (
    <section className="panel review-side" aria-labelledby="quick-try-title">
      <h2 className="section-title" id="quick-try-title">
        Thử nhanh với câu lệnh của bạn
      </h2>
      <textarea
        rows={3}
        maxLength={500}
        aria-label="Câu lệnh thử"
        value={prompt}
        onChange={(e) => setPrompt(e.target.value)}
      />
      <div className="quick-try-row">
        <span className="chip">{selected ? epochLabel(selected) : 'Chưa chọn bản'}</span>
        <span className="chip">{`Độ mạnh ${formatScale(scale)}`}</span>
        <button
          type="button"
          className="btn"
          disabled={!canTry}
          onClick={() =>
            selected &&
            quickTry.mutate({ checkpoint_id: selected.id, prompt: prompt.trim(), lora_scale: scale })
          }
        >
          Tạo thử
        </button>
      </div>
      {quickTry.isError && (
        <p className="job-error" role="alert">
          {errorMessage(quickTry.error)}
        </p>
      )}
      {quickTry.isSuccess && (
        <p className="field-hint" role="status">
          Đã tạo job thử. <Link to="/queue">Xem trong Hàng đợi</Link>
        </p>
      )}
    </section>
  )
}
