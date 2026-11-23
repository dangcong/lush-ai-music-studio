import { useState } from 'react'
import type { FormEvent } from 'react'
import { Database, Plus } from 'lucide-react'
import { Link, useNavigate } from 'react-router-dom'
import { ApiError, errorMessage } from '../api/client'
import { useCatalogs, useGenres } from '../api/hooks'
import { useCreateCatalog } from '../api/mutations'
import { useCurrentUser } from '../auth/AuthContext'
import { Modal } from '../components/Modal'
import { EmptyState, ErrorState, Field, LoadingState, Notice, PageHeader } from '../components/ui'

const TITLE = 'Catalog'
const FORM_ID = 'catalog-create-form'

function CreateCatalogForm({ onClose, onCreated }: { onClose: () => void; onCreated: (id: string) => void }) {
  const genresQuery = useGenres()
  const create = useCreateCatalog()
  const [name, setName] = useState('')
  const [genreId, setGenreId] = useState('')
  const [formError, setFormError] = useState<string | null>(null)

  const genres = genresQuery.data ?? []
  const selectedGenre = genres.some((g) => g.id === genreId) ? genreId : (genres[0]?.id ?? '')

  const onSubmit = (e: FormEvent) => {
    e.preventDefault()
    if (create.isPending) return
    const trimmed = name.trim()
    if (!trimmed) return setFormError('Nhập tên catalog.')
    if (!selectedGenre) return setFormError('Chọn thể loại.')
    setFormError(null)
    create.mutate(
      { name: trimmed, genre_id: selectedGenre },
      {
        onSuccess: (catalog) => onCreated(catalog.id),
        onError: (error) =>
          setFormError(error instanceof ApiError && error.status === 409 ? 'Tên catalog đã có' : errorMessage(error)),
      },
    )
  }

  return (
    <Modal
      title="Tạo catalog"
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
            disabled={create.isPending || genresQuery.isPending}
          >
            Tạo
          </button>
        </>
      }
    >
      {genresQuery.isError ? (
        <ErrorState error={genresQuery.error} onRetry={() => void genresQuery.refetch()} />
      ) : (
        <form id={FORM_ID} className="form-stack" onSubmit={onSubmit} noValidate aria-label="Tạo catalog">
          <Field label="Tên catalog">
            <input type="text" required maxLength={120} value={name} onChange={(e) => setName(e.target.value)} />
          </Field>
          <Field label="Thể loại">
            <select value={selectedGenre} onChange={(e) => setGenreId(e.target.value)}>
              {genres.map((g) => (
                <option key={g.id} value={g.id}>
                  {g.name}
                </option>
              ))}
            </select>
          </Field>
          {formError && <Notice tone="danger">{formError}</Notice>}
        </form>
      )}
    </Modal>
  )
}

export function CatalogsPage() {
  const user = useCurrentUser()
  const allowed = user.role === 'admin' || user.role === 'trainer'
  const navigate = useNavigate()
  const catalogsQuery = useCatalogs(allowed)
  const [creating, setCreating] = useState(false)

  if (!allowed) {
    return (
      <div className="page">
        <PageHeader title={TITLE} />
        <Notice tone="info">
          <span>Chỉ admin và trainer dùng được Catalog</span>
        </Notice>
      </div>
    )
  }

  return (
    <div className="page">
      <PageHeader
        title={TITLE}
        description="Bộ bài hát kèm mô tả, dùng làm dữ liệu để train Style."
        actions={
          <button type="button" className="btn btn-primary" onClick={() => setCreating(true)}>
            <Plus size={16} aria-hidden />
            Tạo catalog
          </button>
        }
      />

      {catalogsQuery.isPending ? (
        <LoadingState />
      ) : catalogsQuery.isError ? (
        <ErrorState error={catalogsQuery.error} onRetry={() => void catalogsQuery.refetch()} />
      ) : catalogsQuery.data.length === 0 ? (
        <EmptyState icon={<Database size={26} aria-hidden />} title="Chưa có catalog nào">
          Bấm “Tạo catalog” để bắt đầu.
        </EmptyState>
      ) : (
        <div className="table-wrap">
          <table className="table">
            <thead>
              <tr>
                <th scope="col">Tên</th>
                <th scope="col">Genre</th>
                <th scope="col">Số bài</th>
                <th scope="col">Tổng phút</th>
              </tr>
            </thead>
            <tbody>
              {catalogsQuery.data.map((c) => (
                <tr key={c.id}>
                  <td className="cell-title">
                    <Link to={`/catalogs/${c.id}`}>{c.name}</Link>
                  </td>
                  <td>{c.genre_name}</td>
                  <td>{c.track_count}</td>
                  <td>{Math.round(c.total_seconds / 60)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}

      {creating && (
        <CreateCatalogForm
          onClose={() => setCreating(false)}
          onCreated={(id) => {
            setCreating(false)
            navigate(`/catalogs/${id}`)
          }}
        />
      )}
    </div>
  )
}
