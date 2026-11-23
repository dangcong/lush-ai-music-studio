import { errorMessage } from '../../api/client'
import { useCatalogTrackEdits } from '../../api/mutations'
import type { CatalogTrack, Model } from '../../api/types'

const NOT_INSTALLED = 'Chưa cài trên máy'
const RERUN_CONFIRM = 'Chạy lại sẽ thay bản hiện tại, kể cả phần bạn đã sửa. Bấm Quay về để lấy lại.'

const isInstalled = (m: Model) => m.installed_nodes.length > 0

export function TechStackCell({ track, captioners }: { track: CatalogTrack; captioners: Model[] }) {
  const edits = useCatalogTrackEdits(track.catalog_id)
  const key = track.current_model_key
  const current = captioners.find((m) => m.key === key)
  const done = key != null && track.models_done.includes(key)
  const busy = track.captioning_model_key != null
  const canRun = key != null && current != null && isInstalled(current) && !busy
  const failed = [edits.update, edits.rerun, edits.undo, edits.redo].find((m) => m.isError)
  const caption = track.caption

  const run = () => {
    if (!key) return
    if (done && !window.confirm(RERUN_CONFIRM)) return
    edits.rerun.mutate({ model_key: key, track_ids: [track.id], force: true })
  }

  return (
    <td className="catalog-tech-cell">
      <select
        className="select catalog-model-select"
        aria-label={`Model của ${track.filename}`}
        value={key ?? ''}
        onChange={(e) => edits.update.mutate({ id: track.id, input: { current_model_key: e.target.value } })}
      >
        {key == null && (
          <option value="" disabled>
            Chưa chọn
          </option>
        )}
        {key != null && current == null && <option value={key}>{key}</option>}
        {captioners.map((m) => (
          <option key={m.key} value={m.key} disabled={!isInstalled(m)}>
            {track.models_done.includes(m.key) ? '● ' : ''}
            {m.name}
            {isInstalled(m) ? '' : ` — ${NOT_INSTALLED}`}
          </option>
        ))}
      </select>
      <div className="catalog-tech-actions">
        <button type="button" className="btn btn-sm" disabled={!canRun || edits.rerun.isPending} onClick={run}>
          {done ? 'Chạy lại' : 'Tạo'}
        </button>
        {caption?.can_undo && key && (
          <button
            type="button"
            className="btn btn-sm"
            disabled={edits.undo.isPending}
            onClick={() => edits.undo.mutate({ id: track.id, modelKey: key })}
          >
            Quay về
          </button>
        )}
        {caption?.can_redo && key && (
          <button
            type="button"
            className="btn btn-sm"
            disabled={edits.redo.isPending}
            onClick={() => edits.redo.mutate({ id: track.id, modelKey: key })}
          >
            Tới
          </button>
        )}
      </div>
      {failed && (
        <small className="catalog-error" role="alert">
          {errorMessage(failed.error)}
        </small>
      )}
    </td>
  )
}
