import type { GenerationMode, GenerationRequest, Genre, Model, Preset } from '../../api/types'

export const DURATION_MIN = 10
export const DURATION_MAX = 240
export const PROMPT_MAX = 2000
const FALLBACK_DURATION = 120
export const AUDIO_INPUT_MAX_MB = 50
export const AUDIO_INPUT_EXTENSIONS = ['.wav', '.mp3', '.flac']

export const MODE_LABELS: Record<GenerationMode, string> = {
  text: 'Text to Music',
  reference: 'Reference to Music',
  cover: 'Cover / Remix',
}

/** Numeric inputs are kept as strings so they can be empty (= null in the request). */
export interface GenerateFormState {
  /** Optional uploaded Reference Audio. Present = the job runs in mode `reference`, empty = `text`. */
  referenceAudioId: string
  prompt: string
  title: string
  genreId: string
  subgenreId: string
  presetId: string
  modelId: string
  loraId: string
  loraScale: string
  duration: number
  bpm: string
  key: string
  timeSignature: string
  seed: string
  inferenceSteps: string
  guidanceScale: string
  instruments: string[]
  moods: string[]
}

export const emptyFormState: GenerateFormState = {
  referenceAudioId: '',
  prompt: '',
  title: '',
  genreId: '',
  subgenreId: '',
  presetId: '',
  modelId: '',
  loraId: '',
  loraScale: '',
  duration: FALLBACK_DURATION,
  bpm: '',
  key: '',
  timeSignature: '',
  seed: '',
  inferenceSteps: '',
  guidanceScale: '',
  instruments: [],
  moods: [],
}

/** One form, two modes: attaching a Reference Audio is what turns a text job into a reference job. */
export function formMode(state: GenerateFormState): GenerationMode {
  return state.referenceAudioId ? 'reference' : 'text'
}

export function clampDuration(seconds: number): number {
  return Math.min(DURATION_MAX, Math.max(DURATION_MIN, Math.round(seconds)))
}

function numberOrNull(text: string): number | null {
  const trimmed = text.trim()
  if (trimmed === '') return null
  const n = Number(trimmed)
  return Number.isFinite(n) ? n : null
}

function textOrNull(text: string): string | null {
  const trimmed = text.trim()
  return trimmed === '' ? null : trimmed
}

function numberToText(value: unknown): string {
  return typeof value === 'number' && Number.isFinite(value) ? String(value) : ''
}

export function topLevelGenres(genres: Genre[]): Genre[] {
  return sortGenres(genres.filter((g) => g.parent_id === null))
}

export function subgenresOf(genres: Genre[], parentId: string): Genre[] {
  if (!parentId) return []
  return sortGenres(genres.filter((g) => g.parent_id === parentId))
}

export function sortGenres(genres: Genre[]): Genre[] {
  return [...genres].sort((a, b) => a.sort_order - b.sort_order || a.name.localeCompare(b.name))
}

/** Presets offered for the current genre selection (plus presets not bound to a genre). */
export function presetsFor(presets: Preset[], genreId: string, subgenreId: string): Preset[] {
  if (!genreId) return presets
  return presets.filter(
    (p) => p.genre_id === null || p.genre_id === genreId || (subgenreId && p.genre_id === subgenreId),
  )
}

function uniq(values: string[]): string[] {
  const seen = new Set<string>()
  return values.filter((v) => {
    const k = v.trim().toLowerCase()
    if (!k || seen.has(k)) return false
    seen.add(k)
    return true
  })
}

/** Instrument / mood suggestions: the subgenre's first, then its parent genre's. */
export function suggestionsFor(
  genres: Genre[],
  genreId: string,
  subgenreId: string,
): { instruments: string[]; moods: string[] } {
  const genre = genres.find((g) => g.id === genreId)
  const sub = genres.find((g) => g.id === subgenreId)
  return {
    instruments: uniq([...(sub?.instruments ?? []), ...(genre?.instruments ?? [])]),
    moods: uniq([...(sub?.moods ?? []), ...(genre?.moods ?? [])]),
  }
}

/** Selecting a genre resets subgenre, drops a preset of another genre and applies the genre's default duration. */
export function applyGenre(
  state: GenerateFormState,
  genreId: string,
  genres: Genre[],
  presets: Preset[],
): GenerateFormState {
  const genre = genres.find((g) => g.id === genreId)
  const presetStillValid = presetsFor(presets, genreId, '').some((p) => p.id === state.presetId)
  const presetId = presetStillValid ? state.presetId : ''
  return {
    ...state,
    genreId,
    subgenreId: '',
    presetId,
    // A selected preset's duration wins over the genre default.
    duration:
      !presetId && genre?.default_duration ? clampDuration(genre.default_duration) : state.duration,
  }
}

export function applySubgenre(
  state: GenerateFormState,
  subgenreId: string,
  genres: Genre[],
): GenerateFormState {
  const sub = genres.find((g) => g.id === subgenreId)
  return {
    ...state,
    subgenreId,
    duration:
      !state.presetId && sub?.default_duration ? clampDuration(sub.default_duration) : state.duration,
  }
}

