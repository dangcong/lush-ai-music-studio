import { useState } from 'react'
import type { FormEvent } from 'react'
import { Palette, Pencil } from 'lucide-react'
import { Link } from 'react-router-dom'
import { api } from '../../api/api'
import { useGenres, useLoras, useModels, usePresets } from '../../api/hooks'
import type { Genre, Lora, Model, Preset, PresetInput } from '../../api/types'
import { ChipSelect } from '../../components/ChipSelect'
import { Modal } from '../../components/Modal'
import { Badge, EmptyState, ErrorState, Field, LoadingState, Notice, PageHeader, Switch } from '../../components/ui'
import { subgenresOf, topLevelGenres } from '../../features/create/formState'
import { numberOrNull, numberToText, useAdminMutation } from './shared'

/**
 * A Style is an approved Train Style plus the settings users generate with. It is stored as a
 * style preset of kind `trained` (the API and database keep the name "preset").
 */
function styleOf(lora: Lora, presets: Preset[]): Preset | undefined {
  return presets.find((p) => p.kind === 'trained' && p.lora_id === lora.id)
}

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

function StyleFormModal({
  lora,
  style,
  genres,
  onClose,
}: {
  lora: Lora
  style: Preset | undefined
  genres: Genre[]
  onClose: () => void
}) {
  const [name, setName] = useState(style?.name ?? lora.name)
  const [genreId, setGenreId] = useState(style?.genre_id ?? lora.genre_id ?? '')
  const [description, setDescription] = useState(style?.description ?? '')
  const [promptTemplate, setPromptTemplate] = useState(style?.prompt_template ?? '')
  const [instruments, setInstruments] = useState(style?.instruments ?? [])
  const [moods, setMoods] = useState(style?.moods ?? [])
  const [bpm, setBpm] = useState(numberToText(style?.bpm))
  const [durationMin, setDurationMin] = useState(numberToText(style?.duration_min))
  const [durationMax, setDurationMax] = useState(numberToText(style?.duration_max))
  const [loraScale, setLoraScale] = useState(style ? numberToText(style.lora_scale) : '1')
  const [inferenceSteps, setInferenceSteps] = useState(numberToText(style?.params.inference_steps))
  const [guidanceScale, setGuidanceScale] = useState(numberToText(style?.params.guidance_scale))

  const save = useAdminMutation(
    (input: PresetInput) => (style ? api.presets.update(style.id, input) : api.presets.create(input)),
    {
      invalidate: ['presets'],
      successMessage: style ? 'Đã lưu Style.' : 'Đã thiết lập Style.',
      onSuccess: onClose,
    },
  )

  const onSubmit = (e: FormEvent) => {
    e.preventDefault()
    save.mutate({
      name: name.trim(),
      genre_id: genreId,
      description: description.trim(),
      kind: 'trained',
      prompt_template: promptTemplate.trim(),
      instruments,
      moods,
      bpm: numberOrNull(bpm),
      duration_min: numberOrNull(durationMin),
      duration_max: numberOrNull(durationMax),
      // A Train Style only works with the base model it was trained on.
      model_id: lora.base_model_id,
      lora_id: lora.id,
      lora_scale: numberOrNull(loraScale),
      // Keep any other keys already stored in params.
      params: {
        ...(style?.params ?? {}),
        inference_steps: numberOrNull(inferenceSteps),
        guidance_scale: numberOrNull(guidanceScale),
      },
    })
  }

  return (
    <Modal
      title={style ? `Sửa Style: ${style.name}` : `Thiết lập Style: ${lora.name}`}
      size="lg"
      onClose={onClose}
      footer={
        <>
          <button type="button" className="btn" onClick={onClose}>
            Hủy bỏ
          </button>
          <button type="submit" form="style-form" className="btn btn-primary" disabled={save.isPending}>
            Lưu
          </button>
        </>
      }
    >
      <form id="style-form" className="form-stack" onSubmit={onSubmit}>
        <div className="field-row">
          <Field label="Name">
            <input type="text" required value={name} onChange={(e) => setName(e.target.value)} />
          </Field>
          <Field label="Genre" hint="Style hiện ở trang Tạo nhạc khi người dùng chọn genre này.">
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
          hint="Dùng {prompt} để chèn prompt của người dùng. Template của Style được ưu tiên hơn subgenre và genre."
        >
          <textarea rows={3} value={promptTemplate} onChange={(e) => setPromptTemplate(e.target.value)} />
        </Field>
        <ChipSelect label="Instruments" value={instruments} onChange={setInstruments} placeholder="Thêm nhạc cụ…" />
        <ChipSelect label="Moods" value={moods} onChange={setMoods} placeholder="Thêm mood…" />
        <div className="field-row cols-4">
          <Field label="Train Style Scale" hint="0–1. Càng cao càng đậm chất Train Style.">
            <input
              type="number"
              min={0}
              max={1}
              step={0.05}
              value={loraScale}
              onChange={(e) => setLoraScale(e.target.value)}
            />
          </Field>
          <Field label="BPM">
            <input type="number" min={1} value={bpm} onChange={(e) => setBpm(e.target.value)} />
          </Field>
          <Field label="Duration từ (giây)">
            <input
              type="number"
              required={durationMax !== ''}
              min={10}
              max={durationMax || 300}
              placeholder="Để trống"
              value={durationMin}
              onChange={(e) => setDurationMin(e.target.value)}
            />
          </Field>
          <Field label="Duration đến (giây)">
            <input
              type="number"
              required={durationMin !== ''}
              min={durationMin || 10}
              max={300}
              placeholder="Để trống"
              value={durationMax}
              onChange={(e) => setDurationMax(e.target.value)}
            />
          </Field>
        </div>
        <p className="field-hint">
          Khoảng Duration của Style (10–300 giây) được ưu tiên hơn subgenre và genre: chọn Style này thì Duration tự nhảy
          về mốc “đến”. Để trống cả hai ô thì dùng khoảng của subgenre, rồi genre.
        </p>
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
      </form>
    </Modal>
  )
}

