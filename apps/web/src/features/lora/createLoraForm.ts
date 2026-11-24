import type { Job, LoraBuildRequest, TrainingRun } from '../../api/types'
import { integerProblem, parseNumber } from '../training/trainingForm'

export type TrainKind = 'quick' | 'standard' | 'thorough' | 'custom'

export const TRAIN_KIND_EPOCHS: Record<Exclude<TrainKind, 'custom'>, number> = {
  quick: 1,
  standard: 4,
  thorough: 8,
}

export const CREATE_LORA_DEFAULTS = {
  rank: 32,
  alpha: 64,
  learning_rate: 0.0001,
  epochs: 4,
  gradient_accumulation: 4,
  save_every: 1,
  warmup_steps: 2,
  max_minutes: 30,
  holdout_count: 2,
} as const

/** Numeric inputs are strings so they can be empty while typing. */
export interface CreateLoraState {
  name: string
  keyword: string
  testPrompt: string
  testPromptEdited: boolean
  kind: TrainKind
  epochs: string
  saveEvery: string
  holdoutCount: string
  rank: string
  alpha: string
  learningRate: string
  gradientAccumulation: string
  warmupSteps: string
  seed: string
  maxMinutes: string
  baseModelId: string
  holdoutSeed: number
  voiceConfirmed: string[]
  excluded: string[]
}

export function initialCreateLoraState(
  baseModelId: string,
  genreName: string,
  random: () => number = Math.random,
): CreateLoraState {
  const d = CREATE_LORA_DEFAULTS
  return {
    name: '',
    keyword: '',
    testPrompt: defaultTestPrompt('', genreName),
    testPromptEdited: false,
    kind: 'standard',
    epochs: String(d.epochs),
    saveEvery: String(d.save_every),
    holdoutCount: String(d.holdout_count),
    rank: String(d.rank),
    alpha: String(d.alpha),
    learningRate: String(d.learning_rate),
    gradientAccumulation: String(d.gradient_accumulation),
    warmupSteps: String(d.warmup_steps),
    seed: '',
    maxMinutes: String(d.max_minutes),
    baseModelId,
    holdoutSeed: Math.floor(random() * 2 ** 31),
    voiceConfirmed: [],
    excluded: [],
  }
}

export function setKind(s: CreateLoraState, kind: TrainKind): CreateLoraState {
  if (kind === 'custom') return { ...s, kind }
  return { ...s, kind, epochs: String(TRAIN_KIND_EPOCHS[kind]) }
}

export function defaultTestPrompt(keyword: string, genreName: string): string {
  const k = keyword.trim()
  return k ? `${k}, ${genreName}` : genreName
}

export function setKeyword(s: CreateLoraState, keyword: string, genreName: string): CreateLoraState {
  return {
    ...s,
    keyword,
    testPrompt: s.testPromptEdited ? s.testPrompt : defaultTestPrompt(keyword, genreName),
  }
}

/** First numeric problem, before any preview call. Wording follows validateTrainingForm. */
export function localProblem(s: CreateLoraState): string | null {
  if (!s.baseModelId) return 'Hãy chọn Base model.'
  const lr = parseNumber(s.learningRate)
  if (lr === null || lr <= 0 || lr > 0.01) return 'Learning rate phải lớn hơn 0 và không quá 0.01.'
  const epochs = parseNumber(s.epochs)
  const problem =
    integerProblem('Rank', s.rank, 1, 128) ??
    integerProblem('Alpha', s.alpha, 1, 256) ??
    integerProblem('Epochs', s.epochs, 1, 1000) ??
    integerProblem('Gradient accumulation', s.gradientAccumulation, 1, 32) ??
    integerProblem('Save every', s.saveEvery, 1, epochs ?? 1) ??
    integerProblem('Warmup steps', s.warmupSteps, 0, 10000) ??
    integerProblem('Giới hạn thời gian', s.maxMinutes, 1, 1440) ??
    integerProblem('Số bài giữ lại', s.holdoutCount, 0, 5)
  if (problem) return problem
  if (s.seed.trim() !== '') {
    const seed = parseNumber(s.seed)
    if (seed === null || !Number.isInteger(seed) || seed < 0) return 'Seed phải là số nguyên không âm (hoặc để trống).'
    if (seed > 2147483647) return 'Seed không được lớn hơn 2147483647.'
  }
  return null
}

/** Builds the request body from the ticked tracks minus `excluded`; null when `localProblem`. */
export function buildLoraRequest(s: CreateLoraState, trackIds: string[]): LoraBuildRequest | null {
  if (localProblem(s) !== null) return null
  const excluded = new Set(s.excluded)
  return {
    name: s.name.trim(),
    keyword: s.keyword.trim(),
    test_prompt: s.testPrompt.trim(),
    track_ids: trackIds.filter((id) => !excluded.has(id)),
    holdout_count: Number(s.holdoutCount),
    holdout_seed: s.holdoutSeed,
    voice_confirmed: s.voiceConfirmed,
    base_model_id: s.baseModelId,
    rank: Number(s.rank),
    alpha: Number(s.alpha),
    learning_rate: Number(s.learningRate),
    epochs: Number(s.epochs),
    gradient_accumulation: Number(s.gradientAccumulation),
    save_every: Number(s.saveEvery),
    warmup_steps: Number(s.warmupSteps),
    seed: parseNumber(s.seed),
    max_minutes: Number(s.maxMinutes),
  }
}

/**
 * Whole minutes (rounded up) to train `totalSteps`, from the seconds-per-step of the latest
 * completed run that reported both train_seconds and optimizer_steps. null when there is none.
 */
export function estimateMinutes(runs: (Job | TrainingRun)[], totalSteps: number): number | null {
  if (!(totalSteps > 0)) return null
  const usable = runs
    .filter((r) => r.status === 'completed')
    .filter((r) => {
      const t = r.result?.train_seconds
      const n = r.result?.optimizer_steps
      return typeof t === 'number' && t > 0 && typeof n === 'number' && n > 0
    })
    .sort((a, b) => (b.finished_at ?? b.created_at).localeCompare(a.finished_at ?? a.created_at))
  const run = usable[0]
  if (!run) return null
  const perStep = (run.result!.train_seconds as number) / (run.result!.optimizer_steps as number)
  return Math.ceil((perStep * totalSteps) / 60)
}
