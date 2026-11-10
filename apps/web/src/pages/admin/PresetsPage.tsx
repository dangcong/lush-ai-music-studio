import { useState } from 'react'
import type { FormEvent } from 'react'
import { Pencil, Plus, SlidersHorizontal } from 'lucide-react'
import { api } from '../../api/api'
import { useGenres, useLoras, useModels, usePresets } from '../../api/hooks'
import type { Genre, Lora, Model, Preset, PresetInput, PresetKind } from '../../api/types'
import { ChipSelect } from '../../components/ChipSelect'
import { Modal } from '../../components/Modal'
import {
  Badge,
  EmptyState,
  ErrorState,
  Field,
  LoadingState,
  Notice,
  PageHeader,
  Switch,
} from '../../components/ui'
import { sortGenres, subgenresOf, topLevelGenres } from '../../features/create/formState'
import { numberOrNull, numberToText, useAdminMutation } from './shared'

function GenreOptions({ genres }: { genres: Genre[] }) {
  return (
    <>
      {topLevelGenres(genres).map((g) => (
        <optgroup key={g.id} label={g.name}>
          <option value={g.id}>{g.name}</option>
          {subgenresOf(genres, g.id).map((sub) => (
            <option key={sub.id} value={sub.id}>
              {g.name} › {sub.name}
            </option>
          ))}
        </optgroup>
      ))}
    </>
  )
}

