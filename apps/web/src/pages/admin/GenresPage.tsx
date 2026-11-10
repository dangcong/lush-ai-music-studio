import { useState } from 'react'
import type { FormEvent } from 'react'
import { CornerDownRight, Pencil, Plus, Tags } from 'lucide-react'
import { api } from '../../api/api'
import { useGenres } from '../../api/hooks'
import type { Genre, GenreInput } from '../../api/types'
import { ChipSelect } from '../../components/ChipSelect'
import { Modal } from '../../components/Modal'
import { EmptyState, ErrorState, Field, LoadingState, PageHeader, Switch } from '../../components/ui'
import { subgenresOf, topLevelGenres } from '../../features/create/formState'
import { cx } from '../../lib/cx'
import { numberOrNull, numberToText, slugify, useAdminMutation } from './shared'

type Editing = { mode: 'create'; parentId: string | null } | { mode: 'edit'; genre: Genre }

function GenreFormModal({
  editing,
  genres,
  onClose,
}: {
  editing: Editing
  genres: Genre[]
  onClose: () => void
}) {
  const existing = editing.mode === 'edit' ? editing.genre : null
  const [name, setName] = useState(existing?.name ?? '')
  const [slug, setSlug] = useState(existing?.slug ?? '')
  const [slugTouched, setSlugTouched] = useState(existing !== null)
  const [parentId, setParentId] = useState(
    existing ? (existing.parent_id ?? '') : (editing.mode === 'create' ? (editing.parentId ?? '') : ''),
  )
  const [description, setDescription] = useState(existing?.description ?? '')
  const [promptTemplate, setPromptTemplate] = useState(existing?.prompt_template ?? '')
  const [instruments, setInstruments] = useState(existing?.instruments ?? [])
  const [moods, setMoods] = useState(existing?.moods ?? [])
  const [bpmMin, setBpmMin] = useState(numberToText(existing?.bpm_min))
  const [bpmMax, setBpmMax] = useState(numberToText(existing?.bpm_max))
  const [defaultDuration, setDefaultDuration] = useState(numberToText(existing?.default_duration))
  const [sortOrder, setSortOrder] = useState(numberToText(existing?.sort_order ?? 0))

  // Only two levels: a genre that already has subgenres cannot itself become a subgenre.
  const hasChildren = existing !== null && genres.some((g) => g.parent_id === existing.id)
  const parentOptions = topLevelGenres(genres).filter((g) => g.id !== existing?.id)

  const save = useAdminMutation(
    (input: GenreInput) => (existing ? api.genres.update(existing.id, input) : api.genres.create(input)),
    {
      invalidate: ['genres'],
      successMessage: existing ? 'Đã lưu genre.' : 'Đã thêm genre.',
      onSuccess: onClose,
    },
  )

  const onSubmit = (e: FormEvent) => {
    e.preventDefault()
    save.mutate({
      name: name.trim(),
      slug: slug.trim(),
      parent_id: parentId || null,
      description: description.trim(),
      prompt_template: promptTemplate.trim(),
      instruments,
      moods,
      bpm_min: numberOrNull(bpmMin),
      bpm_max: numberOrNull(bpmMax),
      default_duration: numberOrNull(defaultDuration),
      sort_order: numberOrNull(sortOrder) ?? 0,
    })
  }

  return (
    <Modal
      title={existing ? `Sửa genre: ${existing.name}` : parentId ? 'Thêm subgenre' : 'Thêm genre'}
      size="lg"
      onClose={onClose}
      footer={
        <>
          <button type="button" className="btn" onClick={onClose}>
            Hủy bỏ
          </button>
          <button type="submit" form="genre-form" className="btn btn-primary" disabled={save.isPending}>
            Lưu
          </button>
        </>
      }
    >
      <form id="genre-form" className="form-stack" onSubmit={onSubmit}>
        <div className="field-row">
          <Field label="Name">
            <input
              type="text"
              required
              value={name}
              onChange={(e) => {
                setName(e.target.value)
                if (!slugTouched) setSlug(slugify(e.target.value))
              }}
            />
          </Field>
          <Field label="Slug">
            <input
              type="text"
              required
              pattern="[a-z0-9]+(-[a-z0-9]+)*"
              title="Chữ thường, số và dấu gạch ngang"
              value={slug}
              onChange={(e) => {
                setSlug(e.target.value)
                setSlugTouched(true)
              }}
            />
          </Field>
        </div>
        <Field
          label="Genre cha"
          hint={hasChildren ? 'Genre này đang có subgenre nên không thể chuyển thành subgenre.' : undefined}
        >
          <select value={parentId} disabled={hasChildren} onChange={(e) => setParentId(e.target.value)}>
            <option value="">— Không có (genre cấp 1) —</option>
            {parentOptions.map((g) => (
              <option key={g.id} value={g.id}>
                {g.name}
              </option>
            ))}
          </select>
        </Field>
        <Field label="Description">
          <textarea rows={2} value={description} onChange={(e) => setDescription(e.target.value)} />
        </Field>
        <Field
          label="Prompt Template"
          hint="Dùng {prompt} để chèn prompt của người dùng; nếu không có, template được nối sau prompt."
        >
          <textarea rows={3} value={promptTemplate} onChange={(e) => setPromptTemplate(e.target.value)} />
        </Field>
        <ChipSelect label="Instruments" value={instruments} onChange={setInstruments} placeholder="Thêm nhạc cụ…" />
        <ChipSelect label="Moods" value={moods} onChange={setMoods} placeholder="Thêm mood…" />
        <div className="field-row cols-4">
          <Field label="BPM min">
            <input type="number" min={1} value={bpmMin} onChange={(e) => setBpmMin(e.target.value)} />
          </Field>
          <Field label="BPM max">
            <input type="number" min={1} value={bpmMax} onChange={(e) => setBpmMax(e.target.value)} />
          </Field>
          <Field label="Default Duration (giây)">
            <input
              type="number"
              min={10}
              max={240}
              value={defaultDuration}
              onChange={(e) => setDefaultDuration(e.target.value)}
            />
          </Field>
          <Field label="Sort Order">
            <input type="number" value={sortOrder} onChange={(e) => setSortOrder(e.target.value)} />
          </Field>
        </div>
      </form>
    </Modal>
  )
}

