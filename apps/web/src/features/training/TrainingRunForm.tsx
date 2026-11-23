import { useEffect, useMemo, useState } from 'react'
import type { FormEvent } from 'react'
import { useMutation, useQueryClient } from '@tanstack/react-query'
import { TriangleAlert } from 'lucide-react'
import { api } from '../../api/api'
import { errorMessage } from '../../api/client'
import { useDatasets, useModels } from '../../api/hooks'
import type { TrainingRun } from '../../api/types'
import { Modal } from '../../components/Modal'
import { useToast } from '../../components/Toast'
import { ErrorState, Field, LoadingState, Notice } from '../../components/ui'
import {
  buildTrainingRequest,
  datasetLabel,
  datasetMinutes,
  datasetSamples,
  defaultTrainingFormState,
  stepPlanFromForm,
  trainableDatasets,
  validateTrainingForm,
} from './trainingForm'
import type { TrainingFormState } from './trainingForm'

/** Engine whose models the trainer can use as a base. */
const TRAINABLE_ENGINE = 'acestep'
const FORM_ID = 'training-run-form'

export function TrainingRunForm({
  onClose,
  onCreated,
}: {
  onClose: () => void
  onCreated: (run: TrainingRun) => void
}) {
  const queryClient = useQueryClient()
  const toast = useToast()
  const datasetsQuery = useDatasets()
  const modelsQuery = useModels()
  const [state, setState] = useState<TrainingFormState>(defaultTrainingFormState)
  const [formError, setFormError] = useState<string | null>(null)

  const set = <K extends keyof TrainingFormState>(key: K, value: TrainingFormState[K]) =>
    setState((s) => ({ ...s, [key]: value }))

  const allDatasets = datasetsQuery.data
  const rawModels = modelsQuery.data
  const allModels = useMemo(() => rawModels?.filter((m) => m.kind === 'generator'), [rawModels])

  // Preselect the first usable dataset / base model once they are known.
  useEffect(() => {
    if (!allDatasets) return
    const options = trainableDatasets(allDatasets)
    setState((s) =>
      options.some((d) => d.key === s.datasetKey) ? s : { ...s, datasetKey: options[0]?.key ?? '' },
    )
  }, [allDatasets])
  useEffect(() => {
    if (!allModels) return
    const options = allModels.filter((m) => m.engine === TRAINABLE_ENGINE)
    setState((s) =>
      {
        if (options.some((m) => m.id === s.baseModelId)) return s
        const next = options[0]?.id ?? ''
        return next === s.baseModelId ? s : { ...s, baseModelId: next }
      },
    )
  }, [allModels])

  const create = useMutation({
    mutationFn: api.trainingRuns.create,
    onSuccess: (run) => {
      void queryClient.invalidateQueries({ queryKey: ['training-runs'] })
      void queryClient.invalidateQueries({ queryKey: ['queue-summary'] })
      toast.success('Đã đưa training run vào hàng đợi.')
      onCreated(run)
    },
    onError: (error) => setFormError(errorMessage(error)),
  })

  const onSubmit = (e: FormEvent) => {
    e.preventDefault()
    if (create.isPending) return
    const problem = validateTrainingForm(state)
    setFormError(problem)
    if (problem) return
    create.mutate(buildTrainingRequest(state))
  }

  const datasets = trainableDatasets(allDatasets ?? [])
  const models = (allModels ?? []).filter((m) => m.engine === TRAINABLE_ENGINE)
  const selectedDataset = datasets.find((d) => d.key === state.datasetKey)
  const samples = datasetSamples(selectedDataset)
  const minutes = datasetMinutes(selectedDataset)
  const plan = stepPlanFromForm(state, samples)
  const loading = datasetsQuery.isPending || modelsQuery.isPending

  const numberField = (
    label: string,
    key: keyof TrainingFormState,
    props: { min?: number; max?: number; step?: number | 'any'; placeholder?: string; hint?: string } = {},
  ) => (
    <Field label={label} hint={props.hint}>
      <input
        type="number"
        inputMode="decimal"
        min={props.min}
        max={props.max}
        step={props.step ?? 1}
        placeholder={props.placeholder}
        value={state[key]}
        onChange={(e) => set(key, e.target.value)}
      />
    </Field>
  )

  return (
    <Modal
      title="Tạo training run"
      size="lg"
      onClose={onClose}
      footer={
        <>
          <button type="button" className="btn" onClick={onClose}>
            Hủy bỏ
          </button>
          <button
            type="submit"
            form={FORM_ID}
            className="btn btn-primary"
            disabled={loading || create.isPending || datasets.length === 0 || models.length === 0}
          >
            Bắt đầu train
          </button>
        </>
      }
    >
      {loading ? (
        <LoadingState />
      ) : datasetsQuery.isError ? (
        <ErrorState error={datasetsQuery.error} onRetry={() => void datasetsQuery.refetch()} />
      ) : modelsQuery.isError ? (
        <ErrorState error={modelsQuery.error} onRetry={() => void modelsQuery.refetch()} />
      ) : (
        <form id={FORM_ID} className="form-stack" onSubmit={onSubmit} noValidate aria-label="Tạo training run">
          <Field label="Tên Train Style">
            <input type="text" required value={state.name} onChange={(e) => set('name', e.target.value)} />
          </Field>

          {datasets.length === 0 ? (
            <Notice tone="info">
              <span>
                Chưa có dataset nào train được. Dataset hiện là các <strong>thư mục trên Render Node</strong>, do node
                tự báo về; hãy đặt dataset vào thư mục dataset của node và chờ node báo lại. Trang quản lý Dataset
                trên web sẽ có sau.
              </span>
            </Notice>
          ) : (
            <Field
              label="Dataset"
              hint={
                selectedDataset
                  ? `Node ${selectedDataset.node_name} · ${samples !== null ? `${samples} mẫu` : 'chưa rõ số mẫu'} · ${
                      minutes !== null ? `tổng ${minutes} phút` : 'chưa rõ tổng thời lượng'
                    }`
                  : undefined
              }
            >
              <select value={state.datasetKey} onChange={(e) => set('datasetKey', e.target.value)}>
                {datasets.map((d) => (
                  <option key={d.key} value={d.key}>
                    {datasetLabel(d)}
                  </option>
                ))}
              </select>
            </Field>
          )}

          <Field
            label="Base model"
            hint={
              models.length === 0
                ? `Chưa có model nào dùng engine ${TRAINABLE_ENGINE}; trainer chỉ hỗ trợ engine này.`
                : 'Train Style chỉ dùng được với đúng base model này khi tạo nhạc.'
            }
          >
            <select
              value={state.baseModelId}
              disabled={models.length === 0}
              onChange={(e) => set('baseModelId', e.target.value)}
            >
              {models.length === 0 && <option value="">— Không có model phù hợp —</option>}
              {models.map((m) => (
                <option key={m.id} value={m.id}>
                  {m.name}
                  {m.version ? ` (${m.version})` : ''}
                </option>
              ))}
            </select>
          </Field>

          <p className="field-hint">
            Các giá trị điền sẵn là <strong>cấu hình đã chạy thử kỹ thuật, không phải khuyến nghị chất lượng</strong>{' '}
            — đây là cấu hình duy nhất từng chạy thật trên GPU này.
          </p>
          <div className="field-row cols-3">
            {numberField('Rank', 'rank', { min: 1, max: 128 })}
            {numberField('Alpha', 'alpha', { min: 1, max: 256 })}
            {numberField('Learning rate', 'learningRate', { min: 0, max: 0.01, step: 'any' })}
          </div>
          <div className="field-row cols-3">
            {numberField('Epochs', 'epochs', { min: 1, max: 1000 })}
            {numberField('Gradient accumulation', 'gradientAccumulation', { min: 1, max: 32 })}
            {numberField('Save every N epochs', 'saveEvery', {
              min: 1,
              hint: 'Checkpoint chỉ lưu theo epoch, không theo step.',
            })}
          </div>
          <div className="field-row cols-3">
            {numberField('Warmup steps', 'warmupSteps', { min: 0, max: 10000 })}
            {numberField('Seed', 'seed', { min: 0, placeholder: 'Ngẫu nhiên', hint: 'Không bắt buộc.' })}
            {numberField('Giới hạn thời gian (phút)', 'maxMinutes', {
              min: 1,
              max: 1440,
              hint: 'Hết giờ thì run dừng và giữ checkpoint cuối đã lưu.',
            })}
          </div>

          <section className="explainer" aria-label="Cách tính số bước">
            <h3>Cách tính số bước</h3>
            {plan ? (
              <dl className="meta-grid compact">
                <div className="meta-item">
                  <dt>Số mẫu</dt>
                  <dd>{plan.samples}</dd>
                </div>
                <div className="meta-item">
                  <dt>Optimizer steps / epoch</dt>
                  <dd>{plan.stepsPerEpoch}</dd>
                </div>
                <div className="meta-item">
                  <dt>Tổng optimizer steps</dt>
                  <dd>{plan.totalSteps}</dd>
                </div>
              </dl>
            ) : (
              <p className="muted">
                {samples === null
                  ? 'Node chưa báo số mẫu của dataset này nên chưa tính được số bước.'
                  : 'Nhập Epochs và Gradient accumulation hợp lệ để tính số bước.'}
              </p>
            )}
            <p className="field-hint">
              1 epoch = 1 lượt qua tất cả mẫu. Optimizer steps / epoch = ceil(số mẫu / gradient accumulation); tổng =
              số đó × epochs. “Step” mà trainer báo về là <strong>optimizer step</strong>, không phải micro-batch.
            </p>
            <p className="field-hint">Validation loss và early stopping: Trainer không hỗ trợ.</p>
          </section>

          <Notice tone="warn">
            <TriangleAlert size={16} aria-hidden />
            <span>
              <strong>Trong lúc training run đang chạy, website không tạo nhạc được.</strong> Các job tạo nhạc mới sẽ
              nằm chờ trong hàng đợi. Job tạo nhạc đang chờ luôn được chạy trước một training run mới. Không có mốc
              thời gian cam kết cho việc tạo nhạc hoạt động trở lại.
            </span>
          </Notice>

          {formError && (
            <Notice tone="danger">
              <span>{formError}</span>
            </Notice>
          )}
        </form>
      )}
    </Modal>
  )
}
