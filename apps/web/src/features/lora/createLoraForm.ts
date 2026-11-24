import type { DatasetSnapshot, Job, LoraBuildRequest, TrainingRun } from '../../api/types'
import { integerProblem, parseNumber } from '../training/trainingForm'

export type TrainKind = 'quick' | 'standard' | 'thorough' | 'custom'

/**
 * Defaults from the Side-Step guide bundled with ACE-Step 1.5 (`docs/sidestep/Training Guide.md`,
 * `acestep/training_v2/cli/args.py`) and the latest upstream Side-Step (1.1.2): LoRA rank 64 / alpha 128,
 * AdamW at 1e-4, cosine schedule, gradient accumulation 4. Epochs follow the guide's table by song count.
 */
export const CREATE_LORA_DEFAULTS = {
  rank: 64,
  alpha: 128,
  learning_rate: 0.0001,
  gradient_accumulation: 4,
  max_minutes: 120,
  holdout_count: 2,
} as const

/** Checkpoints a run should leave: the after-train screen listens to the last 10. */
const TARGET_CHECKPOINTS = 10
const MAX_WARMUP_STEPS = 100

/** Side-Step: 1–10 songs 200–500 epochs, 10–50 songs 100–200, 50+ songs 50–100. "Kỹ" = 1.5 × "Chuẩn". */
export function recommendedEpochs(kind: Exclude<TrainKind, 'custom'>, songs: number): number {
  if (kind === 'quick') return 1
  const standard = songs < 10 ? 300 : songs <= 50 ? 150 : 80
  return kind === 'thorough' ? Math.round(standard * 1.5) : standard
}

/** Save every N epochs so a run leaves about TARGET_CHECKPOINTS checkpoints. */
export function autoSaveEvery(epochs: number): number {
  return Math.max(1, Math.round(epochs / TARGET_CHECKPOINTS))
}

/** About 5% of all optimizer steps, at most 100 (Side-Step's own default). */
export function autoWarmup(songs: number, gradientAccumulation: number, epochs: number): number {
  const steps = Math.ceil(songs / Math.max(1, gradientAccumulation)) * epochs
  return Math.min(MAX_WARMUP_STEPS, Math.round(steps * 0.05))
}

/** Numeric inputs are strings so they can be empty while typing. */
export interface CreateLoraState {
  name: string
  keyword: string
  testPrompt: string
  testPromptEdited: boolean
  kind: TrainKind
  epochs: string
  saveEvery: string
  /** Recomputed from the epochs until the user types a value. */
  saveEveryAuto: boolean
  holdoutCount: string
  rank: string
  alpha: string
  learningRate: string
  gradientAccumulation: string
  warmupSteps: string
  /** Recomputed from songs × epochs until the user types a value. */
  warmupAuto: boolean
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
    epochs: String(recommendedEpochs('standard', 10)),
    saveEvery: String(autoSaveEvery(recommendedEpochs('standard', 10))),
    saveEveryAuto: true,
    holdoutCount: String(d.holdout_count),
    rank: String(d.rank),
    alpha: String(d.alpha),
    learningRate: String(d.learning_rate),
    gradientAccumulation: String(d.gradient_accumulation),
    warmupSteps: '0',
    warmupAuto: true,
    seed: '',
    maxMinutes: String(d.max_minutes),
    baseModelId,
    holdoutSeed: Math.floor(random() * 2 ** 31),
    voiceConfirmed: [],
    excluded: [],
  }
}

/** Songs that will be trained: ticked minus excluded minus the held-out ones (at least 1). */
export function trainSongCount(s: CreateLoraState, trackIds: string[]): number {
  const excluded = new Set(s.excluded)
  const ticked = trackIds.filter((id) => !excluded.has(id)).length
  const held = Number(s.holdoutCount)
  return Math.max(1, ticked - (Number.isInteger(held) && held > 0 ? held : 0))
}

/** Fill the automatic fields (epochs for a preset kind, save every, warmup) for the current selection. */
export function applyAuto(s: CreateLoraState, trackIds: string[]): CreateLoraState {
  const songs = trainSongCount(s, trackIds)
  const epochs = s.kind === 'custom' ? s.epochs : String(recommendedEpochs(s.kind, songs))
  const n = parseNumber(epochs)
  const ga = parseNumber(s.gradientAccumulation)
  const valid = n !== null && Number.isInteger(n) && n >= 1
  return {
    ...s,
    epochs,
    saveEvery: s.saveEveryAuto && valid ? String(autoSaveEvery(n)) : s.saveEvery,
    warmupSteps:
      s.warmupAuto && valid && ga !== null && ga >= 1 ? String(autoWarmup(songs, ga, n)) : s.warmupSteps,
  }
}

/** Dialog start state for "train again": the earlier snapshot's options, a new name "<name> (2)". */
export function stateFromSnapshot(snapshot: DatasetSnapshot, genreName: string): CreateLoraState {
  const c = snapshot.content
  const base = initialCreateLoraState(c.base_model.id, genreName)
  return {
    ...base,
    name: `${c.lora.name} (2)`,
    keyword: c.keyword,
    testPrompt: c.test_prompt,
    testPromptEdited: true,
    // The earlier numbers are kept as they were; only a one-epoch run maps back to a preset.
    kind: c.config.epochs === 1 ? 'quick' : 'custom',
    epochs: String(c.config.epochs),
    saveEvery: String(c.config.save_every),
    saveEveryAuto: false,
    holdoutCount: String(c.held_out.length),
    rank: String(c.config.rank),
    alpha: String(c.config.alpha),
    learningRate: String(c.config.learning_rate),
    gradientAccumulation: String(c.config.gradient_accumulation),
    warmupSteps: String(c.config.warmup_steps),
    warmupAuto: false,
    seed: String(c.config.seed),
    maxMinutes: String(Math.max(1, Math.round(c.limits.max_seconds / 60))),
    baseModelId: c.base_model.id,
    holdoutSeed: c.holdout_seed,
  }
}

/** Picking a preset kind lets `applyAuto` fill its epochs; "Tự nhập" keeps the typed value. */
export function setKind(s: CreateLoraState, kind: TrainKind): CreateLoraState {
  return { ...s, kind }
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
