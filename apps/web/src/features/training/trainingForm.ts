import type { Dataset, TrainingRunInput } from '../../api/types'

/** Numeric inputs are kept as strings so they can be empty while typing. */
export interface TrainingFormState {
  name: string
  datasetKey: string
  baseModelId: string
  rank: string
  alpha: string
  learningRate: string
  epochs: string
  gradientAccumulation: string
  saveEvery: string
  warmupSteps: string
  seed: string
  maxMinutes: string
}

/**
 * The only configuration that has actually been run on this GPU. It is a technical
 * smoke-test configuration, not a quality recommendation.
 */
export const TESTED_CONFIG = {
  rank: 32,
  alpha: 64,
  learning_rate: 0.0001,
  epochs: 4,
  gradient_accumulation: 4,
  save_every: 2,
  warmup_steps: 2,
  max_minutes: 30,
} as const

export const defaultTrainingFormState: TrainingFormState = {
  name: '',
  datasetKey: '',
  baseModelId: '',
  rank: String(TESTED_CONFIG.rank),
  alpha: String(TESTED_CONFIG.alpha),
  learningRate: String(TESTED_CONFIG.learning_rate),
  epochs: String(TESTED_CONFIG.epochs),
  gradientAccumulation: String(TESTED_CONFIG.gradient_accumulation),
  saveEvery: String(TESTED_CONFIG.save_every),
  warmupSteps: String(TESTED_CONFIG.warmup_steps),
  seed: '',
  maxMinutes: String(TESTED_CONFIG.max_minutes),
}

export function parseNumber(text: string): number | null {
  const trimmed = text.trim()
  if (trimmed === '') return null
  const n = Number(trimmed)
  return Number.isFinite(n) ? n : null
}

export function integerProblem(label: string, text: string, min: number, max: number): string | null {
  const n = parseNumber(text)
  if (n === null || !Number.isInteger(n) || n < min || n > max) {
    return `${label} phải là số nguyên từ ${min} đến ${max}.`
  }
  return null
}

/** Mirrors the ranges the API enforces on POST /training-runs. Returns the first problem. */
export function validateTrainingForm(state: TrainingFormState): string | null {
  if (state.name.trim() === '') return 'Hãy nhập Tên Train Style.'
  if (!state.datasetKey) return 'Hãy chọn Dataset.'
  if (!state.baseModelId) return 'Hãy chọn Base model.'

  const lr = parseNumber(state.learningRate)
  if (lr === null || lr <= 0 || lr > 0.01) return 'Learning rate phải lớn hơn 0 và không quá 0.01.'

  const epochs = parseNumber(state.epochs)
  const problem =
    integerProblem('Rank', state.rank, 1, 128) ??
    integerProblem('Alpha', state.alpha, 1, 256) ??
    integerProblem('Epochs', state.epochs, 1, 1000) ??
    integerProblem('Gradient accumulation', state.gradientAccumulation, 1, 32) ??
    integerProblem('Save every', state.saveEvery, 1, epochs ?? 1) ??
    integerProblem('Warmup steps', state.warmupSteps, 0, 10000) ??
    integerProblem('Giới hạn thời gian', state.maxMinutes, 1, 1440)
  if (problem) return problem

  if (state.seed.trim() !== '') {
    const seed = parseNumber(state.seed)
    if (seed === null || !Number.isInteger(seed) || seed < 0) return 'Seed phải là số nguyên không âm (hoặc để trống).'
  }
  return null
}

/** Builds the POST /training-runs body. Call only after `validateTrainingForm` returned null. */
export function buildTrainingRequest(state: TrainingFormState): TrainingRunInput {
  return {
    name: state.name.trim(),
    dataset_key: state.datasetKey,
    base_model_id: state.baseModelId,
    rank: Number(state.rank),
    alpha: Number(state.alpha),
    learning_rate: Number(state.learningRate),
    epochs: Number(state.epochs),
    gradient_accumulation: Number(state.gradientAccumulation),
    save_every: Number(state.saveEvery),
    warmup_steps: Number(state.warmupSteps),
    seed: parseNumber(state.seed),
    max_minutes: Number(state.maxMinutes),
  }
}

export interface StepPlan {
  samples: number
  stepsPerEpoch: number
  totalSteps: number
}

/**
 * Optimizer steps implied by the dataset size: one epoch is one pass over every sample,
 * and the optimizer steps once per `gradient_accumulation` samples.
 * Returns null when any input is unknown or invalid — never a guess.
 */
export function stepPlan(
  samples: number | null | undefined,
  gradientAccumulation: number | null,
  epochs: number | null,
): StepPlan | null {
  if (typeof samples !== 'number' || !Number.isInteger(samples) || samples < 1) return null
  if (gradientAccumulation === null || !Number.isInteger(gradientAccumulation) || gradientAccumulation < 1) return null
  if (epochs === null || !Number.isInteger(epochs) || epochs < 1) return null
  const stepsPerEpoch = Math.ceil(samples / gradientAccumulation)
  return { samples, stepsPerEpoch, totalSteps: stepsPerEpoch * epochs }
}

export function stepPlanFromForm(state: TrainingFormState, samples: number | null | undefined): StepPlan | null {
  return stepPlan(samples, parseNumber(state.gradientAccumulation), parseNumber(state.epochs))
}

/** Datasets a run can be started on, one entry per key (a key may be reported by several nodes). */
export function trainableDatasets(datasets: Dataset[]): Dataset[] {
  const seen = new Set<string>()
  return datasets.filter((d) => {
    if (!d.can_train || seen.has(d.key)) return false
    seen.add(d.key)
    return true
  })
}

export function datasetSamples(dataset: Dataset | undefined): number | null {
  const samples = dataset?.info?.samples
  return typeof samples === 'number' && Number.isFinite(samples) ? samples : null
}

/** Total audio length in minutes, one decimal; null when the node did not report it. */
export function datasetMinutes(dataset: Dataset | undefined): number | null {
  const seconds = dataset?.info?.seconds
  return typeof seconds === 'number' && Number.isFinite(seconds) ? Math.round(seconds / 6) / 10 : null
}

export function datasetLabel(dataset: Dataset): string {
  const name = dataset.info?.name || dataset.key
  const samples = datasetSamples(dataset)
  const minutes = datasetMinutes(dataset)
  const parts = [
    samples !== null ? `${samples} mẫu` : 'chưa rõ số mẫu',
    minutes !== null ? `${minutes} phút` : 'chưa rõ thời lượng',
  ]
  return `${name} — ${parts.join(' · ')}`
}

export const TRAINING_STAGE_LABELS: Record<string, string> = {
  preparing: 'Chuẩn bị',
  stopping_inference: 'Đang dừng engine tạo nhạc',
  preprocessing: 'Tiền xử lý dataset',
  training: 'Đang train',
  saving: 'Đang lưu adapter',
  restoring_inference: 'Đang khôi phục engine tạo nhạc',
}

/** Vietnamese label for a stage; unknown stages are shown as reported. */
export function stageLabel(stage: string | null | undefined): string {
  if (!stage) return '—'
  return TRAINING_STAGE_LABELS[stage] ?? stage
}

/** A run whose worker has not reported for this long is shown as "lost updates". */
export const STALE_AFTER_MS = 90_000

export function isStale(
  status: string,
  updatedAt: string | null | undefined,
  nowMs: number,
): boolean {
  if (status !== 'preparing' && status !== 'running') return false
  if (!updatedAt) return false
  const t = new Date(updatedAt).getTime()
  return Number.isFinite(t) && nowMs - t > STALE_AFTER_MS
}