function StyleRow({
  lora,
  style,
  genres,
  models,
  onEdit,
}: {
  lora: Lora
  style: Preset | undefined
  genres: Genre[]
  models: Model[]
  onEdit: (lora: Lora) => void
}) {
  const toggle = useAdminMutation(
    (is_active: boolean) => (style ? api.presets.update(style.id, { is_active }) : Promise.resolve(undefined)),
    { invalidate: ['presets'] },
  )
  const genre = genres.find((g) => g.id === style?.genre_id)
  const parent = genres.find((g) => g.id === genre?.parent_id)
  const model = models.find((m) => m.id === lora.base_model_id)

  return (
    <tr className={style && !style.is_active ? 'is-inactive' : undefined}>
      <td className="cell-title">
        <span>{style?.name ?? lora.name}</span>
        {style?.description && <small>{style.description}</small>}
      </td>
      <td>{lora.name}</td>
      <td>
        {!style ? (
          <Badge tone="warn">Cần chọn genre</Badge>
        ) : genre ? (
          parent ? (
            `${parent.name} › ${genre.name}`
          ) : (
            genre.name
          )
        ) : (
          '—'
        )}
      </td>
      <td>{model?.name ?? <span className="mono">{lora.base_model_id}</span>}</td>
      <td>{style?.lora_scale ?? '—'}</td>
      <td>
        {style?.duration_min && style.duration_max
          ? `${style.duration_min === style.duration_max ? style.duration_min : `${style.duration_min}–${style.duration_max}`}s`
          : '—'}
      </td>
      <td>{style?.bpm ?? '—'}</td>
      <td className="cell-actions">
        {style ? (
          <>
            <button
              type="button"
              className="icon-btn"
              aria-label={`Sửa ${style.name}`}
              title="Sửa"
              onClick={() => onEdit(lora)}
            >
              <Pencil size={16} aria-hidden />
            </button>
            <Switch
              label={`Bật ${style.name}`}
              hideLabel
              checked={style.is_active}
              disabled={toggle.isPending}
              onChange={(v) => toggle.mutate(v)}
            />
          </>
        ) : (
          <button
            type="button"
            className="btn btn-sm btn-primary"
            aria-label={`Thiết lập ${lora.name}`}
            onClick={() => onEdit(lora)}
          >
            Thiết lập
          </button>
        )}
      </td>
    </tr>
  )
}

export function StylesPage() {
  const lorasQuery = useLoras()
  const presetsQuery = usePresets(true)
  const genresQuery = useGenres(true)
  const models = useModels().data ?? []
  const [editingId, setEditingId] = useState<string | null>(null)

  const genres = genresQuery.data ?? []
  const presets = presetsQuery.data ?? []
  // Only an approved Train Style becomes a Style.
  const approved = (lorasQuery.data ?? []).filter((l) => l.status === 'approved')
  const editing = approved.find((l) => l.id === editingId)
  const needsSetup = approved.some((l) => !styleOf(l, presets))

  return (
    <div className="page">
      <PageHeader
        title="Style"
        description="Các Train Style đã được duyệt. Mỗi Style là một lựa chọn ở trang Tạo nhạc."
      />

      {lorasQuery.isPending || presetsQuery.isPending || genresQuery.isPending ? (
        <LoadingState />
      ) : lorasQuery.isError ? (
        <ErrorState error={lorasQuery.error} onRetry={() => void lorasQuery.refetch()} />
      ) : presetsQuery.isError ? (
        <ErrorState error={presetsQuery.error} onRetry={() => void presetsQuery.refetch()} />
      ) : approved.length === 0 ? (
        <EmptyState icon={<Palette size={26} aria-hidden />} title="Chưa có Style nào">
          Bấm <strong>Duyệt</strong> một Train Style ở <Link to="/lora-library">Train Style Library</Link> để nó xuất
          hiện tại đây.
        </EmptyState>
      ) : (
        <>
          {needsSetup && (
            <Notice tone="warn">
              <span>
                Style ghi <strong>Cần chọn genre</strong> chưa dùng được ở trang Tạo nhạc. Bấm <strong>Thiết lập</strong>{' '}
                và chọn genre cho nó.
              </span>
            </Notice>
          )}
          <div className="table-wrap">
            <table className="table">
              <thead>
                <tr>
                  <th scope="col">Style</th>
                  <th scope="col">Train Style</th>
                  <th scope="col">Genre</th>
                  <th scope="col">Base model</th>
                  <th scope="col">Scale</th>
                  <th scope="col">Duration</th>
                  <th scope="col">BPM</th>
                  <th scope="col">
                    <span className="sr-only">Thao tác</span>
                  </th>
                </tr>
              </thead>
              <tbody>
                {approved.map((lora) => (
                  <StyleRow
                    key={lora.id}
                    lora={lora}
                    style={styleOf(lora, presets)}
                    genres={genres}
                    models={models}
                    onEdit={(l) => setEditingId(l.id)}
                  />
                ))}
              </tbody>
            </table>
          </div>
        </>
      )}

      {editing && (
        <StyleFormModal
          lora={editing}
          style={styleOf(editing, presets)}
          genres={genres}
          onClose={() => setEditingId(null)}
        />
      )}
    </div>
  )
}