function PresetFormModal({
  preset,
  genres,
  models,
  loras,
  defaultGenreId,
  onClose,
}: {
  preset: Preset | null
  genres: Genre[]
  models: Model[]
  loras: Lora[]
  defaultGenreId: string
  onClose: () => void
}) {
  const [name, setName] = useState(preset?.name ?? '')
  const [genreId, setGenreId] = useState(preset?.genre_id ?? defaultGenreId)
  const [description, setDescription] = useState(preset?.description ?? '')
  const [kind, setKind] = useState<PresetKind>(preset?.kind ?? 'genre')
  const [promptTemplate, setPromptTemplate] = useState(preset?.prompt_template ?? '')
  const [instruments, setInstruments] = useState(preset?.instruments ?? [])
  const [moods, setMoods] = useState(preset?.moods ?? [])
  const [bpm, setBpm] = useState(numberToText(preset?.bpm))
  const [duration, setDuration] = useState(numberToText(preset?.duration))
  const [modelId, setModelId] = useState(preset?.model_id ?? '')
  const [loraId, setLoraId] = useState(preset?.lora_id ?? '')
  const [loraScale, setLoraScale] = useState(numberToText(preset?.lora_scale))
  const [inferenceSteps, setInferenceSteps] = useState(numberToText(preset?.params.inference_steps))
  const [guidanceScale, setGuidanceScale] = useState(numberToText(preset?.params.guidance_scale))
  const [error, setError] = useState<string | null>(null)

  const loraOptions = modelId ? loras.filter((l) => l.base_model_id === modelId) : loras

  const save = useAdminMutation(
    (input: PresetInput) => (preset ? api.presets.update(preset.id, input) : api.presets.create(input)),
    {
      invalidate: ['presets'],
      successMessage: preset ? 'Đã lưu preset.' : 'Đã thêm preset.',
      onSuccess: onClose,
    },
  )

  const onSubmit = (e: FormEvent) => {
    e.preventDefault()
    if (kind === 'trained' && !loraId) {
      setError('Preset loại "trained" bắt buộc phải chọn LoRA.')
      return
    }
    setError(null)
    save.mutate({
      name: name.trim(),
      genre_id: genreId,
      description: description.trim(),
      kind,
      prompt_template: promptTemplate.trim(),
      instruments,
      moods,
      bpm: numberOrNull(bpm),
      duration: numberOrNull(duration),
      model_id: modelId || null,
      lora_id: loraId || null,
      lora_scale: loraId ? numberOrNull(loraScale) : null,
      // Keep any other keys already stored in params.
      params: {
        ...(preset?.params ?? {}),
        inference_steps: numberOrNull(inferenceSteps),
        guidance_scale: numberOrNull(guidanceScale),
      },
    })
  }

  return (
    <Modal
      title={preset ? `Sửa preset: ${preset.name}` : 'Thêm preset'}
      size="lg"
      onClose={onClose}
      footer={
        <>
          <button type="button" className="btn" onClick={onClose}>
            Hủy bỏ
          </button>
          <button type="submit" form="preset-form" className="btn btn-primary" disabled={save.isPending}>
            Lưu
          </button>
        </>
      }
    >
      <form id="preset-form" className="form-stack" onSubmit={onSubmit}>
        <div className="field-row">
          <Field label="Name">
            <input type="text" required value={name} onChange={(e) => setName(e.target.value)} />
          </Field>
          <Field label="Genre">
            <select required value={genreId} onChange={(e) => setGenreId(e.target.value)}>
              <option value="" disabled>
                — Chọn genre —
              </option>
              <GenreOptions genres={genres} />
            </select>
          </Field>
        </div>
        <Field label="Description">
          <textarea rows={2} value={description} onChange={(e) => setDescription(e.target.value)} />
        </Field>
        <Field
          label="Prompt Template"
          hint="Dùng {prompt} để chèn prompt của người dùng. Template của preset được ưu tiên hơn subgenre và genre."
        >
          <textarea rows={3} value={promptTemplate} onChange={(e) => setPromptTemplate(e.target.value)} />
        </Field>
        <ChipSelect label="Instruments" value={instruments} onChange={setInstruments} placeholder="Thêm nhạc cụ…" />
        <ChipSelect label="Moods" value={moods} onChange={setMoods} placeholder="Thêm mood…" />
        <div className="field-row cols-3">
          <Field label="Kind" hint='"trained" = preset dựa trên LoRA đã huấn luyện.'>
            <select value={kind} onChange={(e) => setKind(e.target.value as PresetKind)}>
              <option value="genre">genre</option>
              <option value="trained">trained</option>
            </select>
          </Field>
          <Field label="BPM">
            <input type="number" min={1} value={bpm} onChange={(e) => setBpm(e.target.value)} />
          </Field>
          <Field label="Duration (giây)">
            <input type="number" min={10} max={240} value={duration} onChange={(e) => setDuration(e.target.value)} />
          </Field>
        </div>
        <div className="field-row cols-3">
          <Field label="Model">
            <select
              value={modelId}
              onChange={(e) => {
                setModelId(e.target.value)
                if (!loras.some((l) => l.id === loraId && l.base_model_id === e.target.value)) setLoraId('')
              }}
            >
              <option value="">— Không chỉ định —</option>
              {models.map((m) => (
                <option key={m.id} value={m.id}>
                  {m.name}
                  {m.status === 'disabled' ? ' (đang tắt)' : ''}
                </option>
              ))}
            </select>
          </Field>
          <Field label="LoRA" hint={loras.length === 0 ? 'Chưa có LoRA nào (Phase 3).' : undefined}>
            <select value={loraId} disabled={loraOptions.length === 0} onChange={(e) => setLoraId(e.target.value)}>
              <option value="">— Không dùng —</option>
              {loraOptions.map((l) => (
                <option key={l.id} value={l.id}>
                  {l.name}
                </option>
              ))}
            </select>
          </Field>
          <Field label="LoRA Scale">
            <input
              type="number"
              min={0}
              max={2}
              step={0.05}
              disabled={!loraId}
              value={loraScale}
              onChange={(e) => setLoraScale(e.target.value)}
            />
          </Field>
        </div>
        <div className="field-row">
          <Field label="Inference Steps">
            <input
              type="number"
              min={1}
              step={1}
              placeholder="Mặc định"
              value={inferenceSteps}
              onChange={(e) => setInferenceSteps(e.target.value)}
            />
          </Field>
          <Field label="Guidance Scale">
            <input
              type="number"
              min={0}
              step={0.1}
              placeholder="Mặc định"
              value={guidanceScale}
              onChange={(e) => setGuidanceScale(e.target.value)}
            />
          </Field>
        </div>
        {error && (
          <Notice tone="danger">
            <span>{error}</span>
          </Notice>
        )}
      </form>
    </Modal>
  )
}

