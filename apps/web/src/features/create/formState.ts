import type { GenerationMode, GenerationRequest, Genre, Lora, Model, Preset } from '../../api/types'

export const DURATION_MIN = 10
export const DURATION_MAX = 300
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
  /** "Thinking (LM)": only sent when the model has the `thinking` capability. */
  thinking: boolean
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
  thinking: false,
  instruments: [],
  moods: [],
}

export const DEFAULT_LORA_SCALE = 1

/**
 * LoRAs the user may generate with on the given model: `approved` for everyone,
 * plus `evaluation` for admins (trial generation).
 */
export function usableLoras(loras: Lora[], modelId: string, isAdmin: boolean): Lora[] {
  return loras.filter(
    (l) =>
      l.base_model_id === modelId && (l.status === 'approved' || (isAdmin && l.status === 'evaluation')),
  )
}

/**
 * A Style is a preset backed by a Train Style (LoRA) this user may generate with. Presets without
 * one are no longer offered: the Style page only manages Train Style presets.
 */
export function presetUsable(preset: Preset, loras: Lora[], isAdmin: boolean): boolean {
  if (!preset.lora_id) return false
  const lora = loras.find((l) => l.id === preset.lora_id)
  return lora !== undefined && (lora.status === 'approved' || (isAdmin && lora.status === 'evaluation'))
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

export interface DurationRange {
  min: number
  max: number
  /** Name of the Style / subgenre / genre the range comes from. */
  source: string
}

/**
 * The suggested duration range for the current selection. The most specific one that has a
 * range wins: Style, then subgenre, then genre.
 */
export function durationRangeFor(
  genres: Genre[],
  presets: Preset[],
  selection: Pick<GenerateFormState, 'genreId' | 'subgenreId' | 'presetId'>,
): DurationRange | null {
  const candidates = [
    selection.presetId ? presets.find((p) => p.id === selection.presetId) : undefined,
    selection.subgenreId ? genres.find((g) => g.id === selection.subgenreId) : undefined,
    selection.genreId ? genres.find((g) => g.id === selection.genreId) : undefined,
  ]
  for (const c of candidates) {
    if (c?.duration_min && c.duration_max) return { min: c.duration_min, max: c.duration_max, source: c.name }
  }
  return null
}

/**
 * Every change of genre, subgenre or Style re-applies the suggested range: the duration jumps to
 * its upper end, even over a value the user dragged before. Without a range it stays as it is.
 */
function withSuggestedDuration(state: GenerateFormState, genres: Genre[], presets: Preset[]): GenerateFormState {
  const range = durationRangeFor(genres, presets, state)
  return range ? { ...state, duration: clampDuration(range.max) } : state
}

/** Selecting a genre resets subgenre, drops a preset of another genre and re-applies the duration range. */
export function applyGenre(
  state: GenerateFormState,
  genreId: string,
  genres: Genre[],
  presets: Preset[],
): GenerateFormState {
  const presetStillValid = presetsFor(presets, genreId, '').some((p) => p.id === state.presetId)
  const presetId = presetStillValid ? state.presetId : ''
  return withSuggestedDuration({ ...state, genreId, subgenreId: '', presetId }, genres, presets)
}

export function applySubgenre(
  state: GenerateFormState,
  subgenreId: string,
  genres: Genre[],
  presets: Preset[],
): GenerateFormState {
  return withSuggestedDuration({ ...state, subgenreId }, genres, presets)
}

/** Selecting a preset pre-fills its values; every field stays editable afterwards. */
export function applyPreset(
  state: GenerateFormState,
  preset: Preset | undefined,
  genres: Genre[],
  models: Model[],
): GenerateFormState {
  // Dropping the Style falls back to the subgenre's / genre's range.
  if (!preset) return withSuggestedDuration({ ...state, presetId: '' }, genres, [])
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
  if (preset.bpm) next.bpm = String(preset.bpm)
  if (preset.instruments.length > 0) next.instruments = [...preset.instruments]
  if (preset.moods.length > 0) next.moods = [...preset.moods]
  next.loraId = preset.lora_id ?? ''
  next.loraScale = preset.lora_id ? numberToText(preset.lora_scale) : ''
  const steps = numberToText(preset.params.inference_steps)
  const guidance = numberToText(preset.params.guidance_scale)
  if (steps) next.inferenceSteps = steps
  if (guidance) next.guidanceScale = guidance
  next.thinking = preset.params.thinking === true
  return withSuggestedDuration(next, genres, [preset])
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
    thinking: request.params?.thinking === true,
    instruments: request.instruments ?? [],
    moods: request.moods ?? [],
  }
}

/** Builds the POST /generations body exactly as described in API_CONTRACTS.md. */
export function buildGenerationRequest(
  state: GenerateFormState,
  idempotencyKey: string,
  canThink = false,
): GenerationRequest {
  const loraId = textOrNull(state.loraId)
  const params: GenerationRequest['params'] = {
    inference_steps: numberOrNull(state.inferenceSteps),
    guidance_scale: numberOrNull(state.guidanceScale),
    // Sent (true or false) only when the model has the LM, so it also overrides a Style's value.
    ...(canThink ? { thinking: state.thinking } : {}),
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
    // No scale entered = full strength.
    lora_scale: loraId ? (numberOrNull(state.loraScale) ?? DEFAULT_LORA_SCALE) : null,
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

/**
 * ACE-Step reads only about this many characters of the composed caption (256 tokens with the instruction and
 * the bpm/key/duration lines). The API shortens a longer prompt (`fit_prompt`); keep in step with
 * ACE_CAPTION_CHARS in apps/api/src/routes/jobs.rs.
 */
export const ACE_CAPTION_CHARS = 850

/** Same as `compose_prompt` in the API: template (with an optional `{prompt}` slot), then the tags. */
export function composePrompt(
  prompt: string,
  template: string,
  instruments: string[],
  moods: string[],
  instrumental: boolean,
): string {
  const p = prompt.trim()
  const t = template.trim()
  const parts: string[] = []
  if (t.includes('{prompt}')) {
    const filled = t.replaceAll('{prompt}', p).replace(/^[\s,]+|[\s,]+$/g, '')
    if (filled) parts.push(filled)
  } else {
    parts.push(p)
    if (t) parts.push(t)
  }
  for (const tag of [...instruments, ...moods]) {
    if (tag.trim()) parts.push(tag.trim())
  }
  if (instrumental) parts.push('instrumental, no vocals')
  return parts.join(', ')
}

/** The template the API picks: the Style's, else the subgenre's, else the genre's. */
export function promptTemplate(state: GenerateFormState, genres: Genre[], presets: Preset[]): string {
  const candidates = [
    presets.find((p) => p.id === state.presetId)?.prompt_template,
    genres.find((g) => g.id === state.subgenreId)?.prompt_template,
    genres.find((g) => g.id === state.genreId)?.prompt_template,
  ]
  return candidates.find((t) => t && t.trim() !== '') ?? ''
}
