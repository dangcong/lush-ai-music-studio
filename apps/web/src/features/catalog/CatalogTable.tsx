import type { CatalogTrack, CatalogKeyEstimate, CatalogTempoEstimate, Model } from '../../api/types'
import { artifactUrl, errorMessage } from '../../api/client'
import { useCatalogTrackEdits } from '../../api/mutations'
import { formatDuration } from '../../lib/format'
import { EditableCell } from './EditableCell'
import { TechStackCell } from './TechStackCell'

const TRAIN_LIMIT_S = 240

function confidenceLabel(confidence: number): string {
  if (confidence >= 0.6) return 'tin cậy cao'
  if (confidence >= 0.3) return 'tin cậy khá'
  return 'tin cậy thấp'
}

export function TrackCell({ track }: { track: CatalogTrack }) {
  return (
    <td className="catalog-track-cell">
      <div className="catalog-track-name">{track.filename}</div>
      <audio controls preload="none" src={artifactUrl(track.artifact_id)} aria-label={`Nghe ${track.filename}`} />
      <small>
        {formatDuration(track.duration_s)}
        {track.duration_s != null && track.duration_s > TRAIN_LIMIT_S && ' · train 4 phút đầu'}
      </small>
    </td>
  )
}

function tempoHint(estimate: CatalogTempoEstimate | null): string | null {
  if (!estimate) return null
  if (estimate.bpm == null) {
    return estimate.alternatives.length > 0 ? `Máy: ${estimate.alternatives.join(' hoặc ')}` : null
  }
  return `Máy: ${estimate.bpm} (${confidenceLabel(estimate.confidence)})`
}

function keyHint(estimate: CatalogKeyEstimate | null): string {
  return estimate?.key ? `Máy: ${estimate.key} (${confidenceLabel(estimate.confidence)})` : 'Máy: không đủ căn cứ'
}

export function TempoCell({ track }: { track: CatalogTrack }) {
  const edits = useCatalogTrackEdits(track.catalog_id)
  const hint = track.analysis_status === 'done' ? tempoHint(track.estimates.tempo) : null
  return (
    <td className="catalog-measure-cell">
      <EditableCell
        label={`Tempo của ${track.filename}`}
        value={track.tempo_bpm == null ? null : String(track.tempo_bpm)}
        error={edits.update.isError ? errorMessage(edits.update.error) : null}
        onSave={(text) => {
          const t = text.trim()
          const bpm = t === '' ? null : Number(t)
          if (bpm !== null && !Number.isFinite(bpm)) return 'Chỉ nhập số'
          return edits.update.mutateAsync({ id: track.id, input: { tempo_bpm: bpm } })
        }}
      />
      {hint && <small className="catalog-machine">{hint}</small>}
    </td>
  )
}

export function KeyCell({ track }: { track: CatalogTrack }) {
  const edits = useCatalogTrackEdits(track.catalog_id)
  return (
    <td className="catalog-measure-cell">
      <EditableCell
        label={`Key của ${track.filename}`}
        value={track.key_name}
        error={edits.update.isError ? errorMessage(edits.update.error) : null}
        onSave={(text) => {
          return edits.update.mutateAsync({ id: track.id, input: { key_name: text.trim() === '' ? null : text.trim() } })
        }}
      />
      {track.analysis_status === 'done' && <small className="catalog-machine">{keyHint(track.estimates.key)}</small>}
    </td>
  )
}

type CaptionField = 'mood' | 'instruments' | 'vocals' | 'caption'

const FIELD_LABEL: Record<CaptionField, string> = {
  mood: 'Mood',
  instruments: 'Instruments',
  vocals: 'Vocals',
  caption: 'Music caption',
}

/** Editable caption field; saving sends all four fields with one changed. */
export function CaptionFieldCell({
  track,
  field,
  empty = '—',
  multiline = false,
  children,
}: {
  track: CatalogTrack
  field: CaptionField
  empty?: string
  multiline?: boolean
  children?: React.ReactNode
}) {
  const edits = useCatalogTrackEdits(track.catalog_id)
  const key = track.current_model_key
  const base = {
    mood: track.caption?.mood ?? '',
    instruments: track.caption?.instruments ?? '',
    vocals: track.caption?.vocals ?? '',
    caption: track.caption?.caption ?? '',
  }
  return (
    <td className="catalog-text-cell">
      <EditableCell
        label={`${FIELD_LABEL[field]} của ${track.filename}`}
        value={base[field] || null}
        empty={empty}
        multiline={multiline}
        disabled={!key}
        disabledTitle="Chọn model trước"
        error={edits.saveCaption.isError ? errorMessage(edits.saveCaption.error) : null}
        onSave={(text) => {
          if (!key) return
          return edits.saveCaption.mutateAsync({ id: track.id, modelKey: key, body: { ...base, [field]: text } })
        }}
      />
      {children}
    </td>
  )
}

export function StatusCell({ track, onRetry }: { track: CatalogTrack; onRetry: (id: string) => void }) {
  if (track.analysis_status === 'failed') {
    return (
      <td>
        <span className="badge badge-failed">Lỗi đo</span>
        <button type="button" className="btn btn-sm catalog-retry" onClick={() => onRetry(track.id)}>
          Thử lại
        </button>
      </td>
    )
  }
  let badge: { text: string; cls: string }
  if (track.analysis_status === 'pending') badge = { text: 'Đang đo', cls: 'badge-running' }
  else if (track.captioning_model_key) badge = { text: 'Đang tạo mô tả…', cls: 'badge-running' }
  else if (track.caption) badge = { text: 'Có mô tả', cls: 'badge-completed' }
  else badge = { text: 'Mới tải lên', cls: '' }
  return (
    <td>
      <span className={`badge ${badge.cls}`.trim()}>{badge.text}</span>
    </td>
  )
}

interface CatalogTableProps {
  tracks: CatalogTrack[]
  selected: ReadonlySet<string>
  captioners: Model[]
  onToggle: (id: string) => void
  onRetry: (id: string) => void
}

export function CatalogTable({ tracks, selected, captioners, onToggle, onRetry }: CatalogTableProps) {
  return (
    <div className="table-wrap">
      <table className="table catalog-table">
        <thead>
          <tr>
            <th scope="col" aria-label="Chọn" />
            <th scope="col">Bài</th>
            <th scope="col">Tech stack</th>
            <th scope="col">Tempo</th>
            <th scope="col">Key</th>
            <th scope="col">Mood</th>
            <th scope="col">Instruments</th>
            <th scope="col">Vocals</th>
            <th scope="col">Music caption</th>
            <th scope="col">Trạng thái</th>
          </tr>
        </thead>
        <tbody>
          {tracks.map((t) => (
            <tr key={t.id} className={selected.has(t.id) ? 'is-selected' : undefined}>
              <td>
                <input
                  type="checkbox"
                  checked={selected.has(t.id)}
                  aria-label={`Chọn ${t.filename}`}
                  onChange={() => onToggle(t.id)}
                />
              </td>
              <TrackCell track={t} />
              <TechStackCell track={t} captioners={captioners} />
              <TempoCell track={t} />
              <KeyCell track={t} />
              <CaptionFieldCell track={t} field="mood" />
              <CaptionFieldCell track={t} field="instruments" />
              <CaptionFieldCell track={t} field="vocals">
                {t.voice_warning && <span className="badge tone-warn catalog-voice">Có thể có giọng</span>}
              </CaptionFieldCell>
              <CaptionFieldCell track={t} field="caption" empty="Chưa có mô tả." multiline />
              <StatusCell track={t} onRetry={onRetry} />
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  )
}
