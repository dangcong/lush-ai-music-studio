import { useEffect, useMemo, useRef, useState } from 'react'
import type { ReactNode } from 'react'
import { useNavigate } from 'react-router-dom'
import { errorMessage } from '../../api/client'
import { useCatalogTracks, useLoraPreview, useModels, useTrainingRuns } from '../../api/hooks'
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
import { ModelBar } from './ModelBar'

const TRAINABLE_ENGINE = 'acestep'
const DEBOUNCE_MS = 300

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
  catalogName: string
  genreName: string
  trackIds: string[]
  onClose: () => void
  onDeselect: (ids: string[]) => void
  captionModelKey: string
}

export function CreateLoraDialog({ catalogId, catalogName, genreName, trackIds, onClose, onDeselect, captionModelKey }: Props) {
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

  // A requested caption counts as "waiting" only while the track really is being captioned
  // (or until the tracks list has refreshed once after the request).
  const tracksQuery = useCatalogTracks(catalogId)
  const [captionRequested, setCaptionRequested] = useState<Record<string, number>>({})
  const isWaiting = (id: string) => {
    const at = captionRequested[id]
    if (at === undefined) return false
    const track = tracksQuery.data?.find((t) => t.id === id)
    return !!track?.captioning_model_key || tracksQuery.dataUpdatedAt <= at
  }
  const waitingCaption = (plan?.missing_caption ?? []).some((t) => isWaiting(t.id))
  const refetchPreview = preview.refetch
  const refetchTracks = tracksQuery.refetch
  useEffect(() => {
    if (!waitingCaption) return
    const timer = setInterval(() => {
      void refetchPreview()
      void refetchTracks()
    }, 5000)
    return () => clearInterval(timer)
  }, [waitingCaption, refetchPreview, refetchTracks])
  const wasWaiting = useRef(false)
  useEffect(() => {
    // One last look at the plan once the caption job is over (done, failed or cancelled).
    if (wasWaiting.current && !waitingCaption) void refetchPreview()
    wasWaiting.current = waitingCaption
  }, [waitingCaption, refetchPreview])

  const exclude = (id: string) => {
    setRaw((s) => (s.excluded.includes(id) ? s : { ...s, excluded: [...s.excluded, id] }))
    onDeselect([id])
  }
  const confirmVoice = (id: string) =>
    setRaw((s) => (s.voiceConfirmed.includes(id) ? s : { ...s, voiceConfirmed: [...s.voiceConfirmed, id] }))

  const minutes = plan ? estimateMinutes(runsQuery.data ?? [], plan.total_steps) : null
  const stale = preview.isPlaceholderData
  const canStart = problem === null && !!plan && plan.ready && settled && !stale && !createLora.isPending

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

  const selectedCount = trackIds.filter((id) => !raw.excluded.includes(id)).length

  return (
    <Modal
      title={`Tạo LoRA · từ catalog ${catalogName}`}
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
      <p className="lora-sub">
        Mỗi bài dùng mô tả của model đang hiện ở cột Tech stack. Hệ thống chụp lại đúng các mô tả này và lưu kèm LoRA.
      </p>
      <div className="lora-dialog">
        <div className="lora-col">
          <section className="lora-group">
            <h3>
              <span className="lora-num">1</span>Cần điền
            </h3>
            <Field label="Tên LoRA *" hint="Tên hiện trong Style Preset khi tạo nhạc.">
              <input type="text" value={state.name} onChange={(e) => set({ name: e.target.value })} />
            </Field>
            <Field
              label="Từ khóa gọi style"
              aside={<span className="lora-sug">Gợi ý thêm</span>}
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
            <h3>
              <span className="lora-num">2</span>Nên chỉnh
            </h3>
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
              <p className="field-hint">Lần đầu với catalog mới nên chạy “Thử nhanh” để kiểm tra trước khi train lâu.</p>
            </div>
            <Field label="Epoch" hint="Số lượt train qua toàn bộ bài.">
              <input
                type="number"
                min={1}
                value={state.epochs}
                onChange={(e) => setRaw((s) => ({ ...s, kind: 'custom', epochs: e.target.value }))}
              />
            </Field>
            {num('Lưu bản mỗi N epoch', 'saveEvery', { min: 1, hint: 'Mỗi lần lưu là một bản để nghe so sánh, chọn bản hay nhất.' })}
            <div className="field">
              <span className="field-label">
                Bài giữ lại để nghe thử <span className="lora-sug">Gợi ý thêm</span>
              </span>
              {plan && plan.held_out.length > 0 ? (
                <ul className="lora-list">
                  {plan.held_out.map((t) => (
                    <li key={t.id}>{t.filename}</li>
                  ))}
                </ul>
              ) : (
                <p className="field-hint">Chưa có bài nào.</p>
              )}
              <p className="field-hint">Không đưa vào train, dùng để so với nhạc LoRA tạo ra.</p>
            </div>
            <Field
              label="Câu lệnh nghe thử sau khi train"
              aside={<span className="lora-sug">Gợi ý thêm</span>}
              hint="Mỗi bản lưu sẽ tự tạo 1 bài từ câu này (kèm 1 bài không dùng LoRA) để bạn nghe so sánh."
            >
              <textarea
                rows={2}
                value={state.testPrompt}
                onChange={(e) => set({ testPrompt: e.target.value, testPromptEdited: true })}
                placeholder={defaultTestPrompt(state.keyword, genreName)}
              />
            </Field>
          </section>

          <details className="lora-group lora-advanced">
            <summary>
              <span className="lora-num is-grey">3</span>Mặc định · Nâng cao{' '}
              <span className="lora-muted">(thu gọn sẵn, bấm để mở)</span>
            </summary>
            <div className="lora-grid">
              {num('Rank', 'rank', { min: 1, hint: 'Độ “dày” của LoRA.' })}
              {num('Alpha', 'alpha', { min: 1, hint: 'Độ mạnh, thường = 2 × rank.' })}
              {num('Learning rate', 'learningRate', { step: 'any', hint: 'Tốc độ học.' })}
              {num('Gradient accumulation', 'gradientAccumulation', { min: 1, hint: `Gộp ${state.gradientAccumulation || '…'} bài mới cập nhật 1 lần.` })}
              {num('Warmup steps', 'warmupSteps', { min: 0, hint: 'Bước khởi động nhẹ.' })}
              {num('Seed', 'seed', { min: 0, placeholder: 'Ngẫu nhiên', hint: 'Đặt số để train lại ra y hệt.' })}
              {num('Giới hạn thời gian', 'maxMinutes', { min: 1, hint: 'Quá thì dừng, giữ bản đã lưu.' })}
            </div>
            <div className="field">
              <span className="field-label">Độ dài tối đa mỗi bài</span>
              <p>4 phút đầu</p>
              <p className="field-hint">Bài dài hơn bị cắt khi train.</p>
            </div>
            <Field label="Base model" hint="Cố định cho Train Style.">
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
                <div className="lora-tiles">
                  <div>
                    <b>{selectedCount}</b>
                    <span>bài đã chọn</span>
                  </div>
                  <div>
                    <b>{plan.train.length}</b>
                    <span>
                      train · {plan.held_out.length} nghe thử
                    </span>
                  </div>
                  <div>
                    <b>{Math.round(plan.train_seconds / 60)} phút</b>
                    <span>tổng nhạc train</span>
                  </div>
                </div>
                {plan.unknown_duration > 0 && (
                  <p className="field-hint">({plan.unknown_duration} bài chưa rõ thời lượng)</p>
                )}
              </>
            ) : (
              <p className="field-hint">{preview.isError ? 'Chưa kiểm tra được.' : 'Đang kiểm tra…'}</p>
            )}
          </section>

          {plan && plan.models.length > 0 && (
            <section className="lora-group">
              <h3>Mô tả lấy từ model</h3>
              <ModelBar models={plan.models} />
            </section>
          )}

          {plan && (plan.missing_caption.length > 0 || plan.voice_unconfirmed.length > 0) && (
            <section className="lora-group">
              <ul className="lora-blocks">
                {plan.missing_caption.map((t) => (
                  <li key={`m-${t.id}`}>
                    {trackLine({ ...t, vocals: '' })}
                    <span className="lora-block-note">
                      {isWaiting(t.id) ? 'Đang tạo mô tả…' : 'Chưa có mô tả'}
                    </span>
                    <button
                      type="button"
                      className="btn btn-sm"
                      disabled={!captionModelKey || captionJob.isPending || isWaiting(t.id)}
                      onClick={() =>
                        captionJob.mutate(
                          { model_key: captionModelKey, track_ids: [t.id] },
                          { onSuccess: () => setCaptionRequested((r) => ({ ...r, [t.id]: Date.now() })) },
                        )
                      }
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
            <section className={cx('lora-group', stale && 'is-stale')}>
              <h3>Dự kiến</h3>
              <dl className="lora-estimate">
                <Row label="Bước train">{`${plan.steps_per_epoch} bước/epoch × ${debouncedBody?.epochs ?? state.epochs} epoch = ${plan.total_steps} bước`}</Row>
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

          <Notice tone="warn">Trong lúc train, website tạm không tạo nhạc được trên máy rtx6000.</Notice>
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