/** Selecting a preset pre-fills its values; every field stays editable afterwards. */
export function applyPreset(
  state: GenerateFormState,
  preset: Preset | undefined,
  genres: Genre[],
  models: Model[],
): GenerateFormState {
  if (!preset) return { ...state, presetId: '' }
  const next: GenerateFormState = { ...state, presetId: preset.id }

  if (!state.genreId && preset.genre_id) {
    const presetGenre = genres.find((g) => g.id === preset.genre_id)
    if (presetGenre) {
      next.genreId = presetGenre.parent_id ?? presetGenre.id
      next.subgenreId = presetGenre.parent_id ? presetGenre.id : ''
    }
  }
  if (preset.model_id && models.some((m) => m.id === preset.model_id && m.status === 'active')) {
    next.modelId = preset.model_id
  }
  if (preset.duration) next.duration = clampDuration(preset.duration)
  if (preset.bpm) next.bpm = String(preset.bpm)
  if (preset.instruments.length > 0) next.instruments = [...preset.instruments]
  if (preset.moods.length > 0) next.moods = [...preset.moods]
  next.loraId = preset.lora_id ?? ''
  next.loraScale = preset.lora_id ? numberToText(preset.lora_scale) : ''
  const steps = numberToText(preset.params.inference_steps)
  const guidance = numberToText(preset.params.guidance_scale)
  if (steps) next.inferenceSteps = steps
  if (guidance) next.guidanceScale = guidance
  return next
}

/** Fills the form from an earlier request ("Tạo lại với thiết lập này"). */
export function stateFromRequest(request: Partial<GenerationRequest>): GenerateFormState {
  return {
    // Cover / Remix is not offered in the form; an old cover request is refilled as a plain text one.
    referenceAudioId: request.mode === 'reference' ? (request.reference_audio_id ?? '') : '',
    prompt: request.prompt ?? '',
    title: request.title ?? '',
    genreId: request.genre_id ?? '',
    subgenreId: request.subgenre_id ?? '',
    presetId: request.preset_id ?? '',
    modelId: request.model_id ?? '',
    loraId: request.lora_id ?? '',
    loraScale: numberToText(request.lora_scale),
    duration: clampDuration(request.duration ?? FALLBACK_DURATION),
    bpm: numberToText(request.bpm),
    key: request.key ?? '',
    timeSignature: request.time_signature ?? '',
    seed: numberToText(request.seed),
    inferenceSteps: numberToText(request.params?.inference_steps),
    guidanceScale: numberToText(request.params?.guidance_scale),
    instruments: request.instruments ?? [],
    moods: request.moods ?? [],
  }
}

/** Builds the POST /generations body exactly as described in API_CONTRACTS.md. */
export function buildGenerationRequest(
  state: GenerateFormState,
  idempotencyKey: string,
): GenerationRequest {
  const loraId = textOrNull(state.loraId)
  const params: GenerationRequest['params'] = {
    inference_steps: numberOrNull(state.inferenceSteps),
    guidance_scale: numberOrNull(state.guidanceScale),
  }
  return {
    prompt: state.prompt.trim(),
    title: textOrNull(state.title),
    project_id: null,
    genre_id: textOrNull(state.genreId),
    subgenre_id: textOrNull(state.subgenreId),
    preset_id: textOrNull(state.presetId),
    model_id: textOrNull(state.modelId),
    lora_id: loraId,
    lora_scale: loraId ? numberOrNull(state.loraScale) : null,
    duration: clampDuration(state.duration),
    bpm: numberOrNull(state.bpm),
    key: textOrNull(state.key),
    time_signature: textOrNull(state.timeSignature),
    seed: numberOrNull(state.seed),
    instruments: state.instruments,
    moods: state.moods,
    // The studio only produces instrumental music.
    instrumental: true,
    params,
    mode: formMode(state),
    reference_audio_id: textOrNull(state.referenceAudioId),
    source_audio_id: null,
    idempotency_key: idempotencyKey,
  }
}

export function validate(state: GenerateFormState): string | null {
  const prompt = state.prompt.trim()
  if (prompt.length === 0) return 'Hãy nhập Prompt.'
  if (prompt.length > PROMPT_MAX) return `Prompt tối đa ${PROMPT_MAX} ký tự.`
  if (!state.modelId) return 'Hãy chọn Model.'
  return null
}

export function newIdempotencyKey(): string {
  // randomUUID only exists in secure contexts; the studio may be served over plain HTTP on a LAN.
  if (typeof crypto.randomUUID === 'function') return crypto.randomUUID()
  const bytes = crypto.getRandomValues(new Uint8Array(16))
  bytes[6] = ((bytes[6] ?? 0) & 0x0f) | 0x40
  bytes[8] = ((bytes[8] ?? 0) & 0x3f) | 0x80
  const hex = Array.from(bytes, (b) => b.toString(16).padStart(2, '0')).join('')
  return `${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(12, 16)}-${hex.slice(16, 20)}-${hex.slice(20)}`
}
