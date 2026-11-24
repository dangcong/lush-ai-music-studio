import { useEffect, useMemo, useState } from 'react'
import type { ReactNode } from 'react'
import { useNavigate } from 'react-router-dom'
import { errorMessage } from '../../api/client'
import { useLoraPreview, useModels, useTrainingRuns } from '../../api/hooks'
import { useCreateCaptionJob, useCreateLora } from '../../api/mutations'
import type { BuildTrack } from '../../api/types'
import { Modal } from '../../components/Modal'
import { Field, Notice } from '../../components/ui'
import { cx } from '../../lib/cx'
import {
  buildLoraRequest,
  defaultTestPrompt,
  estimateMinutes,
  initialCreateLoraState,
  localProblem,
  setKeyword,
  setKind,
} from './createLoraForm'
import type { CreateLoraState, TrainKind } from './createLoraForm'

const TRAINABLE_ENGINE = 'acestep'
const DEBOUNCE_MS = 300
const SEGMENT_COLORS = ['#7c6cf0', '#2fa37a', '#d98a2b', '#d9577a', '#3b9ac4', '#8f8f3a']

const KINDS: { kind: TrainKind; label: string }[] = [
  { kind: 'quick', label: 'Thử nhanh · 1 epoch' },
  { kind: 'standard', label: 'Chuẩn · 4 epoch' },
  { kind: 'thorough', label: 'Kỹ · 8 epoch' },
  { kind: 'custom', label: 'Tự nhập' },
]

function useDebounced<T>(value: T, ms: number): T {
  const [debounced, setDebounced] = useState(value)
  useEffect(() => {
    const timer = setTimeout(() => setDebounced(value), ms)
    return () => clearTimeout(timer)
  }, [value, ms])
  return debounced
}

interface Props {
  catalogId: string
  genreName: string
  trackIds: string[]
  onClose: () => void
  onDeselect: (ids: string[]) => void
  captionModelKey: string
}

