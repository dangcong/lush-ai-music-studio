import { useEffect, useState } from 'react'
import type { FormEvent } from 'react'
import { useMutation, useQueryClient } from '@tanstack/react-query'
import { ChevronDown, ChevronRight, Sparkles, TriangleAlert } from 'lucide-react'
import { api } from '../../api/api'
import { errorMessage } from '../../api/client'
import { useGenres, useLoras, useModels, usePresets } from '../../api/hooks'
import { ChipSelect } from '../../components/ChipSelect'
import { useToast } from '../../components/Toast'
import { ErrorState, Field, LoadingState, Notice, Spinner } from '../../components/ui'
import { formatDuration } from '../../lib/format'
import {
  DURATION_MAX,
  DURATION_MIN,
  PROMPT_MAX,
  applyGenre,
  applyPreset,
  applySubgenre,
  buildGenerationRequest,
  emptyFormState,
  newIdempotencyKey,
  presetsFor,
  subgenresOf,
  suggestionsFor,
  topLevelGenres,
  validate,
} from './formState'
import type { GenerateFormState } from './formState'

function hasAdvancedValues(s: GenerateFormState): boolean {
  return Boolean(
    s.bpm || s.key || s.timeSignature || s.seed || s.inferenceSteps || s.guidanceScale || s.loraId,
  )
}