function PresetRow({
  preset,
  models,
  onEdit,
}: {
  preset: Preset
  models: Model[]
  onEdit: (preset: Preset) => void
}) {
  const toggle = useAdminMutation((is_active: boolean) => api.presets.update(preset.id, { is_active }), {
    invalidate: ['presets'],
  })
  const model = models.find((m) => m.id === preset.model_id)
  return (
    <tr className={preset.is_active ? undefined : 'is-inactive'}>
      <td className="cell-title">
        <span>{preset.name}</span>
        {preset.description && <small>{preset.description}</small>}
      </td>
      <td>
        <Badge tone={preset.kind === 'trained' ? 'accent' : 'neutral'}>{preset.kind}</Badge>
      </td>
      <td>{model?.name ?? '—'}</td>
      <td>{preset.duration ? `${preset.duration}s` : '—'}</td>
      <td>{preset.bpm ?? '—'}</td>
      <td>v{preset.version}</td>
      <td className="cell-actions">
        <button
          type="button"
          className="icon-btn"
          aria-label={`Sửa ${preset.name}`}
          title="Sửa"
          onClick={() => onEdit(preset)}
        >
          <Pencil size={16} aria-hidden />
        </button>
        <Switch
          label={`Bật ${preset.name}`}
          hideLabel
          checked={preset.is_active}
          disabled={toggle.isPending}
          onChange={(v) => toggle.mutate(v)}
        />
      </td>
    </tr>
  )
}

export function PresetsPage() {
  const presetsQuery = usePresets(true)
  const genresQuery = useGenres(true)
  const modelsQuery = useModels()
  const lorasQuery = useLoras()
  const [genreFilter, setGenreFilter] = useState('')
  const [editing, setEditing] = useState<Preset | 'new' | null>(null)

  const genres = genresQuery.data ?? []
  const models = modelsQuery.data ?? []
  const presets = presetsQuery.data ?? []

  const genreLabel = (id: string | null): string => {
    const genre = genres.find((g) => g.id === id)
    if (!genre) return 'Không gắn genre'
    const parent = genres.find((g) => g.id === genre.parent_id)
    return parent ? `${parent.name} › ${genre.name}` : genre.name
  }

  // Group presets by genre, in genre order (parents before their subgenres).
  const orderedGenreIds: (string | null)[] = []
  for (const top of topLevelGenres(genres)) {
    orderedGenreIds.push(top.id, ...sortGenres(subgenresOf(genres, top.id)).map((g) => g.id))
  }
  for (const p of presets) {
    if (!orderedGenreIds.includes(p.genre_id)) orderedGenreIds.push(p.genre_id)
  }
  const groups = orderedGenreIds
    .filter((id) => !genreFilter || id === genreFilter)
    .map((id) => ({ id, presets: presets.filter((p) => p.genre_id === id) }))
    .filter((group) => group.presets.length > 0)

  return (
    <div className="page">
      <PageHeader
        title="Style Presets"
        description="Bộ thiết lập sẵn theo genre: template, nhạc cụ, mood, model và tham số."
        actions={
          <button type="button" className="btn btn-primary" onClick={() => setEditing('new')}>
            <Plus size={16} aria-hidden />
            Thêm preset
          </button>
        }
      />

      <div className="toolbar">
        <select aria-label="Lọc theo Genre" value={genreFilter} onChange={(e) => setGenreFilter(e.target.value)}>
          <option value="">Mọi Genre</option>
          <GenreOptions genres={genres} />
        </select>
      </div>

      {presetsQuery.isPending || genresQuery.isPending ? (
        <LoadingState />
      ) : presetsQuery.isError ? (
        <ErrorState error={presetsQuery.error} onRetry={() => void presetsQuery.refetch()} />
      ) : groups.length === 0 ? (
        <EmptyState icon={<SlidersHorizontal size={26} aria-hidden />} title="Chưa có preset nào">
          {genreFilter ? 'Genre này chưa có preset.' : 'Thêm preset để nhân viên tạo nhạc nhanh hơn.'}
        </EmptyState>
      ) : (
        groups.map((group) => (
          <section key={group.id ?? 'none'} className="group">
            <h2 className="section-title">{genreLabel(group.id)}</h2>
            <div className="table-wrap">
              <table className="table">
                <thead>
                  <tr>
                    <th scope="col">Preset</th>
                    <th scope="col">Kind</th>
                    <th scope="col">Model</th>
                    <th scope="col">Duration</th>
                    <th scope="col">BPM</th>
                    <th scope="col">Version</th>
                    <th scope="col">
                      <span className="sr-only">Thao tác</span>
                    </th>
                  </tr>
                </thead>
                <tbody>
                  {group.presets.map((preset) => (
                    <PresetRow key={preset.id} preset={preset} models={models} onEdit={setEditing} />
                  ))}
                </tbody>
              </table>
            </div>
          </section>
        ))
      )}

      {editing && (
        <PresetFormModal
          preset={editing === 'new' ? null : editing}
          genres={genres}
          models={models}
          loras={lorasQuery.data ?? []}
          defaultGenreId={genreFilter}
          onClose={() => setEditing(null)}
        />
      )}
    </div>
  )
}