export function CreateLoraDialog({ catalogId, genreName, trackIds, onClose, onDeselect, captionModelKey }: Props) {
  const navigate = useNavigate()
  const modelsQuery = useModels()
  const runsQuery = useTrainingRuns()
  const createLora = useCreateLora(catalogId)
  const captionJob = useCreateCaptionJob(catalogId)
  const [raw, setRaw] = useState<CreateLoraState>(() => initialCreateLoraState('', genreName))

  const baseModels = useMemo(
    () =>
      (modelsQuery.data ?? []).filter(
        (m) => m.kind === 'generator' && m.status === 'active' && m.engine === TRAINABLE_ENGINE,
      ),
    [modelsQuery.data],
  )
  const baseModelId = baseModels.some((m) => m.id === raw.baseModelId) ? raw.baseModelId : (baseModels[0]?.id ?? '')
  const state = useMemo(() => ({ ...raw, baseModelId }), [raw, baseModelId])
  const set = (patch: Partial<CreateLoraState>) => setRaw((s) => ({ ...s, ...patch }))

  const problem = localProblem(state)
  const body = useMemo(() => buildLoraRequest(state, trackIds), [state, trackIds])
  const debouncedBody = useDebounced(body, DEBOUNCE_MS)
  const settled = JSON.stringify(body) === JSON.stringify(debouncedBody)
  const preview = useLoraPreview(catalogId, debouncedBody)
  const plan = preview.data

  const exclude = (id: string) => {
    setRaw((s) => (s.excluded.includes(id) ? s : { ...s, excluded: [...s.excluded, id] }))
    onDeselect([id])
  }
  const confirmVoice = (id: string) =>
    setRaw((s) => (s.voiceConfirmed.includes(id) ? s : { ...s, voiceConfirmed: [...s.voiceConfirmed, id] }))

  const minutes = plan ? estimateMinutes(runsQuery.data ?? [], plan.total_steps) : null
  const canStart = problem === null && !!plan && plan.ready && settled && !createLora.isPending

  const start = () => {
    if (!body || !canStart) return
    createLora.mutate(body, { onSuccess: (r) => navigate(`/training/${r.job_id}`) })
  }

  const trackLine = (t: BuildTrack) => (
    <span className="lora-block-name">
      {t.filename}
      {t.vocals ? <span className="lora-block-note"> · {t.vocals}</span> : null}
    </span>
  )

  const num = (label: string, key: keyof CreateLoraState, extra: { min?: number; step?: number | 'any'; hint?: string; placeholder?: string } = {}) => (
    <Field label={label} hint={extra.hint}>
      <input
        type="number"
        min={extra.min}
        step={extra.step ?? 1}
        placeholder={extra.placeholder}
        value={state[key] as string}
        onChange={(e) => set({ [key]: e.target.value } as Partial<CreateLoraState>)}
      />
    </Field>
  )

  const selectedCount = plan ? plan.train.length + plan.held_out.length : 0

  return (
    <Modal
      title="Tạo LoRA"
      onClose={onClose}
      size="lg"
      footer={
        <>
          <button type="button" className="btn" onClick={onClose}>
            Hủy
          </button>
          <button type="button" className="btn btn-primary" disabled={!canStart} onClick={start}>
            Bắt đầu train
          </button>
        </>
      }
    >
      <div className="lora-dialog">
        <div className="lora-col">
          <section className="lora-group">
            <h3>Cần điền</h3>
            <Field label="Tên LoRA *" hint="Tên hiện trong Style Preset khi tạo nhạc.">
              <input type="text" value={state.name} onChange={(e) => set({ name: e.target.value })} />
            </Field>
            <Field
              label="Từ khóa gọi style"
              hint="Tự thêm vào đầu mọi mô tả khi train. Khi tạo nhạc, gõ từ này để gọi đúng style."
            >
              <input
                type="text"
                value={state.keyword}
                onChange={(e) => setRaw((s) => setKeyword(s, e.target.value, genreName))}
              />
            </Field>
          </section>

          <section className="lora-group">
            <h3>Nên chỉnh</h3>
            <div className="field">
              <span className="field-label">Kiểu train</span>
              <div className="lora-kinds" role="group" aria-label="Kiểu train">
                {KINDS.map(({ kind, label }) => (
                  <button
                    key={kind}
                    type="button"
                    className={cx('btn btn-sm', state.kind === kind && 'btn-primary')}
                    aria-pressed={state.kind === kind}
                    onClick={() => setRaw((s) => setKind(s, kind))}
                  >
                    {label}
                  </button>
                ))}
              </div>
            </div>
            <Field label="Epoch">
              <input
                type="number"
                min={1}
                value={state.epochs}
                onChange={(e) => setRaw((s) => ({ ...s, kind: 'custom', epochs: e.target.value }))}
              />
            </Field>
            {num('Lưu bản mỗi N epoch', 'saveEvery', { min: 1 })}
            <div className="field">
              <span className="field-label">Bài giữ lại để nghe thử</span>
              {plan && plan.held_out.length > 0 ? (
                <ul className="lora-list">
                  {plan.held_out.map((t) => (
                    <li key={t.id}>{t.filename}</li>
                  ))}
                </ul>
              ) : (
                <p className="field-hint">Chưa có bài nào.</p>
              )}
            </div>
            <Field label="Câu lệnh nghe thử sau khi train">
              <textarea
                rows={2}
                value={state.testPrompt}
                onChange={(e) => set({ testPrompt: e.target.value, testPromptEdited: true })}
                placeholder={defaultTestPrompt(state.keyword, genreName)}
              />
            </Field>
          </section>

          <details className="lora-group lora-advanced">
            <summary>Mặc định · Nâng cao</summary>
            <div className="lora-grid">
              {num('Rank', 'rank', { min: 1 })}
              {num('Alpha', 'alpha', { min: 1 })}
              {num('Learning rate', 'learningRate', { step: 'any' })}
              {num('Gradient accumulation', 'gradientAccumulation', { min: 1 })}
              {num('Warmup steps', 'warmupSteps', { min: 0 })}
              {num('Seed', 'seed', { min: 0, placeholder: 'Ngẫu nhiên' })}
              {num('Giới hạn thời gian', 'maxMinutes', { min: 1, hint: 'Đơn vị: phút.' })}
            </div>
            <p className="field-hint">Độ dài tối đa mỗi bài: 4 phút đầu</p>
            <Field label="Base model">
              <select value={baseModelId} onChange={(e) => set({ baseModelId: e.target.value })}>
                {baseModels.length === 0 && <option value="">Chưa có model ACE-Step</option>}
                {baseModels.map((m) => (
                  <option key={m.id} value={m.id}>
                    {m.name}
                  </option>
                ))}
              </select>
            </Field>
          </details>
        </div>

        <div className="lora-col">
          <section className="lora-group">
            <h3>Bài đưa vào LoRA</h3>
            {plan ? (
              <>
                <p>
                  <strong>{selectedCount} bài đã chọn</strong>
                </p>
                <p>
                  {plan.train.length} train · {plan.held_out.length} nghe thử
                </p>
                <p>
                  {Math.round(plan.train_seconds / 60)} phút tổng nhạc train
                  {plan.unknown_duration > 0 && ` (${plan.unknown_duration} bài chưa rõ thời lượng)`}
                </p>
              </>
            ) : (
              <p className="field-hint">{preview.isError ? 'Chưa kiểm tra được.' : 'Đang kiểm tra…'}</p>
            )}
          </section>

          {plan && plan.models.length > 0 && (
            <section className="lora-group">
              <h3>Mô tả lấy từ model</h3>
              <div className="lora-bar" aria-hidden>
                {plan.models.map((m, i) => (
                  <span
                    key={m.model_key}
                    style={{ flex: m.count, background: SEGMENT_COLORS[i % SEGMENT_COLORS.length] }}
                  />
                ))}
              </div>
              <ul className="lora-legend">
                {plan.models.map((m, i) => (
                  <li key={m.model_key}>
                    <i style={{ background: SEGMENT_COLORS[i % SEGMENT_COLORS.length] }} aria-hidden />
                    {`${m.name} · ${m.count} bài`}
                  </li>
                ))}
              </ul>
            </section>
          )}

          {plan && (plan.missing_caption.length > 0 || plan.voice_unconfirmed.length > 0) && (
            <section className="lora-group">
              <h3>Cần xử lý trước khi train</h3>
              <ul className="lora-blocks">
                {plan.missing_caption.map((t) => (
                  <li key={`m-${t.id}`}>
                    {trackLine({ ...t, vocals: '' })}
                    <span className="lora-block-note">Chưa có mô tả</span>
                    <button
                      type="button"
                      className="btn btn-sm"
                      disabled={!captionModelKey || captionJob.isPending}
                      onClick={() => captionJob.mutate({ model_key: captionModelKey, track_ids: [t.id] })}
                    >
                      Tạo mô tả
                    </button>
                    <button type="button" className="btn btn-sm" onClick={() => exclude(t.id)}>
                      Bỏ bài này
                    </button>
                  </li>
                ))}
                {plan.voice_unconfirmed.map((t) => (
                  <li key={`v-${t.id}`}>
                    {trackLine({ ...t, vocals: t.vocals || t.caption })}
                    <button type="button" className="btn btn-sm" onClick={() => exclude(t.id)}>
                      Bỏ bài này
                    </button>
                    <button type="button" className="btn btn-sm" onClick={() => confirmVoice(t.id)}>
                      Xác nhận không có giọng
                    </button>
                  </li>
                ))}
              </ul>
            </section>
          )}

          {plan && (
            <section className="lora-group">
              <h3>Dự kiến</h3>
              <dl className="lora-estimate">
                <Row label="Bước train">{`${plan.steps_per_epoch} bước/epoch × ${state.epochs} epoch = ${plan.total_steps} bước`}</Row>
                <Row label="Bản lưu">{`${plan.checkpoint_epochs.length} bản (epoch ${plan.checkpoint_epochs.join(', ')})`}</Row>
                <Row label="Nhạc nghe thử">{`${Math.max(plan.eval_renders - 1, 0)} bài LoRA + 1 bài gốc`}</Row>
                <Row label="Thời gian">{minutes === null ? '— (đo sau lần train đầu)' : `~${minutes} phút`}</Row>
              </dl>
            </section>
          )}

          {plan && (plan.errors.length > 0 || plan.warnings.length > 0) && (
            <section className="lora-group">
              <h3>Kiểm tra trước khi train</h3>
              {plan.errors.length > 0 && (
                <Notice tone="danger">
                  <ul className="lora-list">
                    {plan.errors.map((e) => (
                      <li key={e}>{e}</li>
                    ))}
                  </ul>
                </Notice>
              )}
              {plan.warnings.length > 0 && (
                <Notice tone="warn">
                  <ul className="lora-list">
                    {plan.warnings.map((w) => (
                      <li key={w}>{w}</li>
                    ))}
                  </ul>
                </Notice>
              )}
            </section>
          )}

          {problem && <Notice tone="danger">{problem}</Notice>}
          {createLora.isError && <Notice tone="danger">{errorMessage(createLora.error)}</Notice>}
          <p className="field-hint">
            Bấm “Bắt đầu train” là chạy GPU thật. Có thể hủy giữa chừng; các bản đã lưu vẫn giữ.
          </p>
        </div>
      </div>
    </Modal>
  )
}

function Row({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div>
      <dt>{label}</dt>
      <dd>{children}</dd>
    </div>
  )
}
