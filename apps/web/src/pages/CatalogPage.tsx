import { useState } from 'react'
import { Link, useParams } from 'react-router-dom'
import { Music } from 'lucide-react'
import { useCaptioners, useCatalog, useCatalogTracks } from '../api/hooks'
import {
  useCreateCaptionJob,
  useDeleteCatalogTracks,
  useReanalyzeCatalogTrack,
  useRefreshCatalog,
} from '../api/mutations'
import { CatalogTable } from '../features/catalog/CatalogTable'
import { SelectionBar, isInstalled } from '../features/catalog/SelectionBar'
import { UploadButton } from '../features/catalog/UploadButton'
import { EmptyState, ErrorState, LoadingState, Notice } from '../components/ui'

export function CatalogPage() {
  const { id = '' } = useParams()
  const catalogQuery = useCatalog(id)
  const tracksQuery = useCatalogTracks(id)
  const captionersQuery = useCaptioners()
  const createJob = useCreateCaptionJob(id)
  const deleteTracks = useDeleteCatalogTracks(id)
  const reanalyze = useReanalyzeCatalogTrack(id)
  const refresh = useRefreshCatalog(id)

  const [selection, setSelection] = useState<ReadonlySet<string>>(new Set())
  const [pickedModel, setPickedModel] = useState('')
  const [uploadErrors, setUploadErrors] = useState<string[]>([])
  const [jobNotice, setJobNotice] = useState<string | null>(null)

  const tracks = tracksQuery.data ?? []
  const captioners = captionersQuery.data ?? []
  // Selection is keyed by track id; ids that vanished after a refetch are ignored.
  const selectedIds = tracks.filter((t) => selection.has(t.id)).map((t) => t.id)
  const selectedSet = new Set(selectedIds)
  const modelKey = captioners.some((m) => m.key === pickedModel && isInstalled(m))
    ? pickedModel
    : (captioners.find(isInstalled)?.key ?? '')

  if (catalogQuery.isError) {
    return (
      <div className="page">
        <ErrorState error={catalogQuery.error} onRetry={() => void catalogQuery.refetch()} />
      </div>
    )
  }

  const toggle = (trackId: string) =>
    setSelection((prev) => {
      const next = new Set(prev)
      if (!next.delete(trackId)) next.add(trackId)
      return next
    })

  const onCreate = () => {
    setJobNotice(null)
    createJob.mutate(
      { model_key: modelKey, track_ids: selectedIds },
      {
        onSuccess: (result) => {
          setSelection(new Set())
          setJobNotice(
            `Đã xếp hàng ${result.queued.length} bài, bỏ qua ${result.skipped.length} bài đã có mô tả của model này.` +
              (result.queued.length > 0 ? ' Trong lúc tạo mô tả, nhạc mới tạo sẽ phải chờ.' : ''),
          )
        },
      },
    )
  }

  const onDelete = () => {
    if (!window.confirm(`Xóa ${selectedIds.length} bài khỏi catalog?`)) return
    deleteTracks.mutate(selectedIds, { onSettled: () => setSelection(new Set()) })
  }

  return (
    <div className="page catalog-page">
      <header className="catalog-head">
        <div>
          <h1>{catalogQuery.data?.name ?? 'Catalog'}</h1>
          {catalogQuery.data && <span className="badge catalog-genre">Genre: {catalogQuery.data.genre_name}</span>}
        </div>
        <UploadButton
          catalogId={id}
          onFinished={(errors) => {
            setUploadErrors(errors)
            refresh()
          }}
        />
      </header>

      {uploadErrors.length > 0 && (
        <Notice tone="warn">
          <ul className="catalog-errors">
            {uploadErrors.map((message) => (
              <li key={message}>{message}</li>
            ))}
          </ul>
        </Notice>
      )}

      <SelectionBar
        selectedCount={selectedIds.length}
        captioners={captioners}
        modelKey={modelKey}
        onModelChange={setPickedModel}
        onSelectAll={() => setSelection(new Set(tracks.map((t) => t.id)))}
        onSelectFirst={(n) => setSelection(new Set(tracks.slice(0, n).map((t) => t.id)))}
        onCreate={onCreate}
        onDelete={onDelete}
        busy={createJob.isPending || deleteTracks.isPending}
      />
      {jobNotice && <Notice tone="info">{jobNotice}</Notice>}
      {tracks.some((t) => t.captioning_model_key) && (
        <Notice tone="info">
          Đang tạo mô tả — xem/hủy ở <Link to="/queue">Hàng đợi</Link>.
        </Notice>
      )}

      {tracksQuery.isPending ? (
        <LoadingState />
      ) : tracksQuery.isError ? (
        <ErrorState error={tracksQuery.error} onRetry={() => void tracksQuery.refetch()} />
      ) : tracks.length === 0 ? (
        <EmptyState icon={<Music size={26} aria-hidden />} title="Chưa có bài nào">
          Bấm “Tải nhạc lên” để thêm bài vào catalog.
        </EmptyState>
      ) : (
        <CatalogTable
          tracks={tracks}
          selected={selectedSet}
          captioners={captioners}
          onToggle={toggle}
          onRetry={(trackId) => reanalyze.mutate(trackId)}
        />
      )}
    </div>
  )
}