function GenreRow({
  genre,
  isChild,
  onEdit,
  onAddChild,
}: {
  genre: Genre
  isChild?: boolean
  onEdit: (genre: Genre) => void
  onAddChild?: (genre: Genre) => void
}) {
  const toggle = useAdminMutation((is_active: boolean) => api.genres.update(genre.id, { is_active }), {
    invalidate: ['genres'],
  })
  return (
    <div className={cx('tree-row', isChild && 'is-child', !genre.is_active && 'is-inactive')}>
      <div className="tree-main">
        {isChild && <CornerDownRight size={15} aria-hidden className="tree-branch" />}
        <div>
          <p className="tree-name">
            {genre.name} <span className="mono muted">{genre.slug}</span>
          </p>
          <p className="tree-sub">
            {genre.bpm_min && genre.bpm_max ? `${genre.bpm_min}–${genre.bpm_max} BPM · ` : ''}
            {genre.default_duration ? `${genre.default_duration}s · ` : ''}
            {genre.instruments.length} instruments · {genre.moods.length} moods
          </p>
        </div>
      </div>
      <div className="tree-actions">
        {onAddChild && (
          <button type="button" className="btn btn-sm" onClick={() => onAddChild(genre)}>
            <Plus size={14} aria-hidden />
            Subgenre
          </button>
        )}
        <button
          type="button"
          className="icon-btn"
          aria-label={`Sửa ${genre.name}`}
          title="Sửa"
          onClick={() => onEdit(genre)}
        >
          <Pencil size={16} aria-hidden />
        </button>
        <Switch
          label={`Bật ${genre.name}`}
          hideLabel
          checked={genre.is_active}
          disabled={toggle.isPending}
          onChange={(v) => toggle.mutate(v)}
        />
      </div>
    </div>
  )
}

export function GenresPage() {
  const genresQuery = useGenres(true)
  const [editing, setEditing] = useState<Editing | null>(null)
  const genres = genresQuery.data ?? []

  return (
    <div className="page">
      <PageHeader
        title="Genres"
        description="Cây genre / subgenre (2 cấp). Template, nhạc cụ và mood ở đây là mặc định cho trang Tạo nhạc."
        actions={
          <button type="button" className="btn btn-primary" onClick={() => setEditing({ mode: 'create', parentId: null })}>
            <Plus size={16} aria-hidden />
            Thêm genre
          </button>
        }
      />

      {genresQuery.isPending ? (
        <LoadingState />
      ) : genresQuery.isError ? (
        <ErrorState error={genresQuery.error} onRetry={() => void genresQuery.refetch()} />
      ) : genres.length === 0 ? (
        <EmptyState icon={<Tags size={26} aria-hidden />} title="Chưa có genre nào">
          Thêm genre đầu tiên để nhân viên có thể chọn khi tạo nhạc.
        </EmptyState>
      ) : (
        <ul className="tree">
          {topLevelGenres(genres).map((genre) => (
            <li key={genre.id} className="tree-group">
              <GenreRow
                genre={genre}
                onEdit={(g) => setEditing({ mode: 'edit', genre: g })}
                onAddChild={(g) => setEditing({ mode: 'create', parentId: g.id })}
              />
              {subgenresOf(genres, genre.id).map((sub) => (
                <GenreRow key={sub.id} genre={sub} isChild onEdit={(g) => setEditing({ mode: 'edit', genre: g })} />
              ))}
            </li>
          ))}
        </ul>
      )}

      {editing && <GenreFormModal editing={editing} genres={genres} onClose={() => setEditing(null)} />}
    </div>
  )
}