export function GenerateForm({ initial }: { initial?: GenerateFormState }) {
  const queryClient = useQueryClient()
  const toast = useToast()
  const genresQuery = useGenres()
  const presetsQuery = usePresets()
  const modelsQuery = useModels()
  const lorasQuery = useLoras()

  const [state, setState] = useState<GenerateFormState>(initial ?? emptyFormState)
  const [advancedOpen, setAdvancedOpen] = useState(() => hasAdvancedValues(initial ?? emptyFormState))
  const [formError, setFormError] = useState<string | null>(null)

  const set = <K extends keyof GenerateFormState>(key: K, value: GenerateFormState[K]) =>
    setState((s) => ({ ...s, [key]: value }))

  const genres = genresQuery.data ?? []
  const presets = presetsQuery.data ?? []
  const allModels = modelsQuery.data
  const activeModels = (allModels ?? []).filter((m) => m.status === 'active')
  const loras = lorasQuery.data ?? []

  // Pick a default model once models are known (prefer one that a node can render now).
  useEffect(() => {
    if (!allModels) return
    const active = allModels.filter((m) => m.status === 'active')
    setState((s) => {
      if (active.some((m) => m.id === s.modelId)) return s
      const preferred = active.find((m) => m.installed_nodes.some((n) => n.online)) ?? active[0]
      return { ...s, modelId: preferred?.id ?? '' }
    })
  }, [allModels])

  const generate = useMutation({
    mutationFn: api.generations.create,
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ['jobs'] })
      void queryClient.invalidateQueries({ queryKey: ['queue-summary'] })
      toast.success('Đã gửi yêu cầu tạo nhạc vào hàng đợi.')
    },
    onError: (error) => setFormError(errorMessage(error)),
  })

  const onSubmit = (e: FormEvent) => {
    e.preventDefault()
    if (generate.isPending) return
    const problem = validate(state)
    setFormError(problem)
    if (problem) return
    // A fresh key per click: a retried/double-delivered request can never create two jobs.
    generate.mutate(buildGenerationRequest(state, newIdempotencyKey()))
  }

  if (genresQuery.isPending || modelsQuery.isPending) return <LoadingState label="Đang tải thiết lập…" />
  if (genresQuery.isError) return <ErrorState error={genresQuery.error} onRetry={() => void genresQuery.refetch()} />
  if (modelsQuery.isError) return <ErrorState error={modelsQuery.error} onRetry={() => void modelsQuery.refetch()} />

  const subgenres = subgenresOf(genres, state.genreId)
  const presetOptions = presetsFor(presets, state.genreId, state.subgenreId)
  const suggestions = suggestionsFor(genres, state.genreId, state.subgenreId)
  const selectedGenre = genres.find((g) => g.id === (state.subgenreId || state.genreId))
  const selectedPreset = presets.find((p) => p.id === state.presetId)
  const selectedModel = activeModels.find((m) => m.id === state.modelId)
  const modelOffline = selectedModel !== undefined && !selectedModel.installed_nodes.some((n) => n.online)
  const loraOptions = loras.filter((l) => l.base_model_id === state.modelId)
  const bpmRange =
    selectedGenre?.bpm_min && selectedGenre.bpm_max
      ? `Gợi ý cho ${selectedGenre.name}: ${selectedGenre.bpm_min}–${selectedGenre.bpm_max}.`
      : null

  return (
    <form className="gen-form" onSubmit={onSubmit} noValidate aria-label="Tạo nhạc">
      <Field
        label="Prompt"
        aside={`${state.prompt.length}/${PROMPT_MAX}`}
        hint="Mô tả bản nhạc bạn muốn: nhạc cụ, không khí, nhịp điệu…"
      >
        <textarea
          rows={5}
          maxLength={PROMPT_MAX}
          required
          placeholder="Warm jazz piano, soft brushed drums, rainy night…"
          value={state.prompt}
          onChange={(e) => set('prompt', e.target.value)}
        />
      </Field>

      <Field label="Tiêu đề">
        <input
          type="text"
          placeholder="Không bắt buộc"
          value={state.title}
          onChange={(e) => set('title', e.target.value)}
        />
      </Field>

      <div className="field-row">
        <Field label="Genre">
          <select
            value={state.genreId}
            onChange={(e) => setState((s) => applyGenre(s, e.target.value, genres, presets))}
          >
            <option value="">— Không chọn —</option>
            {topLevelGenres(genres).map((g) => (
              <option key={g.id} value={g.id}>
                {g.name}
              </option>
            ))}
          </select>
        </Field>
        <Field label="Subgenre">
          <select
            value={state.subgenreId}
            disabled={subgenres.length === 0}
            onChange={(e) => setState((s) => applySubgenre(s, e.target.value, genres))}
          >
            <option value="">{subgenres.length === 0 ? '— Không có —' : '— Không chọn —'}</option>
            {subgenres.map((g) => (
              <option key={g.id} value={g.id}>
                {g.name}
              </option>
            ))}
          </select>
        </Field>
      </div>

      <Field label="Style Preset" hint={selectedPreset?.description || undefined}>
        <select
          value={state.presetId}
          onChange={(e) => {
            const preset = presets.find((p) => p.id === e.target.value)
            setState((s) => applyPreset(s, preset, genres, activeModels))
          }}
        >
          <option value="">— Không dùng preset —</option>
          {presetOptions.map((p) => (
            <option key={p.id} value={p.id}>
              {p.name}
              {p.kind === 'trained' ? ' (LoRA)' : ''}
            </option>
          ))}
        </select>
      </Field>

      <Field label="Model">
        <select
          value={state.modelId}
          required
          onChange={(e) =>
            setState((s) => {
              const loraFits = loras.some((l) => l.id === s.loraId && l.base_model_id === e.target.value)
              return {
                ...s,
                modelId: e.target.value,
                loraId: loraFits ? s.loraId : '',
                loraScale: loraFits ? s.loraScale : '',
              }
            })
          }
        >
          {activeModels.length === 0 && <option value="">— Chưa có model nào đang bật —</option>}
          {activeModels.map((m) => (
            <option key={m.id} value={m.id}>
              {m.name}
              {m.version ? ` (${m.version})` : ''}
            </option>
          ))}
        </select>
      </Field>
      {modelOffline && (
        <Notice tone="warn">
          <TriangleAlert size={16} aria-hidden />
          <span>
            Hiện không có Render Node nào online đang cài model này. Bạn vẫn có thể gửi — job sẽ chờ
            trong hàng đợi cho tới khi có node sẵn sàng.
          </span>
        </Notice>
      )}

      <Field label="Duration" aside={`${formatDuration(state.duration)} · ${state.duration}s`}>
        <input
          type="range"
          className="range"
          min={DURATION_MIN}
          max={DURATION_MAX}
          step={5}
          value={state.duration}
          style={{
            ['--fill' as string]: `${((state.duration - DURATION_MIN) / (DURATION_MAX - DURATION_MIN)) * 100}%`,
          }}
          onChange={(e) => set('duration', Number(e.target.value))}
        />
      </Field>

      <ChipSelect
        label="Instruments"
        suggestions={suggestions.instruments}
        value={state.instruments}
        onChange={(v) => set('instruments', v)}
        placeholder="Thêm nhạc cụ…"
      />
      <ChipSelect
        label="Moods"
        suggestions={suggestions.moods}
        value={state.moods}
        onChange={(v) => set('moods', v)}
        placeholder="Thêm mood…"
      />

      <div className="advanced">
        <button
          type="button"
          className="advanced-toggle"
          aria-expanded={advancedOpen}
          aria-controls="gen-advanced"
          onClick={() => setAdvancedOpen((o) => !o)}
        >
          {advancedOpen ? <ChevronDown size={16} aria-hidden /> : <ChevronRight size={16} aria-hidden />}
          Nâng cao
        </button>
        {advancedOpen && (
          <div id="gen-advanced" className="advanced-body">
            <p className="field-hint">
              BPM, Key và Time Signature là <em>hướng dẫn cho model, không bảo đảm chính xác</em>. Seed,
              Inference Steps và Guidance Scale là tham số kỹ thuật được áp dụng đúng như đã nhập.
            </p>
            <div className="field-row cols-3">
              <Field label="BPM" hint={bpmRange}>
                <input
                  type="number"
                  inputMode="numeric"
                  min={20}
                  max={300}
                  placeholder="Tự động"
                  value={state.bpm}
                  onChange={(e) => set('bpm', e.target.value)}
                />
              </Field>
              <Field label="Key">
                <input
                  type="text"
                  placeholder="vd. A minor"
                  value={state.key}
                  onChange={(e) => set('key', e.target.value)}
                />
              </Field>
              <Field label="Time Signature">
                <input
                  type="text"
                  placeholder="vd. 4/4"
                  value={state.timeSignature}
                  onChange={(e) => set('timeSignature', e.target.value)}
                />
              </Field>
            </div>
            <div className="field-row cols-3">
              <Field label="Seed" hint="Để trống: hệ thống tự chọn.">
                <input
                  type="number"
                  inputMode="numeric"
                  min={0}
                  step={1}
                  placeholder="Ngẫu nhiên"
                  value={state.seed}
                  onChange={(e) => set('seed', e.target.value)}
                />
              </Field>
              <Field label="Inference Steps">
                <input
                  type="number"
                  inputMode="numeric"
                  min={1}
                  step={1}
                  placeholder="Mặc định"
                  value={state.inferenceSteps}
                  onChange={(e) => set('inferenceSteps', e.target.value)}
                />
              </Field>
              <Field label="Guidance Scale">
                <input
                  type="number"
                  inputMode="decimal"
                  min={0}
                  step={0.1}
                  placeholder="Mặc định"
                  value={state.guidanceScale}
                  onChange={(e) => set('guidanceScale', e.target.value)}
                />
              </Field>
            </div>
            {loras.length > 0 && (
              <div className="field-row">
                <Field
                  label="LoRA"
                  hint={loraOptions.length === 0 ? 'Không có LoRA nào tương thích với model đang chọn.' : undefined}
                >
                  <select
                    value={state.loraId}
                    disabled={loraOptions.length === 0}
                    onChange={(e) => set('loraId', e.target.value)}
                  >
                    <option value="">— Không dùng LoRA —</option>
                    {loraOptions.map((l) => (
                      <option key={l.id} value={l.id}>
                        {l.name}
                        {l.version ? ` (${l.version})` : ''}
                      </option>
                    ))}
                  </select>
                </Field>
                <Field label="LoRA Scale">
                  <input
                    type="number"
                    inputMode="decimal"
                    min={0}
                    max={2}
                    step={0.05}
                    placeholder="Mặc định"
                    disabled={!state.loraId}
                    value={state.loraScale}
                    onChange={(e) => set('loraScale', e.target.value)}
                  />
                </Field>
              </div>
            )}
          </div>
        )}
      </div>

      {formError && (
        <Notice tone="danger">
          <span>{formError}</span>
        </Notice>
      )}

      <button type="submit" className="btn btn-primary btn-lg gen-submit" disabled={generate.isPending}>
        {generate.isPending ? <Spinner size={18} label="Đang gửi" /> : <Sparkles size={18} aria-hidden />}
        Generate
      </button>
    </form>
  )
}
