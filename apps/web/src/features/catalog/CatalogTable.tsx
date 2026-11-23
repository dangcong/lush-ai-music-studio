import type { CatalogTrack, CatalogKeyEstimate, CatalogTempoEstimate, Model } from '../../api/types'
import { artifactUrl } from '../../api/client'

const TRAIN_LIMIT_S = 240

function formatDuration(seconds: number): string {
  const s = Math.round(seconds)
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, '0')}`
}

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
        {track.duration_s > TRAIN_LIMIT_S && ' · train 4 phút đầu'}
      </small>
    </td>
  )
}

/** Task 10 swaps this for the interactive cell. */
export function TechStackCell({ track, captioners }: { track: CatalogTrack; captioners: Model[] }) {
  const name = track.current_model_key
    ? (captioners.find((m) => m.key === track.current_model_key)?.name ?? track.current_model_key)
    : null
  return <td>{name ?? <span className="catalog-muted">Chưa chọn</span>}</td>
}

function ValueBox({ value }: { value: string | null }) {
  return value ? (
    <div className="catalog-value">{value}</div>
  ) : (
    <div className="catalog-value catalog-value-empty">Chưa điền</div>
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
  return estimate ? `Máy: ${estimate.key} (${confidenceLabel(estimate.confidence)})` : 'Máy: không đủ căn cứ'
}

export function TempoCell({ track }: { track: CatalogTrack }) {
  const hint = track.analysis_status === 'done' ? tempoHint(track.estimates.tempo) : null
  return (
    <td className="catalog-measure-cell">
      <ValueBox value={track.tempo_bpm == null ? null : String(track.tempo_bpm)} />
      {hint && <small className="catalog-machine">{hint}</small>}
    </td>
  )
}

export function KeyCell({ track }: { track: CatalogTrack }) {
  return (
    <td className="catalog-measure-cell">
      <ValueBox value={track.key_name} />
      {track.analysis_status === 'done' && <small className="catalog-machine">{keyHint(track.estimates.key)}</small>}
    </td>
  )
}

/** Read-only text for the caption fields; Task 10 makes them editable. */
export function TextCell({ value, empty = '—' }: { value: string | undefined; empty?: string }) {
  return <td className="catalog-text-cell">{value ? value : <span className="catalog-muted">{empty}</span>}</td>
}

export function VocalsCell({ track }: { track: CatalogTrack }) {
  return (
    <td className="catalog-text-cell">
      {track.caption?.vocals ? track.caption.vocals : <span className="catalog-muted">—</span>}
      {track.voice_warning && <span className="badge tone-warn catalog-voice">Có thể có giọng</span>}
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
              <TextCell value={t.caption?.mood} />
              <TextCell value={t.caption?.instruments} />
              <VocalsCell track={t} />
              <TextCell value={t.caption?.caption} empty="Chưa có mô tả." />
              <StatusCell track={t} onRetry={onRetry} />
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  )
}
